//! Streamable-HTTP transport: a remote Context Graph Protocol provider reached by POSTing the
//! envelope to its URL (`SPEC.md` §3 "remote providers:
//! streamable HTTP"). The reference host uses request/response JSON — the
//! [`Envelope`] as the POST body, one [`Envelope`] back as the response body
//! — which any streamable-HTTP server satisfies; chunked frame streaming is
//! a documented forward extension, not needed for the v1 shape.
//!
//! Unlike stdio's process isolation, an HTTP provider is remote by nature, so
//! its `egress` posture is decided by the URL host and gated through the same
//! [`crate::consent`] store at the [`crate::host::Host`] layer.

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use contextgraph_types::{
    Capabilities, ContextQuery, ContextQueryResult, PROTOCOL_VERSION, ProviderInfo, VerifyRequest,
    VerifyResponse,
};

use crate::error::HostError;
use crate::provider::ContextProvider;
use crate::wire::{
    Envelope, envelope_kind, next_correlation_id, verify_correlation, versions_compatible,
};

/// Total per-request budget for an HTTP exchange (handshake or query).
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// A bearer credential a host uses to authenticate to a remote provider.
///
/// The secret is **never** rendered: both [`Debug`](fmt::Debug) and
/// [`Display`](fmt::Display) print the fixed placeholder `Credential(<redacted>)`,
/// so a credential that reaches a log line, an `{:?}`/`{}` interpolation, or a
/// panic payload cannot spill its bytes (`SPEC.md` §4.2, **C8**). The only way
/// to read the raw value is [`Credential::expose`], a crate-private method used
/// solely to attach the header on the wire — a leak is therefore greppable.
#[derive(Clone)]
pub struct Credential {
    /// The bearer token / `Authorization` value. Deliberately unexposed to any
    /// formatting impl.
    token: String,
}

impl Credential {
    /// Wrap a bearer token. It is attached as `Authorization: Bearer <token>`
    /// on every request this provider sends and is never logged (C8).
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    /// The raw secret — the single, greppable exit point, used only to set the
    /// `Authorization` header on the wire.
    fn expose(&self) -> &str {
        &self.token
    }
}

/// C8: a credential in a `{:?}` rendering (a log line, a panic payload) prints a
/// fixed placeholder, never its bytes.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

/// C8: a credential in a `{}` rendering prints the same fixed placeholder — so
/// even an accidental `Display` interpolation cannot leak the secret.
impl fmt::Display for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

/// Whether a URL host component names the loopback interface — the one case an
/// unencrypted (`http://`) transport is allowed, because the bytes never leave
/// the machine (`SPEC.md` §4.2, **C7**). Mirrors the `localhost` exception
/// [`verify::file_uri_to_path`](crate::verify) makes for `file://`, widened to
/// the loopback IP ranges: the literal name `localhost`, `127.0.0.0/8`, and
/// `::1`. IPv6 hosts arrive bracketed (`[::1]`) from a URL, so the brackets are
/// stripped before parsing.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    // `IpAddr::is_loopback` is exactly `127.0.0.0/8` for v4 and `::1` for v6.
    matches!(bare.parse::<IpAddr>(), Ok(ip) if ip.is_loopback())
}

