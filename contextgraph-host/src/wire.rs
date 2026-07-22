//! The CGP wire envelope and its framing (`SPEC.md` §Transport bindings).
//!
//! # This is not JSON-RPC
//!
//! Earlier revisions of this module claimed CGP "rides MCP's transport and
//! lifecycle conventions (JSON-RPC 2.0, ...)". That was not true of the wire it
//! described, and the claim has been withdrawn — see
//! [ADR 0002](../../docs/adr/0002-request-correlation-and-the-json-rpc-question.md).
//! CGP's envelope is a bespoke `type`-tagged JSON object: there is no
//! `jsonrpc` member, no `method`/`params` split. Its *lifecycle* is informed by
//! MCP (a handshake that negotiates version and capabilities before any
//! payload moves), but the framing is its own.
//!
//! A JSON-RPC **binding** — an alternate encoding of this same semantic layer —
//! may be specified later without touching frame or query semantics and without
//! a new protocol family. Keeping the semantic layer and the transport binding
//! separate is what makes that possible.
//!
//! # Framing
//!
//! Every message is **newline-delimited JSON (NDJSON): exactly one
//! `serde_json` value per line** — the simplest thing that is unambiguous over
//! a pipe and trivially reimplementable in a provider kit in any language. HTTP
//! providers receive the same envelope as a JSON request body and reply with
//! one as the response body.
//!
//! The envelope is **versioned**: the handshake negotiates the protocol family
//! up front, and a mismatch is a named error, never a hang (`SPEC.md` §H3).
//!
//! # Correlation
//!
//! `query`, `frames`, and `error` carry an optional [`id`](Envelope). A
//! provider **MUST** echo the `id` of the request it is answering. An envelope
//! with no `id` is a *notification*: it expects no reply, which is the shape a
//! future push-invalidation extension needs
//! (`docs/sketches/push-invalidation.md`).
//!
//! `id` is optional so that a provider written against an earlier revision
//! stays conformant: a host that has not seen a provider echo an `id` **MUST**
//! fall back to lock-step. Concurrency is negotiated by observation, not by a
//! capability flag.

use contextgraph_types::{
    Capabilities, ContextQuery, ContextQueryResult, ErrorCode, ProviderInfo, VerifyRequest,
    VerifyResponse,
};
use serde::{Deserialize, Serialize};

use crate::error::HostError;

/// One Context Graph Protocol message. Every variant is a small, versioned, `type`-tagged JSON
/// object; the host writes exactly one per line (NDJSON) over stdio and one
/// per HTTP body (`SPEC.md` §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Envelope {
    /// Host hello: opens the exchange with the protocol version the host
    /// speaks (SPEC.md §3 `initialize`).
    Handshake { protocol_version: String },
    /// Provider hello-back: its protocol version, identity + declared
    /// data-flow direction, and negotiated capabilities (SPEC.md §3). The host
    /// checks the version and surfaces `provider.data_flow` at consent time.
    HandshakeAck {
        protocol_version: String,
        provider: ProviderInfo,
        capabilities: Capabilities,
    },
    /// Host → provider retrieval request (`context/query`).
    Query {
        /// Correlation id. A provider **MUST** echo it on the reply.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        query: ContextQuery,
    },
    /// Provider → host budgeted, provenance-carrying frames.
    Frames {
        /// The `id` of the `query` this answers, echoed verbatim.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        result: ContextQueryResult,
    },
    /// Host → provider revalidation request: are these held frames still
    /// valid (`docs/context-reuse.md` §4 `context/verify`)? Carries frame
    /// identities only — never bodies. Capability-gated: a host sends it only
    /// to a provider advertising [`Capabilities::verify`](contextgraph_types::Capabilities::verify).
    Verify { request: VerifyRequest },
    /// Provider → host per-frame verdicts.
    Verified { response: VerifyResponse },
    /// Lifecycle teardown; the provider should exit cleanly.
    Shutdown,
    /// Provider-reported failure — lets a provider report a bad request
    /// without dying (`SPEC.md` §R1 "fail loud"). The host maps this to
    /// [`HostError::Provider`].
    Error {
        /// The `id` of the request that failed, when the failure is
        /// attributable to one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Machine-readable classification (`SPEC.md` §Errors). Optional so
        /// that a provider written against an earlier revision stays
        /// conformant; a host treats its absence as
        /// [`ErrorCode::Internal`](contextgraph_types::ErrorCode::Internal).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<ErrorCode>,
        /// Human-readable detail. Always present: the code is for the machine,
        /// the message is for whoever reads the log.
        message: String,
    },
}

