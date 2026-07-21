//! The [`Host`] — one uniform handle over every provider, and the fan-out
//! router (`06-context-protocol.md` §2.3, §3.3; `02-architecture.md` §7).
//!
//! The host does the four jobs providers never do (§7): routes a query to
//! capability-matching providers, gates consent so nothing reaches an
//! unconsented egress provider, enforces per-provider timeouts, and audits
//! budget honesty — a provider that returns frames summing above the query
//! budget lied about `token_cost`, so its frames are dropped with a loud
//! named report rather than silently trusted (§3.3, task deliverable 3).
//! Per-provider isolation is total: one provider erroring, timing out, being
//! dropped for a budget lie, or crashing mid-query never poisons the others
//! (task deliverable 5).

use std::time::Duration;

use contextgraph_types::{
    ContextFrame, ContextQuery, ContextQueryResult, DataFlow, EgressScope, ProviderUsage,
    ServedFrame, UsageReport,
};

use crate::consent::{ConsentDecision, ConsentReceipt, ConsentRecord, ConsentStore};
use crate::error::HostError;
use crate::provider::{ContextProvider, capability_matches};
use crate::stdio::StdioProvider;

/// Default per-provider query budget — a slow or hung provider is cut off at
/// this and reported as [`HostError::Timeout`], never allowed to stall the
/// fan-out.
const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

