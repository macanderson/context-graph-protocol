//! `contextgraph-example-docs` — a minimal reference Context Graph Protocol provider over stdio.
//!
//! It serves a couple of canned "documentation" frames, proving the external
//! child-process path end-to-end (`SPEC.md` §11 seed
//! providers). It is also the child-process **test fixture** for the
//! conformance suite: `--misbehave <mode>` deliberately breaks one protocol
//! guarantee at a time so tests can prove the suite catches a broken
//! provider (task deliverable). It reuses `contextgraph-host`'s `wire::Envelope` for
//! (de)serialization since both live in this workspace; a real out-of-tree
//! provider — in any language — would instead implement the line-oriented
//! wire format directly against `contextgraph-types` (the frame/query types) plus a
//! JSON codec, which is the only contract it must honor.

use std::io::{BufRead, Write};

use clap::{Parser, ValueEnum};
use contextgraph_host::wire::Envelope;
use contextgraph_types::capability::QueryCapability;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQueryResult, DataFlow, EgressScope, ErrorCode, FrameKind,
    FrameVerdict, PROTOCOL_VERSION, Provenance, ProviderInfo, Representation, Verdict,
    VerifyRequest, VerifyResponse, budget_tokens,
};

/// Ways this fixture can deliberately violate the protocol, each tripping a
/// different conformance check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum Misbehave {
    /// Return frames whose summed `token_cost` blows the query budget
    /// (trips `budget-honesty`).
    LyingCosts,
    /// Return a frame with a score outside `[0,1]` (trips `frame-validity`).
    BadScore,
    /// Return a frame with an empty citation label (trips `frame-validity`).
    EmptyCitation,
    /// Ack an incompatible protocol version (trips `handshake`).
    BadVersion,
    /// Exit on receiving a query (trips `frame-validity`/`budget-honesty`
    /// and exercises the host's child-death isolation).
    CrashOnQuery,
    /// Exit on receiving a malformed line (trips
    /// `malformed-input-tolerance`).
    CrashOnGarbage,
    /// Declare a `token_cost` far below the canonical count for the content
    /// actually served (trips `budget-honesty` §B3).
    ///
    /// This is the mode that matters most: before the canonical counting rule
    /// existed, this provider passed every check in the suite while destroying
    /// the host's real budget.
    UnderReportCost,
    /// Emit a temporal field that is not in the protocol's timestamp profile
    /// (trips `frame-validity` §F4).
    BadTimestamp,
    /// Emit file provenance whose digest does not match the `sha256:<64 hex>`
    /// grammar (trips `frame-validity` §F5).
    MalformedDigest,
    /// Return far more frames than the query's `max_frames` allows, each
    /// individually cheap so the token budget is respected (trips
    /// `budget-honesty` §B4).
    FloodFrames,
    /// Answer a correlated query without echoing its `id` (trips
    /// `correlation`).
    DropCorrelationId,
    /// Answer `valid` to every `context/verify` entry without comparing
    /// digests — the rubber stamp that makes reuse unsafe (trips
    /// `verify-honesty`).
    RubberStampVerify,
    /// Advertise `verify` support at the handshake but answer `unknown` to
    /// everything — a provider that claims a capability it does not have
    /// (trips `verify-honesty`).
    HollowVerify,
    /// Declare an off-machine egress scope (`third-party-index`) alongside
    /// `egress: false` — a scope that contradicts the data-flow posture
    /// (trips `consent-scope`).
    ScopeLie,
}

#[derive(Parser)]
#[command(
    name = "contextgraph-example-docs",
    about = "A tiny reference Context Graph Protocol provider serving canned documentation frames over stdio."
)]
struct Args {
    /// Deliberately break one protocol guarantee (for conformance testing).
    #[arg(long, value_enum)]
    misbehave: Option<Misbehave>,
}

