//! `contextgraph-conformance` — the public Context Graph Protocol conformance suite
//! (`SPEC.md` §11).
//!
//! "Context Graph Protocol conformant" means *green on this suite for your declared capability
//! set* — a checkable claim, which is what makes third-party adoption safe.
//! [`run_conformance`] drives a provider through the protocol and returns a
//! typed [`ConformanceReport`] with a pass/fail verdict per check and an
//! evidence string for each, so a failure says exactly what was wrong.
//!
//! The checks (all against the frozen `contextgraph-types` contracts):
//!
//! - **handshake** — the provider completes the handshake and reports a
//!   non-empty identity + capabilities (SPEC.md §3).
//! - **consent-scope** — the provider's declared egress scopes are well-formed
//!   and consistent with its `data_flow.egress` (`docs/context-reuse.md` §3):
//!   no off-machine scope alongside `egress: false`, and custom scopes
//!   namespaced.
//! - **frame-validity** — queried frames pass `contextgraph-types` validation: score
//!   in `[0, 1]`, a non-empty title, a non-empty `citation_label` (SPEC.md §6 —
//!   "NEVER a bare uuid").
//! - **verify-honesty** — a provider advertising `verify` answers `valid` for
//!   frames it just served and `stale` when their digests are mutated
//!   (`docs/context-reuse.md` §4). Skipped when `verify` is not advertised —
//!   that is the declared fallback, not a failure.
//! - **budget-honesty** — returned frames' summed `token_cost` never exceeds
//!   the query budget, every declared cost is the canonical count, and the
//!   frame count respects `max_frames` (SPEC.md §7 — "never lies about cost").
//! - **as-of-temporal** — a query pinned with `as_of` gets back no frame whose
//!   `valid_from` is after the pin, i.e. no content that was not yet true at
//!   the pinned instant (SPEC.md §6.1). SHOULD-strength and one-sided: a
//!   provider that returns fewer frames, or none, never fails it.
//! - **shutdown-clean** — the provider tears down without error (SPEC.md §3).
//! - **malformed-input-tolerance** — a garbage line is ignored-or-errored,
//!   never crashing the host (SPEC.md §10, task deliverable). Wire-level, so it
//!   applies to stdio providers.
//! - **embedding-fingerprint** — a provider declaring an
//!   `embeddings_fingerprint` rejects a query embedding whose length
//!   contradicts its declared dimension with `bad_request` (SPEC.md §E1). A
//!   SHOULD, gated on the provider declaring a fingerprint; wire-level, so like
//!   the malformed probe it applies to stdio providers.
//!
//! The suite is deliberately adversarial: pointed at a provider that lies
//! about costs, emits an out-of-range score, omits a citation label, or dies
//! mid-query, the matching check fails loudly. The bundled `contextgraph-example-docs`
//! fixture has `--misbehave` flags that trip each one, proving the suite
//! catches a broken provider (task deliverable).
//!
//! The **host** side of the protocol has binding rules too, which the
//! provider-facing checks above cannot exercise. Those live in
//! [`host_conformance`], the dual suite: [`run_host_conformance`] drives the
//! reference [`Host`] against adversarial in-process providers and asserts it
//! upholds them (`SPEC.md` §11.1; issue #14).

use contextgraph_host::{
    ConsentRecord, ContextProvider, DropReason, Host, HostError, RawStdioConnection,
};
use contextgraph_types::capability::fingerprint_dimensions;
use contextgraph_types::{
    Capabilities, ConsentReceipt, ContextQuery, ContextQueryResult, ErrorCode, FrameId, Grantor,
    ProviderInfo,
};

pub mod host_conformance;
mod report;

pub use host_conformance::{
    HCHECK_BUDGET_DROP, HCHECK_CONSENT_GATE, HCHECK_CONTENT_QUOTING, HCHECK_FRAME_LIMIT,
    HCHECK_PROVENANCE_BYTES, HCHECK_SCOPE_RECEIPT, run_host_conformance,
};
pub use report::{CheckResult, CheckStatus, ConformanceReport};