/// Registers in-process, stdio, and HTTP providers behind one handle and
/// fans queries out across them.
pub struct Host {
    providers: Vec<Box<dyn ContextProvider>>,
    consent: ConsentStore,
    per_provider_timeout: Duration,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    /// A host with no providers and the default per-provider timeout.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            consent: ConsentStore::new(),
            per_provider_timeout: DEFAULT_PROVIDER_TIMEOUT,
        }
    }

    /// A host with a custom per-provider timeout.
    pub fn with_timeout(per_provider_timeout: Duration) -> Self {
        Self {
            per_provider_timeout,
            ..Self::new()
        }
    }

    /// Register an in-process provider (a built-in, e.g. the code graph).
    pub fn register(&mut self, provider: Box<dyn ContextProvider>) {
        self.providers.push(provider);
    }

    /// Spawn and register a child-process provider over stdio, completing the
    /// handshake (`06-context-protocol.md` §3.2).
    pub async fn add_stdio(
        &mut self,
        id: impl Into<String>,
        program: &str,
        args: &[String],
    ) -> Result<(), HostError> {
        let provider = StdioProvider::spawn(id, program, args).await?;
        self.providers.push(Box::new(provider));
        Ok(())
    }

    /// Connect and register a remote HTTP provider, completing the handshake.
    pub async fn add_http(
        &mut self,
        id: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<(), HostError> {
        let provider = crate::http::HttpProvider::connect(id, url).await?;
        self.providers.push(Box::new(provider));
        Ok(())
    }

    /// Record legacy boolean consent for a provider, unlocking an egress
    /// provider that declares no scopes for querying (§3.5).
    pub fn record_consent(&mut self, record: ConsentRecord) {
        self.consent.record(record);
    }

    /// Append a scope-level [`ConsentReceipt`] to the audit ledger, authorizing
    /// one egress scope for one provider (`docs/context-reuse.md` §3). A
    /// provider that declares off-machine egress scopes stays gated until every
    /// such scope has a receipt.
    pub fn record_receipt(&mut self, receipt: ConsentReceipt) {
        self.consent.record_receipt(receipt);
    }

    /// The consent store (read-only), e.g. to persist decisions.
    pub fn consent(&self) -> &ConsentStore {
        &self.consent
    }

    /// The ids of every registered provider, in registration order.
    pub fn provider_ids(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.id()).collect()
    }

    /// Borrow a registered provider by id, e.g. to read its cached
    /// capabilities.
    pub fn provider(&self, id: &str) -> Option<&dyn ContextProvider> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    /// How many providers are registered.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Query a single provider by id, honoring the consent gate and the
    /// per-provider timeout. Querying an unconsented egress provider is
    /// [`HostError::ConsentRequired`] (legacy boolean) or
    /// [`HostError::ConsentScopeRequired`] (an off-machine scope with no
    /// receipt, §3), and the payload is never transmitted (§3.5).
    pub async fn query_provider(
        &self,
        id: &str,
        query: &ContextQuery,
    ) -> Result<ContextQueryResult, HostError> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == id)
            .ok_or_else(|| HostError::UnknownProvider(id.to_string()))?;

        match self.consent.evaluate(provider.id(), provider.info()) {
            ConsentDecision::Permitted => {}
            ConsentDecision::NeedsConsent => {
                return Err(HostError::ConsentRequired {
                    id: id.to_string(),
                    data_flow: provider.info().data_flow.clone(),
                });
            }
            ConsentDecision::NeedsReceipts(scopes) => {
                return Err(HostError::ConsentScopeRequired {
                    id: id.to_string(),
                    scopes,
                });
            }
        }

        match tokio::time::timeout(self.per_provider_timeout, provider.query(query)).await {
            Ok(result) => result,
            Err(_) => Err(HostError::Timeout {
                id: id.to_string(),
                timeout_ms: self.per_provider_timeout.as_millis() as u64,
            }),
        }
    }

    /// Fan a query out to every capability-matching provider concurrently,
    /// collecting a per-provider outcome. Each provider is consent-gated,
    /// timed out, and budget-audited independently — the crash-consistency
    /// contract means one provider's failure never affects another
    /// (task deliverables 3 + 5).
    pub async fn query_all(&self, query: &ContextQuery) -> FanOut {
        use futures_util::future::join_all;

        let futures: Vec<_> = self
            .providers
            .iter()
            .filter(|p| capability_matches(p.capabilities(), query))
            .map(|p| self.query_one_isolated(p.as_ref(), query))
            .collect();

        FanOut {
            outcomes: join_all(futures).await,
        }
    }

    /// Run one provider's leg of a fan-out, converting every failure mode into
    /// a value — never a propagated error that could abort sibling legs.
    async fn query_one_isolated(
        &self,
        provider: &dyn ContextProvider,
        query: &ContextQuery,
    ) -> ProviderOutcome {
        let id = provider.id().to_string();

        // Consent gate first: the query payload itself may carry workspace
        // content, so it must never reach an unconsented egress provider —
        // whether gated by the legacy boolean flag or by an unconsented
        // off-machine egress scope (§3).
        match self.consent.evaluate(provider.id(), provider.info()) {
            ConsentDecision::Permitted => {}
            ConsentDecision::NeedsConsent => {
                return ProviderOutcome {
                    provider_id: id,
                    result: ProviderResult::ConsentRequired(provider.info().data_flow.clone()),
                };
            }
            ConsentDecision::NeedsReceipts(scopes) => {
                return ProviderOutcome {
                    provider_id: id,
                    result: ProviderResult::ConsentScopeRequired {
                        data_flow: provider.info().data_flow.clone(),
                        missing: scopes,
                    },
                };
            }
        }

        let result =
            match tokio::time::timeout(self.per_provider_timeout, provider.query(query)).await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => {
                    return ProviderOutcome {
                        provider_id: id,
                        result: ProviderResult::Failed(error),
                    };
                }
                Err(_) => {
                    let error = HostError::Timeout {
                        id: id.clone(),
                        timeout_ms: self.per_provider_timeout.as_millis() as u64,
                    };
                    return ProviderOutcome {
                        provider_id: id,
                        result: ProviderResult::Failed(error),
                    };
                }
            };

        // Budget honesty: frames that sum above the query budget are a lie
        // about `token_cost`. Drop them, report loudly (§3.3).
        if !result.respects_budget(query.max_tokens) {
            return ProviderOutcome {
                provider_id: id,
                result: ProviderResult::BudgetLie {
                    claimed_tokens: result.total_token_cost(),
                    max_tokens: query.max_tokens,
                    dropped_frames: result.frames.len(),
                },
            };
        }

        ProviderOutcome {
            provider_id: id,
            result: ProviderResult::Frames(result),
        }
    }

    /// Shut every provider down cleanly, consuming the host so its stdio
    /// children are reaped as they drop. Returns each provider's shutdown
    /// result so a caller can log stragglers.
    pub async fn shutdown(self) -> Vec<(String, Result<(), HostError>)> {
        let mut results = Vec::with_capacity(self.providers.len());
        for provider in &self.providers {
            results.push((provider.id().to_string(), provider.shutdown().await));
        }
        results
    }
}