fn main() {
    let args = Args::parse();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or a broken pipe — the host is gone.
            Ok(_) => {}
        }

        let envelope = match serde_json::from_str::<Envelope>(line.trim_end()) {
            Ok(envelope) => envelope,
            Err(_) => {
                // A malformed line: a robust provider stays alive and says so
                // (`SPEC.md` §R1); the misbehaving one dies, to prove the suite
                // notices. Replying with a *code* rather than only prose is
                // what lets the host distinguish "your request was wrong" from
                // "I am broken" without sniffing message strings.
                if args.misbehave == Some(Misbehave::CrashOnGarbage) {
                    std::process::exit(1);
                }
                write_envelope(
                    &mut stdout,
                    &Envelope::Error {
                        id: None,
                        code: Some(ErrorCode::BadRequest),
                        message: "line was not a valid CGP envelope".into(),
                    },
                );
                continue;
            }
        };

        match envelope {
            Envelope::Handshake { .. } => {
                let protocol_version = if args.misbehave == Some(Misbehave::BadVersion) {
                    "contextgraph/2.0".to_string()
                } else {
                    PROTOCOL_VERSION.to_string()
                };
                write_envelope(
                    &mut stdout,
                    &Envelope::HandshakeAck {
                        protocol_version,
                        provider: provider_info(args.misbehave),
                        capabilities: capabilities(),
                    },
                );
            }
            Envelope::Query { id, .. } => {
                if args.misbehave == Some(Misbehave::CrashOnQuery) {
                    std::process::exit(1);
                }
                // Echo the correlation id so the host can match this reply to
                // its request (`SPEC.md` §H4). Dropping it is a misbehaviour
                // mode of its own, because a host that silently accepted an
                // uncorrelated reply could hand frames to the wrong caller.
                let echoed = if args.misbehave == Some(Misbehave::DropCorrelationId) {
                    None
                } else {
                    id
                };
                write_envelope(
                    &mut stdout,
                    &Envelope::Frames {
                        id: echoed,
                        result: ContextQueryResult {
                            frames: canned_frames(args.misbehave),
                            truncated: false,
                            dropped_estimate: None,
                        },
                    },
                );
            }
            Envelope::Verify { request } => {
                let response = match args.misbehave {
                    // Claims every held frame is still good without comparing
                    // a single digest — the lie `verify-honesty` catches.
                    Some(Misbehave::RubberStampVerify) => {
                        VerifyResponse::uniform(&request, Verdict::Valid)
                    }
                    // Advertises verify but can never vouch for anything.
                    Some(Misbehave::HollowVerify) => {
                        VerifyResponse::uniform(&request, Verdict::Unknown)
                    }
                    _ => verify_honestly(&request),
                };
                write_envelope(&mut stdout, &Envelope::Verified { response });
            }
            Envelope::Shutdown => std::process::exit(0),
            // handshake_ack / frames / error are host→provider-invalid inputs;
            // a provider ignores them.
            _ => {}
        }
    }
}

