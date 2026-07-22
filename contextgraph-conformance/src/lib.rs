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
//! - **budget-honesty** — returned frames' summed `token_cost` never exceeds
//!   the query budget (SPEC.md §5 — "never lies about cost").
//! - **shutdown-clean** — the provider tears down without error (SPEC.md §3).
//! - **malformed-input-tolerance** — a garbage line is ignored-or-errored,
//!   never crashing the host (SPEC.md §10, task deliverable). Wire-level, so it
//!   applies to stdio providers.
//!
//! The suite is deliberately adversarial: pointed at a provider that lies
//! about costs, emits an out-of-range score, omits a citation label, or dies
//! mid-query, the matching check fails loudly. The bundled `contextgraph-example-docs`
//! fixture has `--misbehave` flags that trip each one, proving the suite
//! catches a broken provider (task deliverable).

use contextgraph_host::{ConsentRecord, ContextProvider, Host, HostError, RawStdioConnection};
use contextgraph_types::{
    Capabilities, ConsentReceipt, ContextQuery, ContextQueryResult, Grantor, ProviderInfo,
};

mod report;

pub use report::{CheckResult, CheckStatus, ConformanceReport};

/// The stable check names, so reports and callers agree on identifiers.
pub const CHECK_HANDSHAKE: &str = "handshake";
pub const CHECK_CONSENT_SCOPE: &str = "consent-scope";
pub const CHECK_FRAME_VALIDITY: &str = "frame-validity";
pub const CHECK_BUDGET_HONESTY: &str = "budget-honesty";
pub const CHECK_SHUTDOWN: &str = "shutdown-clean";
pub const CHECK_MALFORMED: &str = "malformed-input-tolerance";

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
            run_query_and_shutdown_checks(host, &id, &mut checks).await;
        }
        Err(error) => {
            checks.push(CheckResult::fail(
                CHECK_HANDSHAKE,
                format!("could not establish provider: {error}"),
            ));
            for name in [
                CHECK_CONSENT_SCOPE,
                CHECK_FRAME_VALIDITY,
                CHECK_BUDGET_HONESTY,
                CHECK_SHUTDOWN,
            ] {
                checks.push(CheckResult::skip(name, "handshake failed"));
            }
        }
    }

    match stdio_probe {
        Some((program, args)) => checks.push(malformed_stdio_probe(&program, &args).await),
        None => checks.push(CheckResult::skip(
            CHECK_MALFORMED,
            "wire-level malformed-input probe applies to stdio providers only",
        )),
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

async fn run_query_and_shutdown_checks(host: Host, id: &str, checks: &mut Vec<CheckResult>) {
    let query = sample_query();
    match host.query_provider(id, &query).await {
        Ok(result) => {
            let (ok, evidence) = check_frames(&result);
            checks.push(CheckResult::from_bool(CHECK_FRAME_VALIDITY, ok, evidence));

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
            checks.push(CheckResult::fail(CHECK_BUDGET_HONESTY, evidence));
        }
    }

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
                "{} frame(s) — scores in [0,1], titles, citation labels, RFC 3339 timestamps, well-formed digests, labelled relations",
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