/// The result of fanning one query out across all capability-matching
/// providers.
#[derive(Debug)]
pub struct FanOut {
    /// One entry per provider that matched the query's frame kinds, in
    /// registration order.
    pub outcomes: Vec<ProviderOutcome>,
}

impl FanOut {
    /// Every frame from providers that passed the consent gate, the timeout,
    /// and the budget-honesty audit — the frames a host may honestly compose
    /// into a prompt.
    pub fn accepted_frames(&self) -> impl Iterator<Item = &ContextFrame> {
        self.outcomes
            .iter()
            .filter_map(|outcome| match &outcome.result {
                ProviderResult::Frames(result) => Some(result.frames.iter()),
                _ => None,
            })
            .flatten()
    }

    /// The summed honest token cost of every accepted frame.
    pub fn total_accepted_tokens(&self) -> u64 {
        self.accepted_frames().map(|f| f.token_cost as u64).sum()
    }

    /// Every accepted frame paired with the id of the provider that served it
    /// — the input to deterministic composition (`docs/context-reuse.md` §1).
    pub fn accepted_with_provider(&self) -> impl Iterator<Item = (&str, &ContextFrame)> {
        self.outcomes
            .iter()
            .filter_map(|outcome| match &outcome.result {
                ProviderResult::Frames(result) => Some(
                    result
                        .frames
                        .iter()
                        .map(move |frame| (outcome.provider_id.as_str(), frame)),
                ),
                _ => None,
            })
            .flatten()
    }

    /// Compose every accepted frame into a byte-stable context block via the
    /// deterministic composition contract — canonical order, relevance-free
    /// rendering (`docs/context-reuse.md` §1). Two fan-outs over the same
    /// frame set compose to identical bytes, so an unchanged turn extends the
    /// provider's prompt cache instead of busting it.
    pub fn compose(&self) -> String {
        crate::compose::compose_context(self.accepted_with_provider())
    }

    /// Roll this fan-out up into a per-request [`UsageReport`] for metering
    /// (`docs/context-reuse.md` §2). One [`ProviderUsage`] per provider the
    /// query reached: accepted frames are itemized by stable identity and
    /// declared cost, a budget-lying provider's dropped frames count as
    /// rejected, and a failed or consent-gated provider served nothing.
    ///
    /// The report is a pure function of this fan-out plus the two host-supplied
    /// scalars: `budget_requested` is the query's `max_tokens`, and `as_of` is
    /// the accounting snapshot time (an RFC 3339 string the host stamps — the
    /// report's own as-of, *not* the query's bi-temporal `as_of` pin). The
    /// result always satisfies [`UsageReport::is_consistent`]: its totals are
    /// summed from the same served frames it itemizes.
    pub fn usage_report(&self, query: &ContextQuery, as_of: impl Into<String>) -> UsageReport {
        let providers: Vec<ProviderUsage> = self
            .outcomes
            .iter()
            .map(|outcome| {
                let provider_id = outcome.provider_id.clone();
                match &outcome.result {
                    ProviderResult::Frames(result) => {
                        let served_frames: Vec<ServedFrame> = result
                            .frames
                            .iter()
                            .map(|frame| ServedFrame {
                                frame: frame.identity(&provider_id),
                                token_cost: frame.token_cost,
                            })
                            .collect();
                        let token_cost = served_frames.iter().map(|s| s.token_cost as u64).sum();
                        ProviderUsage {
                            provider_id,
                            frames_served: served_frames.len() as u32,
                            frames_rejected: 0,
                            token_cost,
                            served_frames,
                        }
                    }
                    // A budget lie: the provider's frames were dropped whole,
                    // so nothing was served and every offered frame is rejected.
                    ProviderResult::BudgetLie { dropped_frames, .. } => ProviderUsage {
                        provider_id,
                        frames_served: 0,
                        frames_rejected: *dropped_frames as u32,
                        token_cost: 0,
                        served_frames: vec![],
                    },
                    // Consent-gated or failed: no frames offered, none served,
                    // none rejected — the leg simply contributed nothing.
                    ProviderResult::ConsentRequired(_)
                    | ProviderResult::ConsentScopeRequired { .. }
                    | ProviderResult::Failed(_) => ProviderUsage {
                        provider_id,
                        frames_served: 0,
                        frames_rejected: 0,
                        token_cost: 0,
                        served_frames: vec![],
                    },
                }
            })
            .collect();

        let budget_consumed = providers.iter().map(|p| p.token_cost).sum();
        UsageReport {
            budget_requested: query.max_tokens,
            budget_consumed,
            as_of: as_of.into(),
            providers,
        }
    }