/// The stable check names, so reports and callers agree on identifiers.
pub const CHECK_HANDSHAKE: &str = "handshake";
pub const CHECK_CONSENT_SCOPE: &str = "consent-scope";
pub const CHECK_FRAME_VALIDITY: &str = "frame-validity";
pub const CHECK_VERIFY_HONESTY: &str = "verify-honesty";
pub const CHECK_BUDGET_HONESTY: &str = "budget-honesty";
pub const CHECK_AS_OF: &str = "as-of-temporal";
pub const CHECK_SHUTDOWN: &str = "shutdown-clean";
pub const CHECK_MALFORMED: &str = "malformed-input-tolerance";
pub const CHECK_EMBEDDING_FINGERPRINT: &str = "embedding-fingerprint";

/// How to reach the provider under test. `contextgraph-inspect` builds one of these
/// from its CLI arguments; tests build them directly.
pub enum ProviderTarget {
    /// A child-process provider: `program` plus `args`.
    Stdio { program: String, args: Vec<String> },
    /// A remote provider at `url`.
    Http { url: String },
    /// An already-constructed in-process provider (e.g. a built-in).
    InProcess(Box<dyn ContextProvider>),
}

impl ProviderTarget {
    /// A one-line human description of the target, for the report header.
    pub fn describe(&self) -> String {
        match self {
            ProviderTarget::Stdio { program, args } => {
                if args.is_empty() {
                    format!("stdio: {program}")
                } else {
                    format!("stdio: {program} {}", args.join(" "))
                }
            }
            ProviderTarget::Http { url } => format!("http: {url}"),
            ProviderTarget::InProcess(provider) => format!("in-process: {}", provider.id()),
        }
    }
}

/// Run the full conformance suite against a provider, returning a typed
/// report. Never panics: every failure mode becomes a failing check with
/// evidence.
pub async fn run_conformance(target: ProviderTarget) -> ConformanceReport {
    let description = target.describe();

    // Capture stdio spawn info before `target` is consumed — the malformed
    // probe needs a second, independent connection to the same program.
    let stdio_probe = match &target {
        ProviderTarget::Stdio { program, args } => Some((program.clone(), args.clone())),
        _ => None,
    };

    let mut checks = Vec::new();

    match build_host(target).await {
        Ok((host, id, info, caps)) => {
            if info.name.trim().is_empty() || info.version.trim().is_empty() {
                checks.push(CheckResult::fail(
                    CHECK_HANDSHAKE,
                    format!(
                        "provider identity incomplete: name='{}' version='{}'",
                        info.name, info.version
                    ),
                ));
            } else {
                checks.push(CheckResult::pass(
                    CHECK_HANDSHAKE,
                    describe_handshake(&info, &caps),
                ));
            }
            checks.push(check_consent_scopes(&info));
            run_query_and_shutdown_checks(host, &id, &caps, &mut checks).await;
        }
        Err(error) => {
            checks.push(CheckResult::fail(
                CHECK_HANDSHAKE,
                format!("could not establish provider: {error}"),
            ));
            for name in [
                CHECK_FRAME_VALIDITY,
                CHECK_VERIFY_HONESTY,
                CHECK_CONSENT_SCOPE,
                CHECK_BUDGET_HONESTY,
                CHECK_AS_OF,
                CHECK_SHUTDOWN,
            ] {
                checks.push(CheckResult::skip(name, "handshake failed"));
            }
        }
    }

    match stdio_probe {
        Some((program, args)) => {
            checks.push(malformed_stdio_probe(&program, &args).await);
            checks.push(embedding_fingerprint_stdio_probe(&program, &args).await);
        }
        None => {
            checks.push(CheckResult::skip(
                CHECK_MALFORMED,
                "wire-level malformed-input probe applies to stdio providers only",
            ));
            checks.push(CheckResult::skip(
                CHECK_EMBEDDING_FINGERPRINT,
                "wire-level §E1 bad_request probe applies to stdio providers only",
            ));
        }
    }

    ConformanceReport {
        target: description,
        checks,
    }
}

