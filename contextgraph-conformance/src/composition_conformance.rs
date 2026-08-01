//! Composition conformance (`SPEC.md` §11.1) — the suite a **downstream** host
//! can run against its own composition layer.
//!
//! [`run_host_conformance`](crate::run_host_conformance) drives
//! [`contextgraph_host::Host`] itself, so it certifies the reference host and
//! nothing else. That leaves a real gap, because `Host::query_all` is not the
//! whole host: it audits budget honesty **per provider**, and then hands back a
//! fan-out. Something above it has to turn N providers' accepted frames into the
//! one frame set that reaches a prompt, and that step is where a host makes its
//! own decisions — which frames win a shared budget, what happens to the losers,
//! and in what order the survivors render.
//!
//! That step is not covered by the per-provider audit, and the gap is not
//! theoretical. Three providers each returning one honest 400-token frame against
//! a 1000-token query are *individually* conformant — no `token_cost` lie, no
//! frame flood — and `FanOut::accepted_frames()` yields all three, for 1200
//! tokens. Whether the prompt ends up over budget, and whether anyone is told
//! which evidence was dropped to keep it under, is entirely up to the composing
//! host. A downstream host that got this wrong would pass every check in the
//! provider suite and every check in the host suite.
//!
//! So this module inverts the dependency: instead of driving a fixed host, it
//! takes a [`ComposingHost`] — anything that can answer "given these providers
//! and this query, what reaches the prompt, and what did you drop getting
//! there?" — and holds it to the rules that bind that answer. The reference
//! implementation is [`compose_for_prompt`](contextgraph_host::compose_for_prompt),
//! which passes; a downstream host with its own merge (stella's `recall_via_host`
//! is the known one) implements the trait and gets the same audit.
//!
//! # The rules checked
//!
//! - **[`CCHECK_BUDGET_BOUND`]** — the admitted set's summed token cost does not
//!   exceed the query's `max_tokens`, *including* when every individual provider
//!   was honest and only the sum overflows (§7).
//! - **[`CCHECK_TOTAL_PARTITION`]** — every frame the host was offered is either
//!   admitted or reported as dropped. A frame that is neither has been *silently
//!   truncated*, which is the one outcome an evidence audit cannot tolerate
//!   (issue #15's total-partition requirement).
//! - **[`CCHECK_QUARANTINE`]** — frames from a provider the host's own audit
//!   rejected never reach the prompt. A composing host that reads raw provider
//!   results instead of `accepted_frames()` re-admits exactly what B2/B4 dropped.
//! - **[`CCHECK_DETERMINISM`]** — the same frame set composes to the same
//!   admitted sequence twice running. This is the prompt-cache guarantee
//!   (`docs/context-reuse.md` §1): a turn whose underlying frames did not change
//!   must emit byte-identical text, so selection may depend on score but
//!   *rendering* must not.
//!
//! Every check is **adversarial by construction**, the same discipline
//! [`host_conformance`](crate::host_conformance) uses: each one points the host at
//! input that tries to make it fail *and* at a well-behaved counterpart it must
//! accept, so a check can only pass if the host **discriminates**. A host that
//! admitted nothing at all, or reported every frame as dropped, would fail its
//! counterpart rather than passing vacuously.
//!
//! # Honest residual
//!
//! This suite sees a host's composition as a black box over frames: it cannot
//! check *rendering* (R3 fencing is [`host_conformance`]'s `host-content-quoting`,
//! against the reference renderer), and it cannot check that a host's stated drop
//! *reason* is the true one — only that a drop is reported at all. A host that
//! reported every over-budget drop as a duplicate would pass. Reason fidelity
//! needs a vocabulary this trait deliberately does not impose, because a
//! downstream host's drop reasons are its own (stella has `FrameCount`,
//! `TokenBudget`, `RequiredOverBudget`; the reference has `Duplicate` and
//! `OverBudget`).

use std::collections::BTreeSet;

use async_trait::async_trait;
use contextgraph_types::{
    BYTES_PER_BUDGET_TOKEN, ContextFrame, ContextQuery, FrameId, FrameKind, budget_tokens,
};

