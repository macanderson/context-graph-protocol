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
use sha2::{Digest, Sha256};

use contextgraph_host::wire::Envelope;
use contextgraph_types::capability::{QueryCapability, fingerprint_dimensions};
use contextgraph_types::frame::rel;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQuery, ContextQueryResult, DataFlow, EgressScope, ErrorCode,
    FrameKind, FrameVerdict, PROTOCOL_VERSION, Provenance, ProviderInfo, Relation, Representation,
    Verdict, VerifyRequest, VerifyResponse, budget_tokens,
};

/// The embedding space this fixture declares it indexes (`SPEC.md` §E1). Its
/// dimension (384) is the number a query embedding's length must match; a
/// contradicting length is rejected `bad_request` unless the fixture is in
/// `accept-bad-embedding` mode.
const EMBEDDING_FINGERPRINT: &str = "bge-small-en-v1.5/384/l2";

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
    /// Stay alive on a malformed line but answer it with `internal` instead of
    /// the `bad_request` §R1 recommends — a structured error that is not the
    /// right one (trips `malformed-input-tolerance`).
    MislabelMalformed,
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
    /// Emit a WELL-FORMED `sha256:<64 hex>` file-provenance digest that passes
    /// §F5's grammar but does not match the backing file's real bytes — one hex
    /// digit of the real digest flipped (trips `provenance-fixture-consistency`).
    ///
    /// DISTINCT from `MalformedDigest`: that stub is caught by `frame-validity`
    /// because it is not *shaped* like a digest; this one is shaped correctly and
    /// is self-consistent over the wire, so only a host re-reading the bytes the
    /// digest claims to cover catches it (`SPEC.md` §6.2).
    StaleDigest,
    /// Return far more frames than the query's `max_frames` allows, each
    /// individually cheap so the token budget is respected (trips
    /// `budget-honesty` §B4).
    FloodFrames,
    /// Answer a correlated query without echoing its `id` (trips
    /// `correlation`).
    DropCorrelationId,
    /// Emit a frame that lies about how it carries its content — a `reference`
    /// representation still carrying inline content, which its declared shape
    /// forbids (trips `frame-validity` §P1–P3).
    LyingRepresentation,
    /// Return a frame whose `valid_from` is after the query's `as_of` pin —
    /// content that was not yet true at the pinned instant (trips
    /// `as-of-temporal` §F4/§6.1).
    IgnoreAsOf,
    /// Score a query embedding whose length contradicts the declared
    /// `embeddings_fingerprint` dimension instead of rejecting it (trips
    /// `embedding-fingerprint` §E1).
    AcceptBadEmbedding,
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
    /// Ignore a non-empty `query.kinds`, returning frames of a kind the host
    /// explicitly excluded (trips `kinds-filter`).
    IgnoreKinds,
    /// Declare `capabilities.graph` but ignore `query.anchors`, dropping the
    /// anchored frame instead of boosting it (trips `anchor-relevance`).
    IgnoreAnchors,
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
                // §R1 recommends `bad_request`; `mislabel-malformed` answers
                // with `internal` instead, to prove the malformed-input check
                // now inspects the *code* rather than passing on any error.
                let code = if args.misbehave == Some(Misbehave::MislabelMalformed) {
                    ErrorCode::Internal
                } else {
                    ErrorCode::BadRequest
                };
                write_envelope(
                    &mut stdout,
                    &Envelope::Error {
                        id: None,
                        code: Some(code),
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
            Envelope::Query { id, query } => {
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
                // §E1: a query embedding whose length contradicts this
                // provider's declared fingerprint dimension names a different
                // vector space; scoring it would yield plausible-looking,
                // meaningless similarity. An honest provider rejects it
                // `bad_request` rather than pretending. `accept-bad-embedding`
                // scores it anyway, which the `embedding-fingerprint` probe
                // catches.
                if args.misbehave != Some(Misbehave::AcceptBadEmbedding)
                    && let Some(error) = embedding_dimension_error(&query, echoed.clone())
                {
                    write_envelope(&mut stdout, &error);
                    continue;
                }
                let mut frames = canned_frames(args.misbehave);
                // §Q1: a non-empty `kinds` is a filter, not a hint. Returning a
                // frame outside it spends the host's budget on content it
                // explicitly excluded. `ignore-kinds` skips this, which the
                // `kinds-filter` probe catches.
                if args.misbehave != Some(Misbehave::IgnoreKinds) && !query.kinds.is_empty() {
                    frames.retain(|f| query.kinds.contains(&f.kind));
                }
                // §G4: a frame is anchored when its own `uri`, or any of its
                // relations' `target_uri`, equals one of the query's anchors.
                // A graph-declaring provider must return an anchored frame when
                // it has one, and should rank anchored above unanchored — the
                // "boost" G3 asks for, made decidable. `ignore-anchors` drops
                // the anchored frame instead, which `anchor-relevance` catches.
                if !query.anchors.is_empty() {
                    if args.misbehave == Some(Misbehave::IgnoreAnchors) {
                        frames.retain(|f| !is_anchored(f, &query.anchors));
                    } else {
                        // A stable partition: anchored frames first, relative
                        // order otherwise preserved, so composition stays
                        // deterministic.
                        frames.sort_by_key(|f| !is_anchored(f, &query.anchors));
                    }
                }
                // §F4/§6.1: honor an `as_of` pin — content not yet true at the
                // pinned instant is not returned. The timestamp profile is one
                // spelling per instant, so a lexicographic compare on the UTC
                // strings *is* a chronological one. `ignore-as-of` skips this,
                // returning a not-yet-valid frame the `as-of-temporal` probe
                // catches.
                if args.misbehave != Some(Misbehave::IgnoreAsOf)
                    && let Some(as_of) = query.as_of.as_deref()
                {
                    frames.retain(|f| !f.valid_from.as_deref().is_some_and(|vf| vf > as_of));
                }
                write_envelope(
                    &mut stdout,
                    &Envelope::Frames {
                        id: echoed,
                        result: ContextQueryResult {
                            frames,
                            truncated: false,
                            dropped_estimate: None,
                            ..Default::default()
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
                    _ => verify_honestly(&request, args.misbehave),
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
        // The protocol is named for the graph, and this fixture used to declare
        // `graph: false` with `relations: vec![]` on every frame — so G1/G2
        // passed vacuously and G3's anchor boost was never witnessed at all.
        // It now serves real labelled edges and honors `anchors` (§G4).
        graph: true,
        // Declaring the embedding space it indexes lets the provider reject a
        // vector from a different one (§E1). A provider that declares no
        // fingerprint has nothing to contradict and is not E1-probed.
        embeddings_fingerprint: Some(EMBEDDING_FINGERPRINT.into()),
        // This fixture can compare a presented digest against the one it
        // currently serves, so it advertises pull-based verification. It serves
        // inline full frames only; it does not resolve references.
        verify: true,
        representations: vec![],
        resolve: false,
    }
}

/// The `bad_request` reply for a query embedding whose length contradicts this
/// provider's declared fingerprint dimension (`SPEC.md` §E1), or `None` when the
/// query carries no embedding or one of the correct length.
///
/// A vector of the wrong dimension is not "close enough" — it is a vector from a
/// different space, and the similarity scores it would produce are meaningless.
/// Replying with a *code* (not just prose) is what lets a host tell "your
/// request was wrong" from "I am broken" without sniffing message strings.
fn embedding_dimension_error(query: &ContextQuery, id: Option<String>) -> Option<Envelope> {
    let embedding = query.embedding.as_ref()?;
    let expected = fingerprint_dimensions(EMBEDDING_FINGERPRINT)?;
    if embedding.len() == expected {
        return None;
    }
    Some(Envelope::Error {
        id,
        code: Some(ErrorCode::BadRequest),
        message: format!(
            "query embedding has {} dimensions; this provider indexes {} ({EMBEDDING_FINGERPRINT}) (§E1)",
            embedding.len(),
            expected
        ),
    })
}

/// The directory holding this reference provider's on-disk backing files,
/// resolved at compile time so a digest is computed over the same bytes no
/// matter where the fixture is spawned from (`SPEC.md` §6.2).
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/example-docs");

/// The absolute `file://` URI a host re-reads to verify a frame's provenance
/// digest (`contextgraph_host::verify::verify_file_provenance`). Absolute and
/// cwd-independent, so verification never depends on the host's working
/// directory.
fn fixture_uri(file: &str) -> String {
    format!("file://{FIXTURE_DIR}/{file}")
}

/// The real `sha256:<64 lowercase hex>` digest over a backing file's exact
/// on-disk bytes — byte-for-byte what `contextgraph_host::verify` recomputes when
/// it re-reads the file, so an unmutated frame verifies end to end (§6.2, §F5).
///
/// A name the fixture does not actually ship (the synthetic `flood.md`) hashes
/// as the empty input, which is still a *well-formed* sha256 — enough for §F5's
/// grammar, since the flood mode's violation is its frame count, not its digest.
fn fixture_digest(file: &str) -> String {
    let bytes = std::fs::read(format!("{FIXTURE_DIR}/{file}")).unwrap_or_default();
    let hex: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// Flip the last hex digit of a well-formed digest, yielding one that still
/// passes §F5's `sha256:<64 hex>` grammar but no longer matches the bytes it
/// names — the `stale-digest` forgery. The nibble is moved to a
/// guaranteed-different lowercase hex digit, so the result can never coincide
/// with the real digest.
fn stale_digest(real: &str) -> String {
    let mut digest = real.to_string();
    if let Some(last) = digest.pop() {
        digest.push(if last == '0' { '1' } else { '0' });
    }
    digest
}

/// The digest a frame declares for `file`, honoring the two DISTINCT
/// digest-integrity misbehave modes:
///
///   * [`Misbehave::MalformedDigest`] emits `sha256:abc`, which fails §F5's
///     *grammar* — `frame-validity` rejects it before any bytes are read;
///   * [`Misbehave::StaleDigest`] emits a *well-formed* digest that passes the
///     grammar but does not match the file's bytes — only re-reading the file
///     (`provenance-fixture-consistency`) catches it.
fn declared_digest(file: &str, misbehave: Option<Misbehave>) -> String {
    match misbehave {
        Some(Misbehave::MalformedDigest) => "sha256:abc".to_string(),
        Some(Misbehave::StaleDigest) => stale_digest(&fixture_digest(file)),
        _ => fixture_digest(file),
    }
}

/// The digest this fixture serves for a frame id *right now*, or `None` if it
/// does not serve that frame at all. Threaded through `misbehave` so
/// `stale-digest` stays internally *consistent* over the wire: the provider
/// vouches for the very (forged) digest it served, so `verify-honesty` still
/// passes and the forgery is left for `provenance-fixture-consistency` alone to
/// catch by re-reading the file.
fn current_digest(frame_id: &str, misbehave: Option<Misbehave>) -> Option<String> {
    match frame_id {
        "frm_getting_started" => Some(declared_digest("getting-started.md", misbehave)),
        "frm_configuration" => Some(declared_digest("configuration.md", misbehave)),
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
fn verify_honestly(request: &VerifyRequest, misbehave: Option<Misbehave>) -> VerifyResponse {
    VerifyResponse::new(
        request
            .frames
            .iter()
            .map(|frame| {
                let verdict = match current_digest(&frame.frame_id, misbehave) {
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

/// Whether `frame` is anchored by any of `anchors` (`SPEC.md` §G4): its own
/// `uri` at zero hops, or any labelled edge's `target_uri` at one hop.
fn is_anchored(frame: &ContextFrame, anchors: &[String]) -> bool {
    let zero_hop = frame
        .uri
        .as_deref()
        .is_some_and(|u| anchors.iter().any(|a| a == u));
    zero_hop
        || frame
            .relations
            .iter()
            .any(|r| anchors.contains(&r.target_uri))
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
        // Valid since the start of the year — before the `as_of` probe's pin.
        doc_frame(
            "frm_getting_started",
            "Getting Started",
            "Install the reference binding with `cargo add contextgraph-types`, then implement \
             the four required methods.",
            "getting-started.md",
            "L1-40",
            "2026-01-01T00:00:00Z",
            0.82,
            misbehave,
        ),
        // Became true only in the autumn — *after* the `as_of` probe's pin, so
        // an as_of-honoring provider omits it from a mid-year pinned query.
        //
        // Deliberately a **different kind** from the frame above. The §Q1 probe
        // narrows to the first kind this provider declares (`doc`), so the
        // fixture has to serve something *outside* that narrowing for the
        // filter to have observable work to do. When every frame shared one
        // kind, a provider that ignored `kinds` entirely still passed the
        // check — a decorative check is the thing this round is removing, not
        // adding.
        {
            let mut frame = doc_frame(
                "frm_configuration",
                "Configuration example",
                "let host = Host::new().with_provider(\"docs\", provider);",
                "configuration.md",
                "L1-25",
                "2026-09-01T00:00:00Z",
                0.61,
                misbehave,
            );
            frame.kind = FrameKind::Snippet;
            frame
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, mut frame)| {
        // Only the first frame carries the score/citation/representation
        // defects, so a single failure is attributable to a single frame in
        // the evidence string.
        if index == 0 {
            if bad_score {
                frame.score = 1.5;
            }
            if empty_citation {
                frame.citation_label = Some(String::new());
            }
            if misbehave == Some(Misbehave::LyingRepresentation) {
                // Claim `reference` while still carrying inline content and an
                // inline digest — a structural lie the representation forbids
                // (§P3). The frame is otherwise well-formed and honestly
                // costed, so only `representation_invariants` catches it.
                frame.representation = Representation::Reference;
            }
        }
        frame
    })
    .collect()
}

/// A frame with the defect selected by `misbehave` applied, if any.
///
/// `valid_from` is the instant the frame's content became true in the world
/// (§6.1); callers give the two canned frames *disjoint* windows so an `as_of`
/// pin between them is observable — the `as-of-temporal` probe depends on it.
#[allow(clippy::too_many_arguments)]
fn doc_frame(
    id: &str,
    title: &str,
    content: &str,
    file: &str,
    range: &str,
    valid_from: &str,
    score: f32,
    misbehave: Option<Misbehave>,
) -> ContextFrame {
    let honest_cost = budget_tokens(content);
    ContextFrame {
        id: id.into(),
        kind: FrameKind::Doc,
        title: title.into(),
        content: Some(content.into()),
        // The real sha256 over the backing file's bytes — identical to this
        // frame's file-provenance digest, so a host re-reading the file confirms
        // both (§6.2, §F5). `stale-digest` flips one hex digit (well-formed but
        // wrong bytes); `malformed-digest` replaces it with an ungrammatical stub.
        content_digest: Some(declared_digest(file, misbehave)),
        uri: Some(fixture_uri(file)),
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
            _ => valid_from.to_string(),
        }),
        valid_to: None,
        recorded_at: Some("2026-07-20T18:00:00Z".into()),
        provenance: vec![Provenance {
            kind: "file".into(),
            uri: Some(fixture_uri(file)),
            range: Some(range.into()),
            // The same declared digest as `content_digest`, so a host that
            // re-reads `uri` over `range` and re-hashes gets a match for an
            // honest frame — and a `Mismatch` under `stale-digest` (§6.2).
            digest: Some(declared_digest(file, misbehave)),
            method: None,
            by: Some("contextgraph-example-docs".into()),
        }],
        citation_label: Some(format!("{file} {range}")),
        embedding: None,
        // A labelled edge to the symbol this page documents. §G4 makes a frame
        // "anchored" when its own `uri` or any `relations[].target_uri` equals
        // a query anchor, so this edge is what an anchored query reaches at one
        // hop.
        relations: vec![Relation {
            rel: rel::DOC_DOCUMENTS.into(),
            target_uri: format!("symbol:///docs/{file}#overview"),
            display_name: Some(format!("{title} overview")),
        }],
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
        "2026-01-01T00:00:00Z",
        0.5,
        misbehave.filter(|m| !matches!(m, Misbehave::FloodFrames)),
    )
}