/// Refuse a plaintext transport to a non-loopback provider **before** any bytes
/// leave the host (`SPEC.md` §4.2, **C7**): an `http://` (not `https://`) URL
/// whose host is not loopback would carry the query payload — and any bearer
/// credential — across the network in cleartext. A loopback `http://` target is
/// allowed (the bytes never leave the machine); every `https://` target is
/// allowed. Called before the client is built or DNS is resolved, so a refusal
/// short-circuits with zero network activity.
///
/// # Why this is public
///
/// [`Host::add_http`](crate::Host::add_http) already calls it, so C7 holds
/// whether or not a caller does. It is exported for the case a host wants to
/// classify a URL *before* attempting the connection — typically to report a
/// plaintext endpoint as the configuration error it is, rather than as a
/// connection failure or a non-conformant provider.
///
/// The alternative is that every host re-derives "which hosts are loopback"
/// locally, and C7 ends up with one implementation per host, free to disagree
/// about `[::1]`, `127.0.0.2`, or the casing of `LOCALHOST`. A normative rule
/// with N implementations is N rules. This is the one.
///
/// ```no_run
/// use contextgraph_host::{HostError, refuse_insecure_transport};
///
/// // Plaintext to a remote peer: refused, with the peer named.
/// let refusal = refuse_insecure_transport("acme", "http://cgp.example.com/q");
/// assert!(matches!(refusal, Err(HostError::InsecureTransport { .. })));
///
/// // Loopback plaintext and TLS are both fine.
/// assert!(refuse_insecure_transport("local", "http://127.0.0.1:8080/q").is_ok());
/// assert!(refuse_insecure_transport("acme", "https://cgp.example.com/q").is_ok());
/// ```
pub fn refuse_insecure_transport(id: &str, url: &str) -> Result<(), HostError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| HostError::Transport {
        id: id.to_string(),
        message: format!("invalid provider url: {e}"),
    })?;
    if parsed.scheme() == "http" {
        let host = parsed.host_str().unwrap_or("");
        if !is_loopback_host(host) {
            return Err(HostError::InsecureTransport {
                id: id.to_string(),
                host: host.to_string(),
            });
        }
    }
    Ok(())
}

/// A [`ContextProvider`] backed by a remote HTTP endpoint. Handshakes once on
/// [`HttpProvider::connect`] and caches the negotiated identity + capabilities.
pub struct HttpProvider {
    id: String,
    url: String,
    client: reqwest::Client,
    info: ProviderInfo,
    capabilities: Capabilities,
    /// Bearer credential attached to every request, if the provider requires
    /// one. Redacted from every rendering (C8).
    credential: Option<Credential>,
}

impl HttpProvider {
    /// Connect to a remote provider with no credential — a thin back-compat
    /// wrapper over [`connect_with_auth`](Self::connect_with_auth). POST a
    /// `handshake`, expect a compatible `handshake_ack`, and cache its identity
    /// + capabilities. `id` is the host-facing routing/consent key.
    pub async fn connect(id: impl Into<String>, url: impl Into<String>) -> Result<Self, HostError> {
        Self::connect_with_auth(id, url, None).await
    }

    /// Connect to a remote provider, optionally attaching a bearer
    /// [`Credential`] to every request. Enforces transport security before any
    /// bytes leave the host: a plaintext (`http://`) transport to a non-loopback
    /// provider is refused with [`HostError::InsecureTransport`], so neither the
    /// handshake nor a credential ever crosses the network in cleartext
    /// (`SPEC.md` §4.2, **C7**).
    pub async fn connect_with_auth(
        id: impl Into<String>,
        url: impl Into<String>,
        credential: Option<Credential>,
    ) -> Result<Self, HostError> {
        let id = id.into();
        let url = url.into();
        // C7 first, before the client is built or DNS is resolved: a refusal
        // must short-circuit with zero network activity so no payload leaks.
        refuse_insecure_transport(&id, &url)?;
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| HostError::Transport {
                id: id.clone(),
                message: format!("building HTTP client: {e}"),
            })?;

        let ack = post_envelope(
            &client,
            &url,
            &Envelope::Handshake {
                protocol_version: PROTOCOL_VERSION.to_string(),
            },
            &id,
            credential.as_ref(),
        )
        .await?;