use contextgraph_host::{ContextProvider, Host, ProviderResult, compose_for_prompt};

use crate::host_conformance::{ProbeProvider, probe_query};
use crate::report::{CheckResult, ConformanceReport};

/// §7 — the admitted set fits the query's token budget, including when only the
/// cross-provider sum overflows.
pub const CCHECK_BUDGET_BOUND: &str = "composition-budget-bound";
/// Issue #15 — every offered frame is admitted or reported dropped, never
/// silently truncated.
pub const CCHECK_TOTAL_PARTITION: &str = "composition-total-partition";
/// §7 B2/B4 — frames the host's own audit rejected never reach the prompt.
pub const CCHECK_QUARANTINE: &str = "composition-quarantine";
/// `docs/context-reuse.md` §1 — an unchanged frame set composes identically.
pub const CCHECK_DETERMINISM: &str = "composition-determinism";

/// One frame a composing host declined to admit.
///
/// Deliberately does **not** carry a reason enum. The suite's contract is that a
/// dropped frame is *accounted for*, not that it is accounted for in the
/// protocol's vocabulary — a downstream host's drop reasons are its own product
/// vocabulary (see the module's honest residual). Imposing one here would make
/// the trait unimplementable without a lossy mapping, and a lossy mapping is
/// worse evidence than an honest identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedFrame {
    /// The provider that served it.
    pub provider_id: String,
    /// The provider's own frame id.
    pub frame_id: String,
}

/// What a composing host did with a fan-out: what reaches the prompt, and what
/// it dropped getting there.
#[derive(Debug, Clone, Default)]
pub struct Composition {
    /// The frames that reach the prompt, in the order the host renders them,
    /// each paired with the provider that served it.
    pub admitted: Vec<(String, ContextFrame)>,
    /// Every frame the host was offered and did not admit.
    pub dropped: Vec<ExcludedFrame>,
}

impl Composition {
    /// The summed declared token cost of the admitted frames.
    fn admitted_tokens(&self) -> u64 {
        self.admitted
            .iter()
            .map(|(_, frame)| u64::from(frame.token_cost))
            .sum()
    }

    /// The `(provider, frame id)` pairs accounted for — admitted or dropped.
    fn accounted(&self) -> BTreeSet<(String, String)> {
        self.admitted
            .iter()
            .map(|(provider, frame)| (provider.clone(), frame.id.clone()))
            .chain(
                self.dropped
                    .iter()
                    .map(|drop| (drop.provider_id.clone(), drop.frame_id.clone())),
            )
            .collect()
    }

    /// The admitted frames as identity pairs, in render order — the sequence
    /// [`CCHECK_DETERMINISM`] compares across runs.
    fn render_order(&self) -> Vec<(String, String)> {
        self.admitted
            .iter()
            .map(|(provider, frame)| (provider.clone(), frame.id.clone()))
            .collect()
    }
}

/// A host's composition layer, as this suite needs to see it.
///
/// One method, because one method is the whole contract: a composing host is a
/// function from *(providers, query)* to *what reached the prompt*. Taking the
/// providers rather than a pre-built fan-out is deliberate — it lets the suite
/// hand over adversarial providers and still exercise the host's **own** fan-out
/// and audit path, which is where [`CCHECK_QUARANTINE`] lives. A trait that took
/// an already-audited frame list could not tell whether the host ran the audit.
#[async_trait]
pub trait ComposingHost: Send + Sync {
    /// Register exactly `providers`, execute `query`, and report the result.
    ///
    /// Implementations should build a fresh host per call: the suite relies on
    /// calls being independent, and reuses ids across checks.
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition;
}

/// Run every composition check against `host`, returning a typed
/// [`ConformanceReport`].
///
/// `target` names the implementation under test and appears in the report, so a
/// downstream host's CI output says which host was certified rather than just
/// "passed".
pub async fn run_composition_conformance(
    host: &dyn ComposingHost,
    target: impl Into<String>,
) -> ConformanceReport {
    let checks = vec![
        check_budget_bound(host).await,
        check_total_partition(host).await,
        check_quarantine(host).await,
        check_determinism(host).await,
    ];
    ConformanceReport {
        target: target.into(),
        checks,
    }
}