/// Stand up a one-provider host for the target and read back the provider's
/// negotiated identity + capabilities. Records consent for an egress
/// provider under test — running the suite *is* the consent to its declared
/// flow, so it isn't spuriously gated.
async fn build_host(
    target: ProviderTarget,
) -> Result<(Host, String, ProviderInfo, Capabilities), HostError> {
    let mut host = Host::new();
    let (id, info, caps) = match target {
        ProviderTarget::Stdio { program, args } => {
            let id = "provider-under-test".to_string();
            host.add_stdio(id.clone(), &program, &args).await?;
            capture_identity(&host, &id)?
        }
        ProviderTarget::Http { url } => {
            let id = "provider-under-test".to_string();
            host.add_http(id.clone(), url).await?;
            capture_identity(&host, &id)?
        }
        ProviderTarget::InProcess(provider) => {
            let id = provider.id().to_string();
            let info = provider.info().clone();
            let caps = provider.capabilities().clone();
            host.register(provider);
            (id, info, caps)
        }
    };

    // Running the suite *is* consent to the provider's declared flow, so it
    // isn't spuriously gated. Record the legacy boolean consent, and a receipt
    // for every off-machine egress scope the provider declares (the scope gate).
    if info.data_flow.egress {
        host.record_consent(ConsentRecord::new(
            id.clone(),
            info.data_flow.clone(),
            "conformance run under test",
        ));
    }
    for scope in info.data_flow.off_machine_scopes() {
        host.record_receipt(ConsentReceipt::new(
            id.clone(),
            &info,
            scope.clone(),
            Grantor::Policy("conformance-suite".into()),
            "2026-07-21T00:00:00Z",
        ));
    }

    Ok((host, id, info, caps))
}

fn capture_identity(
    host: &Host,
    id: &str,
) -> Result<(String, ProviderInfo, Capabilities), HostError> {
    let provider = host
        .provider(id)
        .ok_or_else(|| HostError::UnknownProvider(id.to_string()))?;
    Ok((
        id.to_string(),
        provider.info().clone(),
        provider.capabilities().clone(),
    ))
}

async fn run_query_and_shutdown_checks(
    host: Host,
    id: &str,
    caps: &Capabilities,
    checks: &mut Vec<CheckResult>,
) {
    let query = sample_query();
    match host.query_provider(id, &query).await {
        Ok(result) => {
            let (ok, evidence) = check_frames(&result);
            checks.push(CheckResult::from_bool(CHECK_FRAME_VALIDITY, ok, evidence));
            checks.push(check_verify_honesty(&host, id, caps, &result).await);

            let (budget_ok, budget_evidence) = check_budget(&result, &query);
            checks.push(CheckResult::from_bool(
                CHECK_BUDGET_HONESTY,
                budget_ok,
                budget_evidence,
            ));
        }
        Err(error) => {
            let evidence = format!("query failed: {error}");
            checks.push(CheckResult::fail(CHECK_FRAME_VALIDITY, evidence.clone()));
            checks.push(CheckResult::fail(CHECK_VERIFY_HONESTY, evidence.clone()));
            checks.push(CheckResult::fail(CHECK_BUDGET_HONESTY, evidence));
        }
    }

    // The temporal probe fires its own `as_of`-pinned query, so it stands on
    // its own regardless of how the unpinned query above fared.
    checks.push(check_as_of(&host, id).await);

    let results = host.shutdown().await;
    match results.iter().find(|(pid, _)| pid == id) {
        Some((_, Ok(()))) => checks.push(CheckResult::pass(
            CHECK_SHUTDOWN,
            "provider acknowledged shutdown and tore down cleanly",
        )),
        Some((_, Err(error))) => checks.push(CheckResult::fail(
            CHECK_SHUTDOWN,
            format!("shutdown error: {error}"),
        )),
        None => checks.push(CheckResult::fail(
            CHECK_SHUTDOWN,
            "provider vanished before shutdown could be attempted",
        )),
    }
}

/// Suffix appended to a real digest to simulate a mutated source. Derived from
/// the provider's own digest, so it is guaranteed to differ from it while
/// staying vanishingly unlikely to collide with any digest the provider
/// actually serves.
const MUTATED_SUFFIX: &str = "-contextgraph-conformance-mutated";