        match ack {
            Envelope::HandshakeAck {
                protocol_version,
                provider,
                capabilities,
            } => {
                if !versions_compatible(PROTOCOL_VERSION, &protocol_version) {
                    return Err(HostError::VersionMismatch {
                        host: PROTOCOL_VERSION.to_string(),
                        provider: provider.name,
                        provider_version: protocol_version,
                    });
                }
                // An HTTP transport is egress by definition: every query is
                // POSTed off-box to a remote URL. So the consent gate must key
                // off transport, not the remote's self-report — a remote that
                // handshakes `egress:false` would otherwise be queried with no
                // consent. Force it on here regardless of what it declared.
                // (The stdio path keeps its declared posture; a local child
                // that doesn't reach the network genuinely may not be egress.)
                let mut info = provider;
                info.data_flow.egress = true;
                Ok(Self {
                    id,
                    url,
                    client,
                    info,
                    capabilities,
                    credential,
                })
            }
            other => Err(HostError::UnexpectedEnvelope {
                id,
                expected: "handshake_ack".into(),
                got: envelope_kind(&other).into(),
            }),
        }
    }
}

/// POST one envelope to the provider URL and decode the response as one
/// envelope. A non-2xx status or a non-envelope body is a clean named error,
/// never a panic (task deliverable 5).
///
/// When `credential` is present it is attached as `Authorization: Bearer …` via
/// reqwest's [`bearer_auth`](reqwest::RequestBuilder::bearer_auth) — never a
/// format string that could leak the secret into a log (C8).
async fn post_envelope(
    client: &reqwest::Client,
    url: &str,
    env: &Envelope,
    id: &str,
    credential: Option<&Credential>,
) -> Result<Envelope, HostError> {
    let mut request = client.post(url).json(env);
    if let Some(credential) = credential {
        request = request.bearer_auth(credential.expose());
    }
    let response = request.send().await.map_err(|e| HostError::Transport {
        id: id.to_string(),
        message: e.to_string(),
    })?;

    // A rejected credential is its own named error, distinct from any other
    // transport failure — and it names only the id + status, never the
    // credential (C8).
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(HostError::Unauthorized { id: id.to_string() });
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(HostError::Transport {
            id: id.to_string(),
            message: format!("HTTP {status}: {body}"),
        });
    }

    response.json::<Envelope>().await.map_err(|e| {
        HostError::Wire(format!(
            "provider {id} returned a non-envelope HTTP body: {e}"
        ))
    })
}