/// **§7** — the cross-provider budget bound.
///
/// The adversarial input is the case the per-provider audit structurally cannot
/// catch: three providers, each returning a single honest 400-token frame against
/// `max_tokens: 1000`. Nobody lied — every provider is within budget on its own —
/// and the sum is 1200. A composing host must drop something.
///
/// The well-behaved counterpart is the same shape under the budget (3 × 200 =
/// 600), where the host must admit **all three**. Without it a host that always
/// returned an empty composition would pass.
async fn check_budget_bound(host: &dyn ComposingHost) -> CheckResult {
    let query = probe_query(); // max_tokens: 1000, max_frames: 8
    let over = host
        .compose(three_providers_each_costing(400), &query)
        .await;
    let within_budget = over.admitted_tokens() <= u64::from(query.max_tokens);
    let dropped_something = !over.dropped.is_empty();

    let under = host
        .compose(three_providers_each_costing(200), &query)
        .await;
    let kept_all = under.admitted.len() == 3 && under.dropped.is_empty();

    CheckResult::from_bool(
        CCHECK_BUDGET_BOUND,
        within_budget && dropped_something && kept_all,
        format!(
            "§7 (composition): three individually-honest providers summing 1200 against a \
             1000-token budget compose within budget={within_budget} \
             (admitted {} tokens) and report a drop={dropped_something}; the same shape summing \
             600 keeps all three frames={kept_all}",
            over.admitted_tokens()
        ),
    )
}

/// **Issue #15 total partition** — no frame vanishes unaccounted.
///
/// Uses the same over-budget input as [`check_budget_bound`], because that is the
/// case where a host *must* shed frames and therefore the case where silent
/// truncation is tempting: the naive implementation stops walking the moment the
/// budget fills, and everything after it disappears uncounted as well as unkept.
///
/// The counterpart is the under-budget input, where the host must report *no*
/// drops — otherwise a host could pass by declaring every frame dropped.
async fn check_total_partition(host: &dyn ComposingHost) -> CheckResult {
    let query = probe_query();
    let offered: BTreeSet<(String, String)> = (0..3)
        .map(|i| (format!("p{i}"), format!("p{i}-f")))
        .collect();

    let over = host
        .compose(three_providers_each_costing(400), &query)
        .await;
    let accounted = over.accounted();
    let missing: Vec<_> = offered.difference(&accounted).collect();
    let total = missing.is_empty();
    // A host cannot satisfy the partition by inventing frames it was never
    // offered, either: the accounted set must not exceed the offered one.
    let no_phantoms = accounted.difference(&offered).count() == 0;

    let under = host
        .compose(three_providers_each_costing(200), &query)
        .await;
    let no_spurious_drops = under.dropped.is_empty();

    CheckResult::from_bool(
        CCHECK_TOTAL_PARTITION,
        total && no_phantoms && no_spurious_drops,
        format!(
            "issue #15 (composition): every offered frame is admitted or reported \
             dropped={total} (unaccounted: {missing:?}), no frame is reported that was never \
             offered={no_phantoms}; a composition that drops nothing reports \
             nothing={no_spurious_drops}"
        ),
    )
}