/// Probe `context/verify` honesty (`docs/context-reuse.md` §4, requirement V1).
///
/// A provider's digest is opaque and provider-declared, so only the provider
/// can say whether an identity still names its current bytes — which means the
/// suite cannot check the *answer*, only that the provider **distinguishes**.
/// So it asks twice about frames the provider just served:
///
/// 1. with the **real** digests it returned — an honest provider says `valid`;
/// 2. with those digests **mutated** — from the provider's side this is
///    indistinguishable from a source that changed underneath the host, and an
///    honest provider says `stale`.
///
/// A provider that rubber-stamps everything `valid` fails the second ask; one
/// that advertises `verify` but can never vouch for anything fails the first.
/// Both are caught without the suite needing to mutate a real source.
///
/// Skipped — not failed — when the provider does not advertise `verify`: that
/// is the declared capability-gated fallback (V3), and the host re-queries
/// instead.
async fn check_verify_honesty(
    host: &Host,
    id: &str,
    caps: &Capabilities,
    result: &ContextQueryResult,
) -> CheckResult {
    if !caps.verify {
        return CheckResult::skip(
            CHECK_VERIFY_HONESTY,
            "provider does not advertise `verify`; a host falls back to re-querying its frames (§4)",
        );
    }

    let held: Vec<FrameId> = result
        .frames
        .iter()
        .filter(|frame| frame.content_digest.is_some())
        .map(|frame| FrameId::new(id, frame.id.clone(), frame.content_digest.clone()))
        .collect();
    if held.is_empty() {
        return CheckResult::skip(
            CHECK_VERIFY_HONESTY,
            "provider served no frame carrying a `content_digest`, so nothing is verifiable (§1 D4)",
        );
    }

    // Ask 1: the real digests. Every frame the provider just served must
    // verify valid — otherwise it cannot vouch for its own output.
    let unchanged = host.verify_frames(&held).await;
    if !unchanged.dropped.is_empty() {
        let detail: Vec<String> = unchanged
            .dropped
            .iter()
            .map(|dropped| format!("{} => {:?}", dropped.frame.frame_id, dropped.reason))
            .collect();
        return CheckResult::fail(
            CHECK_VERIFY_HONESTY,
            format!(
                "provider advertises `verify` but did not answer `valid` for {} of {} frame(s) it had just served with unchanged digests: {}",
                unchanged.dropped.len(),
                held.len(),
                detail.join(", ")
            ),
        );
    }

    // Ask 2: the same frames with mutated digests — what a changed source
    // looks like from the provider's side.
    let mutated: Vec<FrameId> = held
        .iter()
        .map(|frame| {
            FrameId::new(
                id,
                frame.frame_id.clone(),
                frame
                    .content_digest
                    .as_ref()
                    .map(|digest| format!("{digest}{MUTATED_SUFFIX}")),
            )
        })
        .collect();
    let changed = host.verify_frames(&mutated).await;

    if !changed.retained.is_empty() {
        return CheckResult::fail(
            CHECK_VERIFY_HONESTY,
            format!(
                "provider answered `valid` for {} frame(s) whose content digest it never served — a rubber stamp that lets a host cite stale evidence",
                changed.retained.len()
            ),
        );
    }
    let not_stale: Vec<String> = changed
        .dropped
        .iter()
        .filter(|dropped| !matches!(dropped.reason, DropReason::Stale { .. }))
        .map(|dropped| format!("{} => {:?}", dropped.frame.frame_id, dropped.reason))
        .collect();
    if !not_stale.is_empty() {
        return CheckResult::fail(
            CHECK_VERIFY_HONESTY,
            format!(
                "a digest mismatch on a frame the provider still serves MUST verify `stale` (§4 V1); got: {}",
                not_stale.join(", ")
            ),
        );
    }

    CheckResult::pass(
        CHECK_VERIFY_HONESTY,
        format!(
            "provider verified {n} unchanged frame(s) `valid` and all {n} mutated digest(s) `stale`, carrying no frame bodies",
            n = held.len()
        ),
    )
}