fn write_envelope(stdout: &mut std::io::Stdout, envelope: &Envelope) {
    // A provider is a plain pipe writer; if the host has gone, give up quietly.
    if let Ok(line) = serde_json::to_string(envelope) {
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}

fn provider_info(misbehave: Option<Misbehave>) -> ProviderInfo {
    // A docs index reads the query and serves local frames; nothing leaves the
    // machine, so it honestly declares the `local-only` egress scope. The
    // `scope-lie` mode instead declares an off-machine scope alongside
    // `egress: false` — a contradiction the `consent-scope` check must catch.
    let (egress, egress_scopes) = if misbehave == Some(Misbehave::ScopeLie) {
        (false, vec![EgressScope::ThirdPartyIndex])
    } else {
        (false, vec![EgressScope::LocalOnly])
    };
    ProviderInfo {
        name: "contextgraph-example-docs".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        data_flow: DataFlow {
            reads: true,
            writes: false,
            egress,
            egress_scopes,
        },
    }
}

fn capabilities() -> Capabilities {
    Capabilities {
        query: QueryCapability {
            kinds: vec!["doc".into(), "snippet".into()],
        },
        correlation: true,
        graph: false,
        embeddings_fingerprint: None,
        // This fixture can compare a presented digest against the one it
        // currently serves, so it advertises pull-based verification. It serves
        // inline full frames only; it does not resolve references.
        verify: true,
        representations: vec![],
        resolve: false,
    }
}

/// A syntactically valid `sha256:` digest for a fixture whose bytes are canned
/// rather than read from disk.
///
/// The value is stable but not a real hash of anything: this fixture serves
/// string literals, not files, so there are no on-disk bytes to digest. The
/// `frame-validity` check it feeds asserts the *grammar* (`SPEC.md` §F5), which
/// is what catches the `sha256:abc` placeholders that were previously
/// conformant. Verifying a digest against real bytes is a host-side concern.
fn fixture_digest(seed: u8) -> String {
    format!("sha256:{}", format!("{seed:02x}").repeat(32))
}

/// The digest this fixture serves for a frame id *right now*, or `None` if it
/// does not serve that frame at all. Uses the same `fixture_digest` seeds the
/// served frames declare, so an unmutated frame verifies `valid`.
fn current_digest(frame_id: &str) -> Option<String> {
    match frame_id {
        "frm_getting_started" => Some(fixture_digest(1)),
        "frm_configuration" => Some(fixture_digest(2)),
        _ => None,
    }
}

/// Answer a `context/verify` request honestly, by comparing each presented
/// digest against the one this provider currently serves (`docs/context-reuse.md` §4).
///
/// This is the whole provider-side contract: the digest is provider-declared
/// and opaque, so only the provider can say whether the bytes behind an
/// identity still match. A digest that differs from the current one is exactly
/// what a mutated source looks like from here.
fn verify_honestly(request: &VerifyRequest) -> VerifyResponse {
    VerifyResponse::new(
        request
            .frames
            .iter()
            .map(|frame| {
                let verdict = match current_digest(&frame.frame_id) {
                    // Never served, or no longer served: nothing to revalidate.
                    None => Verdict::Gone,
                    Some(current) => match frame.content_digest.as_deref() {
                        // Nothing presented to compare against.
                        None => Verdict::Unknown,
                        Some(presented) if current.as_str() == presented => Verdict::Valid,
                        // The source moved on. Offer the current digest so the
                        // host can tell what it would be re-fetching.
                        Some(_) => Verdict::Stale {
                            replacement_digest: Some(current),
                        },
                    },
                };
                FrameVerdict::new(frame.clone(), verdict)
            })
            .collect(),
    )
}

fn canned_frames(misbehave: Option<Misbehave>) -> Vec<ContextFrame> {
    let bad_score = misbehave == Some(Misbehave::BadScore);
    let empty_citation = misbehave == Some(Misbehave::EmptyCitation);

    if misbehave == Some(Misbehave::FloodFrames) {
        // Each frame is individually honest and nearly free, so the token
        // budget is respected — the violation is purely the frame count, which
        // nothing audited before §B4.
        return (0..64)
            .map(|i| {
                let mut frame = base_frame(bad_score, empty_citation, misbehave);
                frame.id = format!("frm_flood_{i}");
                frame.content = Some("x".into());
                frame.token_cost = frame.expected_inline_token_cost();
                frame
            })
            .collect();
    }

    vec![
        doc_frame(
            "frm_getting_started",
            "Getting Started",
            "Install the reference binding with `cargo add contextgraph-types`, then implement \
             the four required methods.",
            "getting-started.md",
            "L1-40",
            0.82,
            1,
            misbehave,
        ),
        doc_frame(
            "frm_configuration",
            "Configuration",
            "Providers declare their data-flow direction at the handshake so hosts can \
             gate consent before sending any query.",
            "configuration.md",
            "L1-25",
            0.61,
            2,
            misbehave,
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, mut frame)| {
        // Only the first frame carries the score/citation defects, so a single
        // failure is attributable to a single frame in the evidence string.
        if index == 0 {
            if bad_score {
                frame.score = 1.5;
            }
            if empty_citation {
                frame.citation_label = Some(String::new());
            }
        }
        frame
    })
    .collect()
}

/// A frame with the defect selected by `misbehave` applied, if any.
#[allow(clippy::too_many_arguments)]
fn doc_frame(
    id: &str,
    title: &str,
    content: &str,
    file: &str,
    range: &str,
    score: f32,
    digest_seed: u8,
    misbehave: Option<Misbehave>,
) -> ContextFrame {
    let honest_cost = budget_tokens(content);
    ContextFrame {
        id: id.into(),
        kind: FrameKind::Doc,
        title: title.into(),
        content: Some(content.into()),
        // The digest of the content bytes, matching this frame's file
        // provenance digest — a well-formed digest keeps the frame's identity
        // verifiable (`docs/context-reuse.md` §1) and satisfies §F5's grammar.
        content_digest: Some(match misbehave {
            Some(Misbehave::MalformedDigest) => "sha256:abc".into(),
            _ => fixture_digest(digest_seed),
        }),
        uri: Some(format!("file:///docs/{file}")),
        // This fixture serves inline `full` frames only.
        representation: Representation::Full,
        content_fidelity: None,
        canonical_content_hash: None,
        content_ref: None,
        transform: None,
        minimum_content_fidelity: None,
        inline_content_requirement: None,
        score,
        token_cost: match misbehave {
            // Claims an absurd cost so the sum blows any sane budget (§B1).
            Some(Misbehave::LyingCosts) => 99_999,
            // Claims almost nothing while serving the full body (§B3). This is
            // the lie the old arithmetic-only check could not see.
            Some(Misbehave::UnderReportCost) => 1,
            _ => honest_cost,
        },
        canonical_token_cost: None,
        tokenizer_ref: None,
        valid_from: Some(match misbehave {
            Some(Misbehave::BadTimestamp) => "last tuesday".into(),
            _ => "2026-01-01T00:00:00Z".to_string(),
        }),
        valid_to: None,
        recorded_at: Some("2026-07-20T18:00:00Z".into()),
        provenance: vec![Provenance {
            kind: "file".into(),
            uri: Some(format!("file:///docs/{file}")),
            range: Some(range.into()),
            digest: Some(match misbehave {
                // The placeholder shape the pre-spec fixtures used, which is
                // not a digest and no longer passes for one.
                Some(Misbehave::MalformedDigest) => "sha256:abc".into(),
                _ => fixture_digest(digest_seed),
            }),
            method: None,
            by: Some("contextgraph-example-docs".into()),
        }],
        citation_label: Some(format!("{file} {range}")),
        embedding: None,
        relations: vec![],
    }
}

/// The frame the flood mode clones. Shares `doc_frame`'s defect handling so a
/// flooded response is otherwise perfectly conformant.
fn base_frame(
    _bad_score: bool,
    _empty_citation: bool,
    misbehave: Option<Misbehave>,
) -> ContextFrame {
    doc_frame(
        "frm_flood",
        "Flood",
        "x",
        "flood.md",
        "L1",
        0.5,
        3,
        misbehave.filter(|m| !matches!(m, Misbehave::FloodFrames)),
    )
}