impl Envelope {
    /// The correlation id this envelope carries, if any.
    ///
    /// `None` means one of two things, and the caller can tell them apart from
    /// context: either the envelope is a lifecycle message that never carries
    /// one (`handshake`, `shutdown`), or the peer does not implement
    /// correlation and the exchange must stay lock-step.
    pub fn correlation_id(&self) -> Option<&str> {
        match self {
            Envelope::Query { id, .. }
            | Envelope::Frames { id, .. }
            | Envelope::Error { id, .. } => id.as_deref(),
            _ => None,
        }
    }

    /// The error code carried by an `error` envelope, defaulting to
    /// [`ErrorCode::Internal`] when the provider declared none.
    ///
    /// Defaulting to `Internal` rather than to something retryable is the
    /// conservative reading: a host must not infer "safe to retry" from a
    /// provider's silence.
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            Envelope::Error { code, .. } => Some(code.clone().unwrap_or(ErrorCode::Internal)),
            _ => None,
        }
    }
}

/// Mint a fresh correlation id.
///
/// Ids need only be unique among the exchanges in flight on one connection; a
/// process-wide counter satisfies that with no per-connection state and no
/// randomness dependency. They are opaque to the provider, which must echo the
/// string verbatim rather than parse it.
pub fn next_correlation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("q{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Check that a reply carries the correlation id of the request it answers
/// (`SPEC.md` §H4).
///
/// `sent` is `None` when the provider did not declare `correlation`, in which
/// case the exchange is lock-step and there is nothing to verify.
pub fn verify_correlation(
    provider_id: &str,
    sent: Option<&str>,
    echoed: Option<&str>,
) -> Result<(), HostError> {
    let Some(expected) = sent else {
        return Ok(());
    };
    match echoed {
        Some(got) if got == expected => Ok(()),
        // A correlation-declaring provider that answers without an id, or with
        // the wrong one, is worse than one that never declared it: a host that
        // accepted the reply anyway could hand one caller's frames to another.
        other => Err(HostError::CorrelationMismatch {
            id: provider_id.to_string(),
            expected: expected.to_string(),
            got: other.unwrap_or("<absent>").to_string(),
        }),
    }
}

/// The human name of an envelope variant, for error messages that report
/// "expected X, got Y".
pub fn envelope_kind(env: &Envelope) -> &'static str {
    match env {
        Envelope::Handshake { .. } => "handshake",
        Envelope::HandshakeAck { .. } => "handshake_ack",
        Envelope::Query { .. } => "query",
        Envelope::Frames { .. } => "frames",
        Envelope::Verify { .. } => "verify",
        Envelope::Verified { .. } => "verified",
        Envelope::Shutdown => "shutdown",
        Envelope::Error { .. } => "error",
    }
}

/// Serialize an envelope to a single NDJSON line (trailing `\n` included).
pub fn encode_line(env: &Envelope) -> Result<String, HostError> {
    let mut line = serde_json::to_string(env).map_err(|e| HostError::Wire(e.to_string()))?;
    line.push('\n');
    Ok(line)
}

/// Parse one NDJSON line into an envelope. A garbage line is a clean
/// [`HostError::Wire`], never a panic — the crash-consistency contract
/// (task deliverable 5).
pub fn decode_line(line: &str) -> Result<Envelope, HostError> {
    serde_json::from_str(line.trim_end()).map_err(|e| HostError::Wire(e.to_string()))
}

/// Two protocol version strings interoperate when they share a **major
/// family** — the substring up to the first `.`. So `contextgraph/1.0-draft` and
/// `contextgraph/1.0` interoperate (both `contextgraph/1`), while `contextgraph/2.0` does not. This is
/// what lets the public v1.0 freeze drop the `-draft` suffix without a flag
/// day (`SPEC.md`).
pub fn versions_compatible(a: &str, b: &str) -> bool {
    protocol_family(a) == protocol_family(b)
}