/// Wire-level probe: complete the handshake on a fresh connection, inject a
/// malformed line, then send a valid query. A conforming provider ignores or
/// cleanly errors on the garbage and stays alive to answer the query; a
/// provider that dies on one bad line fails (SPEC.md §10).
async fn malformed_stdio_probe(program: &str, args: &[String]) -> CheckResult {
    let mut conn = match RawStdioConnection::spawn(program, args).await {
        Ok(conn) => conn,
        Err(error) => {
            return CheckResult::fail(
                CHECK_MALFORMED,
                format!("could not spawn provider: {error}"),
            );
        }
    };
    if let Err(error) = conn.handshake().await {
        return CheckResult::fail(
            CHECK_MALFORMED,
            format!("handshake failed before the probe could run: {error}"),
        );
    }
    if let Err(error) = conn.send_raw_line("this is not valid json {{{\n").await {
        return CheckResult::fail(
            CHECK_MALFORMED,
            format!("provider closed its input on a malformed line: {error}"),
        );
    }
    if let Err(error) = conn
        .send(&contextgraph_host::Envelope::Query {
            id: None,
            query: sample_query(),
        })
        .await
    {
        return CheckResult::fail(
            CHECK_MALFORMED,
            format!("provider died after a malformed line (before a valid query): {error}"),
        );
    }
    match conn.recv().await {
        Ok(contextgraph_host::Envelope::Frames { .. }) => CheckResult::pass(
            CHECK_MALFORMED,
            "provider ignored a malformed line and still answered a valid query",
        ),
        Ok(contextgraph_host::Envelope::Error { message, .. }) => CheckResult::pass(
            CHECK_MALFORMED,
            format!("provider errored cleanly on malformed input and stayed alive: {message}"),
        ),
        Ok(other) => CheckResult::fail(
            CHECK_MALFORMED,
            format!(
                "provider replied to a valid query with an unexpected `{}` envelope",
                contextgraph_host::envelope_kind(&other)
            ),
        ),
        Err(HostError::ProviderCrashed { .. }) => CheckResult::fail(
            CHECK_MALFORMED,
            "provider crashed on a malformed line — it must error-or-ignore, not die",
        ),
        Err(error) => CheckResult::fail(
            CHECK_MALFORMED,
            format!("provider mishandled malformed input: {error}"),
        ),
    }
}