/// **§7 B2/B4** — a provider the audit rejected stays out of the prompt.
///
/// The adversarial provider is a **frame flooder**: `max_frames + 9` frames, each
/// individually cheap. The host's own audit is required to reject the whole set
/// (B4) and does. The composing layer must not put it back.
///
/// A flooder rather than a `token_cost` liar, and the distinction matters. A
/// liar's frames are *also* over the token budget, so a host that skipped the
/// audit entirely would still drop them while packing — and would pass this check
/// by accident, for the wrong reason. (That is not hypothetical; it is what the
/// first version of this check did.) A flooder's frames are cheap: they sail
/// through any token-budget pack, so the **only** thing that keeps them out of
/// the prompt is having consulted the audit. That makes the check load-bearing
/// instead of incidental.
///
/// The counterpart pairs the flooder with an honest provider whose frame **must**
/// still arrive: quarantining one leg may not take the other down with it, which
/// is the crash-isolation posture applied to the budget audit.
async fn check_quarantine(host: &dyn ComposingHost) -> CheckResult {
    let query = probe_query(); // max_frames: 8, max_tokens: 1000
    let flood: Vec<ContextFrame> = (0..query.max_frames + 9)
        .map(|i| honest_frame(&format!("flood-{i}"), 1))
        .collect();
    let providers: Vec<Box<dyn ContextProvider>> = vec![
        Box::new(ProbeProvider::local("flooder", flood)),
        Box::new(ProbeProvider::local(
            "honest",
            vec![honest_frame("honest-f", 100)],
        )),
    ];
    let composed = host.compose(providers, &query).await;

    let flooder_excluded = !composed
        .admitted
        .iter()
        .any(|(provider, _)| provider == "flooder");
    let honest_admitted = composed
        .admitted
        .iter()
        .any(|(provider, frame)| provider == "honest" && frame.id == "honest-f");

    CheckResult::from_bool(
        CCHECK_QUARANTINE,
        flooder_excluded && honest_admitted,
        format!(
            "§7 B2/B4 (composition): the frames of a provider the audit rejected (a frame \
             flooder, whose frames are individually cheap enough to pass any token pack) never \
             reach the prompt={flooder_excluded}; an honest provider queried alongside it still \
             arrives={honest_admitted}"
        ),
    )
}

/// **`docs/context-reuse.md` §1** — an unchanged frame set composes identically.
///
/// Composes the same input twice and compares the admitted sequence. This is the
/// prompt-cache guarantee: selection may depend on score, but *rendering order*
/// must be a function of the frame set alone, so a turn whose underlying frames
/// did not change emits byte-identical text and rides the provider's cache
/// instead of busting it.
///
/// The frames are given deliberately **tied scores** — the reference frame
/// fixture scores every frame 0.5 — because a tie is where an unstable sort or a
/// hash-ordered map leaks nondeterminism. A host that ordered by score alone
/// would pass on distinct scores and fail here, which is the point.
///
/// The counterpart is inverted: rather than a second input the host must accept,
/// the check also asserts the composition is **non-empty**, so a host that
/// admitted nothing cannot pass by being trivially stable.
async fn check_determinism(host: &dyn ComposingHost) -> CheckResult {
    let query = probe_query();
    let first = host
        .compose(three_providers_each_costing(100), &query)
        .await;
    let second = host
        .compose(three_providers_each_costing(100), &query)
        .await;

    let stable = first.render_order() == second.render_order();
    let non_empty = !first.admitted.is_empty();

    CheckResult::from_bool(
        CCHECK_DETERMINISM,
        stable && non_empty,
        format!(
            "context-reuse §1 (composition): an unchanged frame set composes to the same render \
             order twice={stable} (first {:?}, second {:?}); the composition is non-empty, so \
             stability is not vacuous={non_empty}",
            first.render_order(),
            second.render_order()
        ),
    )
}

/// The reference composing host: [`Host::query_all`] for the fan-out and audit,
/// then [`compose_for_prompt`] for the shared-budget pack.
///
/// Two jobs. It is the proof the suite is **satisfiable** — a suite no
/// implementation passes is a suite with a bug, not a bar — and it is the worked
/// example a downstream host implements against, which is why the body is short
/// enough to read: fan out, keep what the audit accepted, pack it, read the
/// partition back off the audit.
///
/// Note what it can and cannot report. Frames the audit quarantined (a
/// `token_cost` liar's whole set) are **absent** from `dropped`, because
/// [`ProviderResult::BudgetLie`](contextgraph_host::ProviderResult) carries a
/// *count* of dropped frames and not their ids — the host knows how many it threw
/// out, not which. That is why [`CCHECK_QUARANTINE`] asserts only that those
/// frames stay out of the prompt, and why [`CCHECK_TOTAL_PARTITION`] is posed
/// over honest providers: demanding that a quarantined frame be named would be
/// demanding information the fan-out does not carry.
pub struct ReferenceComposingHost;