fn protocol_family(version: &str) -> &str {
    match version.split_once('.') {
        Some((family, _)) => family,
        None => version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::capability::QueryCapability;
    use contextgraph_types::{DataFlow, FrameKind, PROTOCOL_VERSION};

    fn sample_ack() -> Envelope {
        Envelope::HandshakeAck {
            protocol_version: PROTOCOL_VERSION.to_string(),
            provider: ProviderInfo {
                name: "contextgraph-docs".into(),
                version: "0.1.0".into(),
                data_flow: DataFlow {
                    reads: true,
                    writes: false,
                    egress: false,
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: QueryCapability {
                    kinds: vec!["doc".into()],
                },
                ..Capabilities::default()
            },
        }
    }

    #[test]
    fn envelope_kind_matches_the_serialized_type_tag_for_every_variant() {
        // `envelope_kind` hand-writes the strings serde derives from
        // `rename_all = "snake_case"`; they feed "expected X, got Y" wire
        // errors. Without this test, renaming a variant would silently
        // desynchronize the error text from the actual wire tag.
        let variants: Vec<Envelope> = vec![
            Envelope::Handshake {
                protocol_version: PROTOCOL_VERSION.to_string(),
            },
            sample_ack(),
            Envelope::Query {
                id: None,
                query: ContextQuery {
                    goal: "g".into(),
                    query_text: None,
                    embedding: None,
                    kinds: vec![],
                    anchors: vec![],
                    max_frames: 1,
                    max_tokens: 1,
                    as_of: None,
                    representation_preferences: vec![],
                },
            },
            Envelope::Frames {
                id: None,
                result: ContextQueryResult {
                    frames: vec![],
                    truncated: false,
                    dropped_estimate: None,
                },
            },
            Envelope::Shutdown,
            Envelope::Error {
                id: None,
                code: None,
                message: "m".into(),
            },
        ];
        for env in &variants {
            let value: serde_json::Value =
                serde_json::from_str(encode_line(env).unwrap().trim_end()).unwrap();
            assert_eq!(
                value["type"].as_str(),
                Some(envelope_kind(env)),
                "envelope_kind drifted from the serde tag for {env:?}"
            );
        }
    }

    #[test]
    fn envelope_roundtrips_through_a_single_ndjson_line() {
        let env = sample_ack();
        let line = encode_line(&env).unwrap();
        assert!(line.ends_with('\n'), "a frame is exactly one line");
        assert_eq!(line.matches('\n').count(), 1, "no embedded newlines");
        let back = decode_line(&line).unwrap();
        assert_eq!(back, env);
    }

    #[test]
    fn query_envelope_carries_contextgraph_types_shapes_verbatim() {
        let query = ContextQuery {
            goal: "fix the failing test".into(),
            query_text: None,
            embedding: None,
            kinds: vec![FrameKind::Doc],
            anchors: vec![],
            max_frames: 5,
            max_tokens: 2000,
            as_of: None,
            representation_preferences: vec![],
        };
        let env = Envelope::Query {
            id: None,
            query: query.clone(),
        };
        let line = encode_line(&env).unwrap();
        match decode_line(&line).unwrap() {
            Envelope::Query { query: back, .. } => assert_eq!(back, query),
            other => panic!("expected query, got {}", envelope_kind(&other)),
        }
    }

    #[test]
    fn a_garbage_line_is_a_clean_wire_error_never_a_panic() {
        let err = decode_line("this is not json {{{").unwrap_err();
        assert!(matches!(err, HostError::Wire(_)));
    }

    #[test]
    fn version_families_interoperate_within_a_major_but_not_across() {
        assert!(versions_compatible(
            "contextgraph/1.0-draft",
            "contextgraph/1.0"
        ));
        assert!(versions_compatible(
            "contextgraph/1.0-draft",
            "contextgraph/1.0-draft"
        ));
        assert!(versions_compatible(PROTOCOL_VERSION, "contextgraph/1.9"));
        assert!(!versions_compatible(
            "contextgraph/1.0-draft",
            "contextgraph/2.0"
        ));
        assert!(!versions_compatible("contextgraph/1.0", "mcp/1.0"));
    }
}