/// Wire-level probe for §E1: a provider that declares an
/// `embeddings_fingerprint` **SHOULD** reject a query embedding whose length
/// contradicts that fingerprint's dimension with `bad_request`, rather than
/// scoring a vector from a different space into plausible-looking, meaningless
/// similarity.
///
/// Driven on the raw wire like [`malformed_stdio_probe`], because the honest
/// reply is an `error` envelope carrying a *code* — and the host's query path
/// collapses that to a bare message, losing the `bad_request` §E1 names. Reading
/// the code directly is what makes this checkable, and is why the probe is
/// stdio-only (skipped for in-process/HTTP targets — a documented limitation).
///
/// Gated on the provider declaring a fingerprint: one that declares none has no
/// dimension to contradict and is skipped, exactly as a provider that does not
/// advertise `verify` skips `verify-honesty`.
async fn embedding_fingerprint_stdio_probe(program: &str, args: &[String]) -> CheckResult {
    let mut conn = match RawStdioConnection::spawn(program, args).await {
        Ok(conn) => conn,
        Err(error) => {
            return CheckResult::fail(
                CHECK_EMBEDDING_FINGERPRINT,
                format!("could not spawn provider: {error}"),
            );
        }
    };
    let caps = match conn.handshake().await {
        Ok((_, caps)) => caps,
        Err(error) => {
            return CheckResult::skip(
                CHECK_EMBEDDING_FINGERPRINT,
                format!("handshake failed before the §E1 probe could run: {error}"),
            );
        }
    };
    let Some(fingerprint) = caps.embeddings_fingerprint.clone() else {
        return CheckResult::skip(
            CHECK_EMBEDDING_FINGERPRINT,
            "provider declares no embeddings_fingerprint, so §E1 has no dimension to contradict",
        );
    };
    let Some(dimension) = fingerprint_dimensions(&fingerprint) else {
        return CheckResult::skip(
            CHECK_EMBEDDING_FINGERPRINT,
            format!(
                "fingerprint `{fingerprint}` declares no parseable dimension, so §E1 cannot be probed"
            ),
        );
    };

    // A length guaranteed to differ from the declared dimension — the
    // wrong-space vector §E1 says to reject.
    let wrong_len = if dimension == 1 { 2 } else { 1 };
    let mut query = sample_query();
    query.embedding = Some(vec![0.0; wrong_len]);
    if let Err(error) = conn
        .send(&contextgraph_host::Envelope::Query { id: None, query })
        .await
    {
        return CheckResult::fail(
            CHECK_EMBEDDING_FINGERPRINT,
            format!("provider closed its input before the §E1 probe query: {error}"),
        );
    }
    match conn.recv().await {
        // The recommended reply: it named the request wrong with the code §E1
        // specifies.
        Ok(contextgraph_host::Envelope::Error {
            code: Some(ErrorCode::BadRequest),
            ..
        }) => CheckResult::pass(
            CHECK_EMBEDDING_FINGERPRINT,
            format!(
                "provider declares {fingerprint} ({dimension}-dim) and rejected a {wrong_len}-dim embedding with `bad_request` (§E1)"
            ),
        ),
        // Refused, but not with the code §E1 recommends. Refusing at all is the
        // load-bearing half of a SHOULD, so this passes with a note.
        Ok(contextgraph_host::Envelope::Error { code, message, .. }) => CheckResult::pass(
            CHECK_EMBEDDING_FINGERPRINT,
            format!(
                "provider rejected a {wrong_len}-dim embedding against {fingerprint} with `{}` rather than the `bad_request` §E1 recommends: {message}",
                code.unwrap_or(ErrorCode::Internal)
            ),
        ),
        // The violation: it *scored* a vector from a different space.
        Ok(contextgraph_host::Envelope::Frames { .. }) => CheckResult::fail(
            CHECK_EMBEDDING_FINGERPRINT,
            format!(
                "provider declares {fingerprint} ({dimension}-dim) but scored a {wrong_len}-dim embedding into frames instead of rejecting it — meaningless similarity from a different vector space (§E1)"
            ),
        ),
        Ok(other) => CheckResult::fail(
            CHECK_EMBEDDING_FINGERPRINT,
            format!(
                "provider answered the §E1 probe with an unexpected `{}` envelope",
                contextgraph_host::envelope_kind(&other)
            ),
        ),
        Err(HostError::ProviderCrashed { .. }) => CheckResult::fail(
            CHECK_EMBEDDING_FINGERPRINT,
            "provider crashed on a dimension-mismatched embedding — §E1 asks it to reply `bad_request`, not die",
        ),
        Err(error) => CheckResult::fail(
            CHECK_EMBEDDING_FINGERPRINT,
            format!("provider mishandled the §E1 probe: {error}"),
        ),
    }
}

/// The instant the `as_of` probe pins retrieval to (`SPEC.md` §6.1). Chosen to
/// fall *between* the reference fixture's two frame validity windows, so an
/// honest provider's pinned answer is observably narrower than its unpinned one.
const AS_OF_PIN: &str = "2026-07-01T00:00:00Z";