#[async_trait]
impl ComposingHost for ReferenceComposingHost {
    async fn compose(
        &self,
        providers: Vec<Box<dyn ContextProvider>>,
        query: &ContextQuery,
    ) -> Composition {
        let mut host = Host::new();
        for provider in providers {
            host.register(provider);
        }
        let fanout = host.query_all(query).await;

        // Compose from the **audited** accepted set, never from raw provider
        // results — this line is the whole of `CCHECK_QUARANTINE`.
        let offered: Vec<(String, ContextFrame)> = fanout
            .outcomes
            .iter()
            .filter_map(|outcome| match &outcome.result {
                ProviderResult::Frames(result) => Some(
                    result
                        .frames
                        .iter()
                        .map(|frame| (outcome.provider_id.clone(), frame.clone())),
                ),
                _ => None,
            })
            .flatten()
            .collect();

        let composed = compose_for_prompt(
            offered
                .iter()
                .map(|(provider, frame)| (provider.as_str(), frame)),
            query.max_tokens,
        );

        // The audit is a total partition of `offered`, so reading `admitted` and
        // `dropped` straight off it is what makes this host's own
        // `CCHECK_TOTAL_PARTITION` hold by construction rather than by care.
        let included: Vec<&FrameId> = composed.audit.included().collect();
        let admitted = included
            .iter()
            .filter_map(|id| {
                offered
                    .iter()
                    .find(|(provider, frame)| {
                        provider == &id.provider_id && frame.id == id.frame_id
                    })
                    .cloned()
            })
            .collect();
        let dropped = composed
            .audit
            .excluded()
            .map(|entry| ExcludedFrame {
                provider_id: entry.frame.provider_id.clone(),
                frame_id: entry.frame.frame_id.clone(),
            })
            .collect();

        Composition { admitted, dropped }
    }
}

/// A frame whose declared `token_cost` is the **honest** canonical count for its
/// own content (§7 B3): the content is exactly `token_cost *
/// BYTES_PER_BUDGET_TOKEN` bytes, so `ceil(len / 4) == token_cost`.
///
/// This is load-bearing, and getting it wrong is the first mistake this suite
/// invites — it is the bug the suite's own first run had. A composing host is
/// entitled to pack by a frame's **canonical** cost rather than its declared one;
/// the reference host does, deliberately, so an under-declared frame cannot sneak
/// past the budget. A fixture declaring `token_cost: 400` on a one-byte body is
/// therefore measured as costing 1 by the host and 400 by the suite, and the
/// check fails a *correct* host over a fixture defect.
///
/// So every frame this suite offers satisfies B3. The rule under test is the
/// cross-provider **sum**; posing it over frames that already lie about their
/// individual cost would test something else entirely — something the provider
/// suite's `budget-honesty` check already covers.
fn honest_frame(id: &str, token_cost: u32) -> ContextFrame {
    let content = "x".repeat(token_cost as usize * BYTES_PER_BUDGET_TOKEN);
    debug_assert_eq!(
        budget_tokens(&content),
        token_cost,
        "the fixture must satisfy B3 or the suite measures the wrong thing"
    );
    let mut frame = ContextFrame::full(id, FrameKind::Doc, id, &content, 0.5, token_cost);
    frame.citation_label = Some(id.into());
    frame
}

/// Three local providers, each serving exactly one B3-honest frame costing
/// `token_cost`.
///
/// Each is individually honest for any budget at or above `token_cost`, so
/// whether the set overflows is purely a property of the *sum* — which is what
/// makes this the fixture the per-provider audit cannot help with.
fn three_providers_each_costing(token_cost: u32) -> Vec<Box<dyn ContextProvider>> {
    (0..3)
        .map(|i| {
            let id = format!("p{i}");
            let frame = honest_frame(&format!("{id}-f"), token_cost);
            Box::new(ProbeProvider::local(&id, vec![frame])) as Box<dyn ContextProvider>
        })
        .collect()
}

#[cfg(test)]
mod tests;