#[async_trait]
impl ContextProvider for HttpProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    async fn query(&self, query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
        let sent_id = self.capabilities.correlation.then(next_correlation_id);
        let reply = post_envelope(
            &self.client,
            &self.url,
            &Envelope::Query {
                id: sent_id.clone(),
                query: query.clone(),
            },
            &self.id,
            self.credential.as_ref(),
        )
        .await?;
        match reply {
            Envelope::Frames { id: echoed, result } => {
                verify_correlation(&self.id, sent_id.as_deref(), echoed.as_deref())?;
                Ok(result)
            }
            Envelope::Error { message, code, .. } => Err(HostError::Provider {
                id: self.id.clone(),
                code,
                message,
            }),
            other => Err(HostError::UnexpectedEnvelope {
                id: self.id.clone(),
                expected: "frames".into(),
                got: envelope_kind(&other).into(),
            }),
        }
    }

    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, HostError> {
        let reply = post_envelope(
            &self.client,
            &self.url,
            &Envelope::Verify {
                request: request.clone(),
            },
            &self.id,
            self.credential.as_ref(),
        )
        .await?;
        match reply {
            Envelope::Verified { response } => Ok(response),
            Envelope::Error { message, code, .. } => Err(HostError::Provider {
                id: self.id.clone(),
                code,
                message,
            }),
            other => Err(HostError::UnexpectedEnvelope {
                id: self.id.clone(),
                expected: "verified".into(),
                got: envelope_kind(&other).into(),
            }),
        }
    }

    async fn shutdown(&self) -> Result<(), HostError> {
        // Best-effort teardown notice; a remote endpoint is not ours to reap.
        let _ = post_envelope(
            &self.client,
            &self.url,
            &Envelope::Shutdown,
            &self.id,
            self.credential.as_ref(),
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextgraph_types::capability::QueryCapability;
    use contextgraph_types::{ContextFrame, DataFlow, FrameKind};
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ack_body(version: &str) -> serde_json::Value {
        serde_json::to_value(Envelope::HandshakeAck {
            protocol_version: version.to_string(),
            provider: ProviderInfo {
                name: "remote-docs".into(),
                version: "0.1.0".into(),
                data_flow: DataFlow {
                    reads: true,
                    writes: false,
                    egress: true,
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: QueryCapability {
                    kinds: vec!["doc".into()],
                },
                ..Capabilities::default()
            },
        })
        .unwrap()
    }

    fn frames_body() -> serde_json::Value {
        serde_json::to_value(Envelope::Frames {
            id: None,
            result: ContextQueryResult {
                frames: vec![ContextFrame {
                    id: "frm_h".into(),
                    kind: FrameKind::Doc,
                    title: "remote doc".into(),
                    content: Some("remote content".into()),
                    content_digest: None,
                    uri: Some("https://example.test/doc".into()),
                    representation: Default::default(),
                    content_fidelity: None,
                    canonical_content_hash: None,
                    content_ref: None,
                    transform: None,
                    minimum_content_fidelity: None,
                    inline_content_requirement: None,
                    score: 0.6,
                    token_cost: 20,
                    canonical_token_cost: None,
                    tokenizer_ref: None,
                    valid_from: None,
                    valid_to: None,
                    recorded_at: None,
                    provenance: vec![],
                    citation_label: Some("remote doc".into()),
                    embedding: None,
                    relations: vec![],
                }],
                truncated: false,
                dropped_estimate: None,
                ..Default::default()
            },
        })
        .unwrap()
    }

    fn sample_query() -> ContextQuery {
        ContextQuery {
            goal: "g".into(),
            query_text: None,
            embedding: None,
            kinds: vec![],
            anchors: vec![],
            max_frames: 5,
            max_tokens: 4000,
            as_of: None,
            representation_preferences: vec![],
        }
    }

    #[tokio::test]
    async fn http_handshake_then_query_round_trips_via_wiremock() {
        let server = MockServer::start().await;
        // Both the handshake and the query POST to the same URL; the responder
        // dispatches on the request envelope's `type`.
        Mock::given(method("POST"))
            .respond_with(|req: &wiremock::Request| {
                let body = match serde_json::from_slice::<Envelope>(&req.body) {
                    Ok(Envelope::Handshake { .. }) => ack_body(PROTOCOL_VERSION),
                    Ok(Envelope::Query { .. }) => frames_body(),
                    _ => serde_json::to_value(Envelope::Error {
                        id: None,
                        code: None,
                        message: "unexpected request".into(),
                    })
                    .unwrap(),
                };
                ResponseTemplate::new(200).set_body_json(body)
            })
            .mount(&server)
            .await;

        let provider = HttpProvider::connect("remote", server.uri())
            .await
            .expect("handshake ok");
        assert_eq!(provider.info().name, "remote-docs");
        assert!(provider.info().data_flow.egress);

        let result = provider.query(&sample_query()).await.expect("query ok");
        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].title, "remote doc");
    }

    #[tokio::test]
    async fn http_version_mismatch_rejects_the_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ack_body("contextgraph/2.0")))
            .mount(&server)
            .await;

        let err = match HttpProvider::connect("remote", server.uri()).await {
            Ok(_) => panic!("incompatible version must reject"),
            Err(e) => e,
        };
        assert!(matches!(err, HostError::VersionMismatch { .. }));
    }

    #[tokio::test]
    async fn a_non_envelope_http_body_is_a_clean_wire_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<html>not contextgraph</html>"),
            )
            .mount(&server)
            .await;

        let err = match HttpProvider::connect("remote", server.uri()).await {
            Ok(_) => panic!("garbage body must not panic the host"),
            Err(e) => e,
        };
        assert!(matches!(err, HostError::Wire(_)));
    }

    #[tokio::test]
    async fn http_transport_forces_egress_even_when_the_remote_claims_local() {
        // A remote self-declares `egress:false` in its handshake. Using an HTTP
        // transport IS egress (the query is POSTed off-box), so the host must
        // override the claim and still gate consent — otherwise a remote could
        // opt itself out of the consent gate by lying.
        let server = MockServer::start().await;
        let sneaky_ack = serde_json::to_value(Envelope::HandshakeAck {
            protocol_version: PROTOCOL_VERSION.to_string(),
            provider: ProviderInfo {
                name: "sneaky-remote".into(),
                version: "0.1.0".into(),
                data_flow: DataFlow {
                    reads: true,
                    writes: false,
                    egress: false, // the lie the host must not trust
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: QueryCapability {
                    kinds: vec!["doc".into()],
                },
                ..Capabilities::default()
            },
        })
        .unwrap();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sneaky_ack))
            .mount(&server)
            .await;

        let provider = HttpProvider::connect("remote", server.uri())
            .await
            .expect("handshake ok");

        assert!(
            provider.info().data_flow.egress,
            "an HTTP transport must be treated as egress regardless of the remote's claim"
        );
        assert!(
            crate::consent::ConsentStore::requires_consent(provider.info()),
            "an HTTP provider must always require consent, even claiming egress:false"
        );
    }

    // ---- transport security (§4.2, C7/C8) ----

    #[tokio::test]
    async fn a_plaintext_non_loopback_transport_is_refused_before_any_bytes_leave() {
        // C7: an `http://` (not `https://`) URL whose host is not loopback is
        // refused BEFORE a client is built or DNS is resolved — the query
        // payload and any credential must never cross the network in cleartext.
        // The proof it short-circuits is the error *kind*: a real network
        // attempt to this host would surface as a `Transport` (connect) error,
        // never `InsecureTransport`.
        let err = match HttpProvider::connect("remote", "http://example.com:9/cgp").await {
            Ok(_) => panic!("a plaintext non-loopback transport must be refused (C7)"),
            Err(e) => e,
        };
        match err {
            HostError::InsecureTransport { id, host } => {
                assert_eq!(id, "remote");
                assert_eq!(host, "example.com");
            }
            other => panic!("expected InsecureTransport, got {other:?}"),
        }
    }

    /// The C7 rule is now public API ([`refuse_insecure_transport`]) so a host can
    /// classify a URL without re-deriving "which hosts are loopback" locally. That
    /// makes these edge cases part of the exported contract rather than an
    /// internal detail, so they are pinned directly instead of only through
    /// `connect`: they are exactly the cases an independent reimplementation gets
    /// wrong, and the reason the rule is exported at all.
    #[test]
    fn the_exported_c7_rule_classifies_every_loopback_spelling() {
        // Allowed: TLS anywhere, and plaintext to loopback in each of its
        // spellings — the literal name (any casing), all of `127.0.0.0/8` rather
        // than just `127.0.0.1`, and bracketed IPv6 `::1`.
        for allowed in [
            "https://example.com/cgp",
            "http://localhost:8080/cgp",
            "http://LOCALHOST:8080/cgp",
            "http://127.0.0.1/cgp",
            "http://127.0.0.2/cgp",
            "http://[::1]:8080/cgp",
        ] {
            assert!(
                refuse_insecure_transport("p", allowed).is_ok(),
                "C7 must allow {allowed}"
            );
        }

        // Refused: plaintext to anything off-machine. `127.0.0.1.example.com` is
        // the prefix-matching trap — it *starts with* a loopback IP and is a
        // remote DNS name.
        for refused in [
            "http://example.com/cgp",
            "http://127.0.0.1.example.com/cgp",
            "http://[2001:db8::1]/cgp",
            "http://10.0.0.5/cgp",
        ] {
            assert!(
                matches!(
                    refuse_insecure_transport("p", refused),
                    Err(HostError::InsecureTransport { .. })
                ),
                "C7 must refuse {refused}"
            );
        }

        // An unparseable URL is a config error, not a security verdict: reporting
        // it as `InsecureTransport` would tell an operator to add TLS to a string
        // that is not a URL at all.
        assert!(matches!(
            refuse_insecure_transport("p", "not a url"),
            Err(HostError::Transport { .. })
        ));
    }

    #[tokio::test]
    async fn a_plaintext_loopback_transport_is_allowed() {
        // The C7 loopback exception: wiremock serves plain `http://` on
        // `127.0.0.1`, and the host must NOT refuse it — the bytes never leave
        // the machine. This is also what keeps every other wiremock test in this
        // module (all on 127.0.0.1) working.
        let server = MockServer::start().await;
        assert!(
            server.uri().starts_with("http://"),
            "wiremock serves plaintext http on loopback"
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ack_body(PROTOCOL_VERSION)))
            .mount(&server)
            .await;
        let provider = HttpProvider::connect("remote", server.uri())
            .await
            .expect("a plaintext loopback (127.0.0.1) transport is allowed");
        assert_eq!(provider.info().name, "remote-docs");
    }

    #[tokio::test]
    async fn a_supplied_credential_is_attached_as_a_bearer_header() {
        const TOKEN: &str = "s3cr3t-bearer-token-value";
        let server = MockServer::start().await;
        let auth_value = format!("Bearer {TOKEN}");
        // The mock only matches when the `Authorization` header is present and
        // exact. If the header were missing (or mangled), no mock matches,
        // wiremock 404s, and the handshake/query below fail — so a green test
        // proves the bearer credential was attached on the wire.
        Mock::given(method("POST"))
            .and(header("authorization", auth_value.as_str()))
            .respond_with(|req: &wiremock::Request| {
                let body = match serde_json::from_slice::<Envelope>(&req.body) {
                    Ok(Envelope::Handshake { .. }) => ack_body(PROTOCOL_VERSION),
                    Ok(Envelope::Query { .. }) => frames_body(),
                    _ => serde_json::to_value(Envelope::Error {
                        id: None,
                        code: None,
                        message: "unexpected request".into(),
                    })
                    .unwrap(),
                };
                ResponseTemplate::new(200).set_body_json(body)
            })
            .mount(&server)
            .await;

        let provider = HttpProvider::connect_with_auth(
            "remote",
            server.uri(),
            Some(Credential::bearer(TOKEN)),
        )
        .await
        .expect("handshake carries the bearer credential");
        // The query carries it too — the same header matcher gates its response.
        let result = provider.query(&sample_query()).await.expect("query ok");
        assert_eq!(result.frames.len(), 1);
    }

    #[test]
    fn a_credential_is_redacted_in_every_rendering_and_never_in_an_error() {
        // C8: the secret must not appear in any `{:?}`/`{}` rendering — a
        // credential that reaches a log line or a panic payload prints a fixed
        // placeholder, not its bytes.
        const SECRET: &str = "ghp_this_must_never_appear_in_a_log_0xDEADBEEF";
        let credential = Credential::bearer(SECRET);

        let debug = format!("{credential:?}");
        let display = format!("{credential}");
        assert_eq!(debug, "Credential(<redacted>)");
        assert_eq!(display, "Credential(<redacted>)");
        assert!(
            !debug.contains(SECRET),
            "Debug must not leak the secret (C8)"
        );
        assert!(
            !display.contains(SECRET),
            "Display must not leak the secret (C8)"
        );
        // Cloning preserves redaction — a duplicated credential still can't leak.
        assert_eq!(
            format!("{:?}", credential.clone()),
            "Credential(<redacted>)"
        );

        // No `HostError` carries credential material: the auth-related variants
        // render only id/host/status, so a secret can never reach a surfaced
        // error string (C8).
        let insecure = HostError::InsecureTransport {
            id: "remote".into(),
            host: "example.com".into(),
        };
        let unauthorized = HostError::Unauthorized {
            id: "remote".into(),
        };
        assert!(!insecure.to_string().contains(SECRET));
        assert!(!unauthorized.to_string().contains(SECRET));
    }
}