/// Probe `as_of` temporal pinning (`SPEC.md` §6.1, §F4). `as_of` pins retrieval
/// to an instant; a frame whose `valid_from` is strictly after the pin is
/// content that was not yet true then — exactly what the pin exists to keep out
/// of the answer.
///
/// SHOULD-strength and deliberately one-sided: it never penalizes a provider for
/// returning *fewer* frames (or none) under a pin, because implementing
/// time-travel retrieval is optional. It fails only on a frame the provider
/// *did* return whose `valid_from` provably postdates the pin — a temporal lie
/// no matter how sophisticated the provider's time handling. Comparison is
/// lexicographic on the UTC strings, which is chronological because the
/// timestamp profile admits one spelling per instant (§6.1). A provider serving
/// no timestamped content trivially passes.
async fn check_as_of(host: &Host, id: &str) -> CheckResult {
    match host.query_provider(id, &as_of_query()).await {
        Ok(result) => {
            let not_yet_valid: Vec<String> = result
                .frames
                .iter()
                .filter_map(|frame| {
                    frame
                        .valid_from
                        .as_deref()
                        .filter(|valid_from| *valid_from > AS_OF_PIN)
                        .map(|valid_from| format!("{} (valid_from={valid_from})", frame.id))
                })
                .collect();
            if not_yet_valid.is_empty() {
                CheckResult::pass(
                    CHECK_AS_OF,
                    format!(
                        "as_of={AS_OF_PIN}: none of the {} returned frame(s) is dated after the pin",
                        result.frames.len()
                    ),
                )
            } else {
                CheckResult::fail(
                    CHECK_AS_OF,
                    format!(
                        "provider returned {} frame(s) whose valid_from is after as_of={AS_OF_PIN} — content that was not yet true at the pinned instant (§6.1): {}",
                        not_yet_valid.len(),
                        not_yet_valid.join(", ")
                    ),
                )
            }
        }
        Err(error) => CheckResult::fail(CHECK_AS_OF, format!("as_of query failed: {error}")),
    }
}

/// The [`sample_query`] pinned to [`AS_OF_PIN`] — the query the temporal probe
/// fires. Everything else is held equal so only the pin varies.
fn as_of_query() -> ContextQuery {
    ContextQuery {
        as_of: Some(AS_OF_PIN.into()),
        ..sample_query()
    }
}

/// The query the suite probes every provider with — no `kinds` filter, so any
/// provider is asked for its best frames (SPEC.md §5).
pub fn sample_query() -> ContextQuery {
    ContextQuery {
        goal: "conformance probe: return your most relevant frames".into(),
        query_text: Some("conformance probe".into()),
        embedding: None,
        kinds: vec![],
        anchors: vec![],
        max_frames: 8,
        max_tokens: 4096,
        as_of: None,
        representation_preferences: vec![],
    }
}

/// Validate a query result's frames against the `ContextFrame` contract
/// (SPEC.md §6). Returns `(passed, evidence)`. Zero frames is permitted — a
/// provider may simply have nothing relevant.
pub fn check_frames(result: &ContextQueryResult) -> (bool, String) {
    if result.frames.is_empty() {
        return (
            true,
            "provider returned 0 frames (permitted — nothing relevant to the probe)".into(),
        );
    }

    let mut problems = Vec::new();
    for (i, frame) in result.frames.iter().enumerate() {
        if !frame.has_valid_score() {
            problems.push(format!("frame[{i}] score {} is outside [0,1]", frame.score));
        }
        if frame.title.trim().is_empty() {
            problems.push(format!("frame[{i}] has an empty title"));
        }
        match &frame.citation_label {
            Some(label) if !label.trim().is_empty() => {}
            _ => problems.push(format!(
                "frame[{i}] is missing a citation_label (§F3 — never a bare id)"
            )),
        }
        // §P1–P3: a frame must not lie about how it carries its content — a
        // `reference` carrying inline content, a `compact` missing its
        // canonical hash. `representation_invariants` names the exact breach.
        // The predicate shipped in PR #42 with no caller; this is the caller.
        if let Err(violation) = frame.representation_invariants() {
            problems.push(format!("frame[{i}] {violation} (§P1–P3)"));
        }
        // §F4: temporal fields must be in the protocol's timestamp profile.
        // Naming the offending field is what makes this actionable — before
        // this check, `"valid_from": "last tuesday"` was fully conformant and
        // the bi-temporal guarantee was unfalsifiable.
        for field in frame.invalid_temporal_fields() {
            problems.push(format!(
                "frame[{i}] field `{field}` is not an RFC 3339 UTC timestamp (§F4)"
            ));
        }
        // §F5: file provenance must carry a well-formed digest, since that is
        // the only provenance a host can independently re-read and verify.
        for index in frame.provenance_with_unusable_digests() {
            problems.push(format!(
                "frame[{i}] provenance[{index}] addresses a file but its digest is missing or not `sha256:<64 lowercase hex>` (§F5)"
            ));
        }
        // §G1: a graph edge must be citable by a human label.
        for (edge_index, edge) in frame.relations.iter().enumerate() {
            if !edge.has_display_name() {
                problems.push(format!(
                    "frame[{i}] relation[{edge_index}] `{}` has no display_name (§G1 — an edge is surfaced by label, never a raw id)",
                    edge.rel
                ));
            }
        }
    }

    if problems.is_empty() {
        (
            true,
            format!(
                "{} frame(s) — scores in [0,1], titles, citation labels, honest representations, RFC 3339 timestamps, well-formed digests, labelled relations",
                result.frames.len()
            ),
        )
    } else {
        (false, problems.join("; "))
    }
}