    /// Providers that failed (error, timeout, or crash), with their errors.
    pub fn failures(&self) -> impl Iterator<Item = (&str, &HostError)> {
        self.outcomes
            .iter()
            .filter_map(|outcome| match &outcome.result {
                ProviderResult::Failed(error) => Some((outcome.provider_id.as_str(), error)),
                _ => None,
            })
    }

    /// Providers whose frames were dropped for exceeding the query budget —
    /// the loud report the host must surface, never swallow (§3.3).
    pub fn budget_liars(&self) -> impl Iterator<Item = &ProviderOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| matches!(outcome.result, ProviderResult::BudgetLie { .. }))
    }
}

/// One provider's outcome within a [`FanOut`].
#[derive(Debug)]
pub struct ProviderOutcome {
    pub provider_id: String,
    pub result: ProviderResult,
}

/// What became of one provider's leg of a fan-out — a total function over
/// every failure mode, so no leg can abort another.
#[derive(Debug)]
pub enum ProviderResult {
    /// Frames the host accepted: passed consent, timeout, and budget honesty.
    Frames(ContextQueryResult),
    /// The provider's frames summed above the query budget — a `token_cost`
    /// lie. Dropped and reported (§3.3).
    BudgetLie {
        claimed_tokens: u64,
        max_tokens: u32,
        dropped_frames: usize,
    },
    /// Skipped: an egress provider (declaring no scopes) without recorded
    /// boolean consent. The query payload was **not** transmitted (§3.5).
    ConsentRequired(DataFlow),
    /// Skipped: the provider declares off-machine egress scope(s) with no
    /// recorded consent receipt (`docs/context-reuse.md` §3). `missing` names
    /// the scopes lacking a receipt. The query payload was **not** transmitted.
    ConsentScopeRequired {
        data_flow: DataFlow,
        missing: Vec<EgressScope>,
    },
    /// The provider errored, timed out, or crashed mid-query.
    Failed(HostError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consent::Grantor;
    use async_trait::async_trait;
    use contextgraph_types::capability::QueryCapability;
    use contextgraph_types::{Capabilities, ContextFrame, FrameKind, ProviderInfo};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A configurable in-process provider for exercising the router.
    struct FakeProvider {
        id: String,
        info: ProviderInfo,
        capabilities: Capabilities,
        behavior: Behavior,
        queried: Arc<AtomicBool>,
    }

    enum Behavior {
        Frames(Vec<ContextFrame>),
        Fail(String),
        Slow(Duration),
    }

    impl FakeProvider {
        fn new(id: &str, egress: bool, behavior: Behavior) -> Self {
            Self::with_data_flow(
                id,
                DataFlow {
                    reads: true,
                    writes: false,
                    egress,
                    egress_scopes: vec![],
                },
                behavior,
            )
        }

        /// A provider declaring egress scopes, for the scope-consent gate.
        fn scoped(id: &str, scopes: Vec<EgressScope>, behavior: Behavior) -> Self {
            Self::with_data_flow(
                id,
                DataFlow {
                    reads: true,
                    writes: false,
                    egress: true,
                    egress_scopes: scopes,
                },
                behavior,
            )
        }

        fn with_data_flow(id: &str, data_flow: DataFlow, behavior: Behavior) -> Self {
            Self {
                id: id.into(),
                info: ProviderInfo {
                    name: id.into(),
                    version: "0.0.1".into(),
                    data_flow,
                },
                capabilities: Capabilities {
                    query: QueryCapability {
                        kinds: vec!["doc".into()],
                        filters: vec![],
                    },
                    ..Capabilities::default()
                },
                behavior,
                queried: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    #[async_trait]
    impl ContextProvider for FakeProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn info(&self) -> &ProviderInfo {
            &self.info
        }
        fn capabilities(&self) -> &Capabilities {
            &self.capabilities
        }
        async fn query(&self, _query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
            self.queried.store(true, Ordering::SeqCst);
            match &self.behavior {
                Behavior::Frames(frames) => Ok(ContextQueryResult {
                    frames: frames.clone(),
                    truncated: false,
                    dropped_estimate: None,
                }),
                Behavior::Fail(message) => Err(HostError::Provider {
                    id: self.id.clone(),
                    message: message.clone(),
                }),
                Behavior::Slow(duration) => {
                    tokio::time::sleep(*duration).await;
                    Ok(ContextQueryResult {
                        frames: vec![],
                        truncated: false,
                        dropped_estimate: None,
                    })
                }
            }
        }
    }

    fn frame(id: &str, cost: u32) -> ContextFrame {
        ContextFrame {
            id: id.into(),
            kind: FrameKind::Doc,
            title: id.into(),
            content: "c".into(),
            content_digest: None,
            uri: None,
            score: 0.5,
            token_cost: cost,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: vec![],
            citation_label: Some(id.into()),
            embedding: None,
            relations: vec![],
        }
    }

    fn query() -> ContextQuery {
        ContextQuery {
            goal: "g".into(),
            query_text: None,
            embedding: None,
            kinds: vec![],
            anchors: vec![],
            max_frames: 10,
            max_tokens: 1000,
            as_of: None,
        }
    }

    #[tokio::test]
    async fn query_all_collects_frames_from_healthy_providers() {
        let mut host = Host::new();
        host.register(Box::new(FakeProvider::new(
            "a",
            false,
            Behavior::Frames(vec![frame("f1", 100), frame("f2", 100)]),
        )));
        host.register(Box::new(FakeProvider::new(
            "b",
            false,
            Behavior::Frames(vec![frame("f3", 50)]),
        )));

        let fanout = host.query_all(&query()).await;
        assert_eq!(fanout.outcomes.len(), 2);
        assert_eq!(fanout.accepted_frames().count(), 3);
        assert_eq!(fanout.total_accepted_tokens(), 250);
    }

    #[tokio::test]
    async fn the_host_composes_the_same_frame_set_to_identical_bytes_across_turns() {
        // The reference host's deterministic-composition round trip
        // (`docs/context-reuse.md` §1): the same frame set, fanned out twice,
        // composes to byte-identical bytes — so an unchanged turn extends the
        // provider's prompt-cache prefix instead of forfeiting it.
        let mut host = Host::new();
        host.register(Box::new(FakeProvider::new(
            "prov-b",
            false,
            Behavior::Frames(vec![frame("f2", 100), frame("f1", 100)]),
        )));
        host.register(Box::new(FakeProvider::new(
            "prov-a",
            false,
            Behavior::Frames(vec![frame("f3", 50)]),
        )));

        let first = host.query_all(&query()).await.compose();
        let second = host.query_all(&query()).await.compose();
        assert_eq!(
            first, second,
            "an unchanged frame set must compose to identical bytes"
        );
        // All three frames are present, each fenced exactly once, and the
        // lower-sorting provider id renders first regardless of registration
        // order.
        assert_eq!(first.matches("<frame ").count(), 3);
        assert!(first.find("prov-a").unwrap() < first.find("prov-b").unwrap());
    }

    #[tokio::test]
    async fn a_fan_out_rolls_up_into_a_self_consistent_usage_report() {
        let mut host = Host::new();
        host.register(Box::new(FakeProvider::new(
            "a",
            false,
            Behavior::Frames(vec![frame("f1", 100), frame("f2", 100)]),
        )));
        // 1200 tokens against a 1000-token budget: dropped as a budget lie.
        host.register(Box::new(FakeProvider::new(
            "liar",
            false,
            Behavior::Frames(vec![frame("big", 1200)]),
        )));

        let query = query();
        let fanout = host.query_all(&query).await;
        let report = fanout.usage_report(&query, "2026-07-21T00:00:00Z");

        assert_eq!(report.budget_requested, 1000);
        assert_eq!(report.budget_consumed, 200);
        assert_eq!(report.as_of, "2026-07-21T00:00:00Z");
        // The report re-sums from its own itemized frames…
        assert!(report.is_consistent());
        assert!(report.within_budget());
        // …and its consumed total equals an INDEPENDENT re-sum of the accepted
        // frames — the arithmetic identity, not a build-then-assert tautology.
        let independent: u64 = fanout.accepted_frames().map(|f| f.token_cost as u64).sum();
        assert_eq!(report.budget_consumed, independent);
        assert_eq!(report.budget_consumed, fanout.total_accepted_tokens());

        let a = report
            .providers
            .iter()
            .find(|p| p.provider_id == "a")
            .expect("provider a is in the report");
        assert_eq!(a.frames_served, 2);
        assert_eq!(a.frames_rejected, 0);
        assert_eq!(a.token_cost, 200);
        // Served frames are itemized by stable identity for audit walk-back.
        let ids: Vec<&str> = a
            .served_frames
            .iter()
            .map(|s| s.frame.frame_id.as_str())
            .collect();
        assert!(ids.contains(&"f1") && ids.contains(&"f2"));
        assert!(a.served_frames.iter().all(|s| s.frame.provider_id == "a"));

        let liar = report
            .providers
            .iter()
            .find(|p| p.provider_id == "liar")
            .expect("the liar is still accounted for");
        assert_eq!(liar.frames_served, 0);
        assert_eq!(liar.frames_rejected, 1);
        assert_eq!(liar.token_cost, 0);
        assert!(liar.served_frames.is_empty());
    }

    #[tokio::test]
    async fn a_provider_lying_about_token_cost_has_its_frames_dropped_loudly() {
        let mut host = Host::new();
        // 1200 tokens claimed against a 1000-token budget: a lie.
        host.register(Box::new(FakeProvider::new(
            "liar",
            false,
            Behavior::Frames(vec![frame("big", 1200)]),
        )));
        host.register(Box::new(FakeProvider::new(
            "honest",
            false,
            Behavior::Frames(vec![frame("ok", 200)]),
        )));

        let fanout = host.query_all(&query()).await;
        // The liar's frames never reach the accepted set…
        assert_eq!(fanout.accepted_frames().count(), 1);
        assert_eq!(fanout.total_accepted_tokens(), 200);
        // …and the lie is reported loudly, not swallowed.
        let liars: Vec<_> = fanout.budget_liars().collect();
        assert_eq!(liars.len(), 1);
        assert_eq!(liars[0].provider_id, "liar");
        match liars[0].result {
            ProviderResult::BudgetLie {
                claimed_tokens,
                max_tokens,
                dropped_frames,
            } => {
                assert_eq!(claimed_tokens, 1200);
                assert_eq!(max_tokens, 1000);
                assert_eq!(dropped_frames, 1);
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn one_failing_provider_never_poisons_the_others() {
        let mut host = Host::new();
        host.register(Box::new(FakeProvider::new(
            "healthy",
            false,
            Behavior::Frames(vec![frame("f", 10)]),
        )));
        host.register(Box::new(FakeProvider::new(
            "broken",
            false,
            Behavior::Fail("kaboom".into()),
        )));

        let fanout = host.query_all(&query()).await;
        assert_eq!(fanout.accepted_frames().count(), 1);
        let failures: Vec<_> = fanout.failures().collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "broken");
    }

    #[tokio::test]
    async fn a_slow_provider_is_timed_out_without_stalling_the_fan_out() {
        let mut host = Host::with_timeout(Duration::from_millis(50));
        host.register(Box::new(FakeProvider::new(
            "fast",
            false,
            Behavior::Frames(vec![frame("f", 10)]),
        )));
        host.register(Box::new(FakeProvider::new(
            "slow",
            false,
            Behavior::Slow(Duration::from_secs(30)),
        )));

        let fanout = host.query_all(&query()).await;
        assert_eq!(fanout.accepted_frames().count(), 1);
        let failures: Vec<_> = fanout.failures().collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "slow");
        assert!(matches!(failures[0].1, HostError::Timeout { .. }));
    }

    #[tokio::test]
    async fn an_egress_provider_is_not_queried_until_consent_is_recorded() {
        let mut host = Host::new();
        let provider = FakeProvider::new("github", true, Behavior::Frames(vec![frame("f", 10)]));
        let queried = provider.queried.clone();
        host.register(Box::new(provider));

        // Without consent: skipped, and — critically — query() never ran, so
        // the payload never left.
        let fanout = host.query_all(&query()).await;
        assert_eq!(fanout.accepted_frames().count(), 0);
        assert!(!queried.load(Ordering::SeqCst), "payload must not be sent");
        assert!(matches!(
            fanout.outcomes[0].result,
            ProviderResult::ConsentRequired(_)
        ));
        // Direct query is the named error.
        assert!(matches!(
            host.query_provider("github", &query()).await,
            Err(HostError::ConsentRequired { .. })
        ));

        // After consent: queried and its frames accepted.
        host.record_consent(ConsentRecord::new(
            "github",
            DataFlow {
                reads: true,
                writes: false,
                egress: true,
                egress_scopes: vec![],
            },
            "issue titles leave to github.com",
        ));
        let fanout = host.query_all(&query()).await;
        assert!(queried.load(Ordering::SeqCst));
        assert_eq!(fanout.accepted_frames().count(), 1);
    }

    #[tokio::test]
    async fn a_scoped_egress_provider_is_not_queried_until_a_receipt_is_recorded() {
        let mut host = Host::new();
        let provider = FakeProvider::scoped(
            "cloud",
            vec![EgressScope::ThirdPartyModel],
            Behavior::Frames(vec![frame("f", 10)]),
        );
        let queried = provider.queried.clone();
        let info = provider.info().clone();
        host.register(Box::new(provider));

        // Without a receipt: skipped as a scope-consent gap, and the payload
        // never left.
        let fanout = host.query_all(&query()).await;
        assert_eq!(fanout.accepted_frames().count(), 0);
        assert!(!queried.load(Ordering::SeqCst), "payload must not be sent");
        match &fanout.outcomes[0].result {
            ProviderResult::ConsentScopeRequired { missing, .. } => {
                assert_eq!(missing, &vec![EgressScope::ThirdPartyModel]);
            }
            other => panic!("expected ConsentScopeRequired, got {other:?}"),
        }
        // Direct query is the scope-specific typed error naming what would leave.
        match host.query_provider("cloud", &query()).await {
            Err(HostError::ConsentScopeRequired { scopes, .. }) => {
                assert_eq!(scopes, vec![EgressScope::ThirdPartyModel]);
            }
            other => panic!("expected ConsentScopeRequired error, got {other:?}"),
        }

        // After a receipt for the declared scope: queried and accepted.
        host.record_receipt(ConsentReceipt::new(
            "cloud",
            &info,
            EgressScope::ThirdPartyModel,
            Grantor::Human("ops@oxagen.sh".into()),
            "2026-07-21T00:00:00Z",
        ));
        let fanout = host.query_all(&query()).await;
        assert!(queried.load(Ordering::SeqCst));
        assert_eq!(fanout.accepted_frames().count(), 1);
    }

    #[tokio::test]
    async fn query_provider_reports_unknown_ids() {
        let host = Host::new();
        assert!(matches!(
            host.query_provider("nope", &query()).await,
            Err(HostError::UnknownProvider(_))
        ));
    }
}