/// Validate a query result against the budget contract (`SPEC.md` §B1, §B3,
/// §B4). Returns `(passed, evidence)`.
///
/// Three distinct promises, deliberately checked separately so a failure says
/// which one broke:
///
/// - **§B1** the declared costs sum within `max_tokens`;
/// - **§B3** each declared cost equals the canonical count for its content —
///   this is what turned the check from arithmetic into truth;
/// - **§B4** the frame count respects `max_frames`.
pub fn check_budget(result: &ContextQueryResult, query: &ContextQuery) -> (bool, String) {
    let mut problems = Vec::new();

    let declared = result.total_token_cost();
    if declared > query.max_tokens as u64 {
        problems.push(format!(
            "declared cost {declared} exceeds the query budget of {} (§B1)",
            query.max_tokens
        ));
    }

    let dishonest = result.frames_with_dishonest_cost();
    if !dishonest.is_empty() {
        let canonical = result.canonical_token_cost();
        problems.push(format!(
            "{} frame(s) misdeclare token_cost — {} (§B3); declared total {declared}, canonical total {canonical}",
            dishonest.len(),
            dishonest.join(", ")
        ));
    }

    if !result.respects_frame_limit(query.max_frames) {
        problems.push(format!(
            "returned {} frames against max_frames={} (§B4)",
            result.frames.len(),
            query.max_frames
        ));
    }

    if problems.is_empty() {
        (
            true,
            format!(
                "{} frame(s), {declared} tokens within the {} budget; every declared cost matches its canonical count",
                result.frames.len(),
                query.max_tokens
            ),
        )
    } else {
        (false, problems.join("; "))
    }
}

/// Validate a provider's declared egress scopes against its `data_flow`
/// (`docs/context-reuse.md` §3, requirement C5): every scope must be well-formed
/// (custom scopes namespaced), and no off-machine scope may be declared with
/// `egress: false`. A scope-lying provider — one claiming local posture while
/// naming a destination that leaves — fails here.
fn check_consent_scopes(info: &ProviderInfo) -> CheckResult {
    if info.data_flow.scopes_consistent() {
        let scopes: Vec<&str> = info
            .data_flow
            .egress_scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect();
        CheckResult::pass(
            CHECK_CONSENT_SCOPE,
            format!(
                "declared egress scopes {scopes:?} are well-formed and consistent with egress={}",
                info.data_flow.egress
            ),
        )
    } else {
        CheckResult::fail(
            CHECK_CONSENT_SCOPE,
            format!(
                "egress scopes {:?} are inconsistent with egress={}: an off-machine scope alongside egress=false, or a non-namespaced custom scope (§3, C5)",
                info.data_flow
                    .egress_scopes
                    .iter()
                    .map(|scope| scope.as_str())
                    .collect::<Vec<_>>(),
                info.data_flow.egress
            ),
        )
    }
}

fn describe_handshake(info: &ProviderInfo, caps: &Capabilities) -> String {
    format!(
        "provider '{}' v{} — data-flow reads={} writes={} egress={}; query kinds={:?}, graph={}",
        info.name,
        info.version,
        info.data_flow.reads,
        info.data_flow.writes,
        info.data_flow.egress,
        caps.query.kinds,
        caps.graph,
    )
}
