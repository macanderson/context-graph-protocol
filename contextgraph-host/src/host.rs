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

use std::collections::HashMap;
use std::time::Duration;

use contextgraph_types::{
    ContextFrame, ContextQuery, ContextQueryResult, DataFlow, FrameId, ProviderUsage, ServedFrame,
    UsageReport, Verdict, VerifyRequest,
};

use crate::consent::{ConsentRecord, ConsentStore};
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

    /// Record consent for a provider, unlocking an egress provider for
    /// querying (§3.5).
    pub fn record_consent(&mut self, record: ConsentRecord) {
        self.consent.record(record);
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

    /// Revalidate frames this host already holds, so unchanged context can be
    /// reused without re-querying it (`docs/context-reuse.md` §4).
    ///
    /// Identities are grouped by provider and each capable provider is asked
    /// once. The rule is **default-deny**: a frame is retained only on an
    /// explicit [`Verdict::Valid`], and every other outcome — a negative
    /// verdict, a missing digest, a provider that doesn't support verify, an
    /// unregistered provider, a failed request — drops the frame with a reason
    /// (requirement V2). Reasons that
    /// [warrant a re-query](DropReason::warrants_requery) tell the host which
    /// dropped frames are worth fetching again.
    ///
    /// This method holds **no state**: it neither caches frames nor tracks turn
    /// boundaries. When to re-verify is the host's policy (§4 gives informative
    /// guidance); the protocol's job is only to answer the question when asked.
    /// No frame body travels in either direction.
    pub async fn verify_frames(&self, held: &[FrameId]) -> VerifyOutcome {
        use futures_util::future::join_all;

        // Group by provider, preserving first-seen provider order so the
        // outcome is deterministic for a given input.
        let mut order: Vec<&str> = Vec::new();
        let mut grouped: HashMap<&str, Vec<FrameId>> = HashMap::new();
        for frame in held {
            let id = frame.provider_id.as_str();
            if !grouped.contains_key(id) {
                order.push(id);
            }
            grouped.entry(id).or_default().push(frame.clone());
        }

        let legs = order.into_iter().map(|provider_id| {
            let frames = grouped.remove(provider_id).unwrap_or_default();
            self.verify_one_provider(provider_id, frames)
        });

        let mut outcome = VerifyOutcome::default();
        for leg in join_all(legs).await {
            outcome.retained.extend(leg.retained);
            outcome.dropped.extend(leg.dropped);
        }
        outcome
    }

    /// Verify one provider's slice of the held set, converting every failure
    /// mode into dropped frames rather than a propagated error — one provider's
    /// verify failure never affects another's.
    async fn verify_one_provider(&self, provider_id: &str, frames: Vec<FrameId>) -> VerifyOutcome {
        let mut outcome = VerifyOutcome::default();

        let Some(provider) = self.provider(provider_id) else {
            outcome.drop_all(frames, DropReason::UnknownProvider);
            return outcome;
        };
        if !provider.capabilities().verify {
            // The declared fallback: a provider that can't verify gets its
            // frames re-queried rather than trusted (§4, requirement V3).
            outcome.drop_all(frames, DropReason::VerifyUnsupported);
            return outcome;
        }

        // A frame with no digest can't be revalidated — §1's D4 makes that a
        // re-query, not a reuse. Filter before asking, so the request only
        // carries answerable identities.
        let (verifiable, undigested): (Vec<FrameId>, Vec<FrameId>) =
            frames.into_iter().partition(FrameId::is_verifiable);
        outcome.drop_all(undigested, DropReason::NoDigest);
        if verifiable.is_empty() {
            return outcome;
        }

        let request = VerifyRequest::new(verifiable.clone());
        let response = match tokio::time::timeout(
            self.per_provider_timeout,
            provider.verify(&request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                outcome.drop_all(verifiable, DropReason::VerifyFailed(error.to_string()));
                return outcome;
            }
            Err(_) => {
                let error = HostError::Timeout {
                    id: provider_id.to_string(),
                    timeout_ms: self.per_provider_timeout.as_millis() as u64,
                };
                outcome.drop_all(verifiable, DropReason::VerifyFailed(error.to_string()));
                return outcome;
            }
        };

        for frame in verifiable {
            // Correlate by full identity, never by position. A provider that
            // omits an answer gets `Unknown` — silence is not validity.
            match response.verdict_for(&frame) {
                Some(Verdict::Valid) => outcome.retained.push(frame),
                Some(Verdict::Stale { replacement_digest }) => outcome.drop_one(
                    frame,
                    DropReason::Stale {
                        replacement_digest: replacement_digest.clone(),
                    },
                ),
                Some(Verdict::Gone) => outcome.drop_one(frame, DropReason::Gone),
                Some(Verdict::Unknown) | None => outcome.drop_one(frame, DropReason::Unknown),
            }
        }
        outcome
    }

    /// Query a single provider by id, honoring the consent gate and the
    /// per-provider timeout. Querying an unconsented egress provider is
    /// [`HostError::ConsentRequired`] and the payload is never transmitted
    /// (§3.5).
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

        if !self.consent.permits(provider.id(), provider.info()) {
            return Err(HostError::ConsentRequired {
                id: id.to_string(),
                data_flow: provider.info().data_flow,
            });
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
        // content, so it must never reach an unconsented egress provider.
        if !self.consent.permits(provider.id(), provider.info()) {
            return ProviderOutcome {
                provider_id: id,
                result: ProviderResult::ConsentRequired(provider.info().data_flow),
            };
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
                    ProviderResult::ConsentRequired(_) | ProviderResult::Failed(_) => {
                        ProviderUsage {
                            provider_id,
                            frames_served: 0,
                            frames_rejected: 0,
                            token_cost: 0,
                            served_frames: vec![],
                        }
                    }
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
    /// Skipped: an egress provider without recorded consent. The query
    /// payload was **not** transmitted (§3.5).
    ConsentRequired(DataFlow),
    /// The provider errored, timed out, or crashed mid-query.
    Failed(HostError),
}

/// Why a held frame was dropped by [`Host::verify_frames`]
/// (`docs/context-reuse.md` §4).
///
/// The first three mirror the provider's [`Verdict`]s; the rest are host-side
/// reasons a frame could not be revalidated at all. Either way the frame leaves
/// the composed context — the difference matters only for deciding whether to
/// re-query it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropReason {
    /// The provider answered `stale`: the frame exists but its content changed.
    /// Carries the provider's current digest when it offered one.
    Stale { replacement_digest: Option<String> },
    /// The provider answered `gone` — the frame no longer exists.
    Gone,
    /// The provider answered `unknown`, or returned no verdict for the frame at
    /// all. Silence is not validity.
    Unknown,
    /// The frame carries no `content_digest`, so it cannot be revalidated
    /// (§1, requirement D4).
    NoDigest,
    /// The provider does not advertise the `verify` capability, so the host
    /// falls back to re-querying its frames (§4, requirement V3).
    VerifyUnsupported,
    /// No provider with this frame's `provider_id` is registered with the host.
    UnknownProvider,
    /// The verify request itself failed — a transport error or a timeout.
    VerifyFailed(String),
}

impl DropReason {
    /// Whether re-querying the provider could recover usable content for this
    /// frame. False only for [`Gone`](Self::Gone) — every other reason means
    /// the host simply doesn't have a trustworthy copy and should ask again.
    pub fn warrants_requery(&self) -> bool {
        !matches!(self, Self::Gone)
    }
}

/// One dropped frame and why (`docs/context-reuse.md` §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedFrame {
    /// The identity that was dropped.
    pub frame: FrameId,
    /// Why it was dropped.
    pub reason: DropReason,
}

/// The result of revalidating a held frame set (`docs/context-reuse.md` §4).
///
/// Partitions the input into frames the host may keep reusing and frames it
/// must drop. The partition is **total and default-deny**: every input identity
/// appears in exactly one of the two lists, and it lands in `retained` only on
/// an explicit `valid`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyOutcome {
    /// Frames that verified `valid` — safe to keep reusing, and the frames
    /// whose byte-stable reuse §1's canonical ordering was built to protect.
    pub retained: Vec<FrameId>,
    /// Frames that must leave the composed context, each with its reason.
    pub dropped: Vec<DroppedFrame>,
}

impl VerifyOutcome {
    /// The dropped frames worth re-querying — everything except `gone`, which
    /// is not there to re-fetch.
    pub fn requery(&self) -> impl Iterator<Item = &FrameId> {
        self.dropped
            .iter()
            .filter(|dropped| dropped.reason.warrants_requery())
            .map(|dropped| &dropped.frame)
    }

    /// Whether an identity was dropped.
    pub fn was_dropped(&self, frame: &FrameId) -> bool {
        self.dropped.iter().any(|dropped| &dropped.frame == frame)
    }

    /// The reason an identity was dropped, if it was.
    pub fn drop_reason(&self, frame: &FrameId) -> Option<&DropReason> {
        self.dropped
            .iter()
            .find(|dropped| &dropped.frame == frame)
            .map(|dropped| &dropped.reason)
    }

    fn drop_one(&mut self, frame: FrameId, reason: DropReason) {
        self.dropped.push(DroppedFrame { frame, reason });
    }

    fn drop_all(&mut self, frames: impl IntoIterator<Item = FrameId>, reason: DropReason) {
        for frame in frames {
            self.drop_one(frame, reason.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            Self {
                id: id.into(),
                info: ProviderInfo {
                    name: id.into(),
                    version: "0.0.1".into(),
                    data_flow: DataFlow {
                        reads: true,
                        writes: false,
                        egress,
                    },
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
            },
            "issue titles leave to github.com",
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

    // ---- context/verify (§4) ----

    use contextgraph_types::{FrameVerdict, VerifyResponse};
    use std::collections::HashMap as StdHashMap;

    /// A provider that answers `context/verify` from a scripted verdict table.
    struct VerifyingProvider {
        id: String,
        capabilities: Capabilities,
        /// frame id -> verdict. A frame absent from the table gets no verdict
        /// entry at all, exercising the "silence is not validity" path.
        verdicts: StdHashMap<String, Verdict>,
        /// When set, `verify` fails instead of answering.
        verify_error: Option<String>,
        /// Identities this provider was actually asked about.
        asked: Arc<std::sync::Mutex<Vec<FrameId>>>,
    }

    impl VerifyingProvider {
        fn new(id: &str, supports_verify: bool, verdicts: &[(&str, Verdict)]) -> Self {
            Self {
                id: id.into(),
                capabilities: Capabilities {
                    query: QueryCapability {
                        kinds: vec!["doc".into()],
                        filters: vec![],
                    },
                    verify: supports_verify,
                    ..Capabilities::default()
                },
                verdicts: verdicts
                    .iter()
                    .map(|(f, v)| ((*f).to_string(), v.clone()))
                    .collect(),
                verify_error: None,
                asked: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn failing(id: &str) -> Self {
            let mut provider = Self::new(id, true, &[]);
            provider.verify_error = Some("index unavailable".into());
            provider
        }
    }

    #[async_trait]
    impl ContextProvider for VerifyingProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn info(&self) -> &ProviderInfo {
            // Local-only: nothing here is about consent.
            static_info()
        }
        fn capabilities(&self) -> &Capabilities {
            &self.capabilities
        }
        async fn query(&self, _query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
            Ok(ContextQueryResult {
                frames: vec![],
                truncated: false,
                dropped_estimate: None,
            })
        }
        async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, HostError> {
            self.asked
                .lock()
                .unwrap()
                .extend(request.frames.iter().cloned());
            if let Some(message) = &self.verify_error {
                return Err(HostError::Provider {
                    id: self.id.clone(),
                    message: message.clone(),
                });
            }
            Ok(VerifyResponse::new(
                request
                    .frames
                    .iter()
                    .filter_map(|frame| {
                        self.verdicts
                            .get(&frame.frame_id)
                            .map(|verdict| FrameVerdict::new(frame.clone(), verdict.clone()))
                    })
                    .collect(),
            ))
        }
    }

    fn static_info() -> &'static ProviderInfo {
        use std::sync::OnceLock;
        static INFO: OnceLock<ProviderInfo> = OnceLock::new();
        INFO.get_or_init(|| ProviderInfo {
            name: "verifier".into(),
            version: "0.0.1".into(),
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: false,
            },
        })
    }

    fn held(provider: &str, frame: &str, digest: Option<&str>) -> FrameId {
        FrameId::new(provider, frame, digest.map(String::from))
    }

    #[tokio::test]
    async fn a_stale_frame_is_dropped_and_a_valid_one_is_retained() {
        // The core §4 guarantee: the host demonstrably evicts a frame the
        // provider says has changed, and keeps the one it vouches for.
        let mut host = Host::new();
        host.register(Box::new(VerifyingProvider::new(
            "docs",
            true,
            &[
                ("fresh", Verdict::Valid),
                (
                    "changed",
                    Verdict::Stale {
                        replacement_digest: Some("sha256:new".into()),
                    },
                ),
            ],
        )));

        let fresh = held("docs", "fresh", Some("sha256:a"));
        let changed = held("docs", "changed", Some("sha256:b"));
        let outcome = host.verify_frames(&[fresh.clone(), changed.clone()]).await;

        assert_eq!(outcome.retained, vec![fresh]);
        assert!(outcome.was_dropped(&changed));
        assert_eq!(
            outcome.drop_reason(&changed),
            Some(&DropReason::Stale {
                replacement_digest: Some("sha256:new".into())
            })
        );
        // A stale frame is worth re-fetching; the replacement digest tells the
        // host what it would be getting.
        assert_eq!(outcome.requery().collect::<Vec<_>>(), vec![&changed]);
    }

    #[tokio::test]
    async fn a_gone_frame_is_dropped_and_not_worth_re_querying() {
        let mut host = Host::new();
        host.register(Box::new(VerifyingProvider::new(
            "docs",
            true,
            &[("deleted", Verdict::Gone)],
        )));
        let deleted = held("docs", "deleted", Some("sha256:a"));
        let outcome = host.verify_frames(std::slice::from_ref(&deleted)).await;

        assert!(outcome.retained.is_empty());
        assert_eq!(outcome.drop_reason(&deleted), Some(&DropReason::Gone));
        // Nothing to re-fetch — `gone` is the one reason that doesn't warrant it.
        assert_eq!(outcome.requery().count(), 0);
    }

    #[tokio::test]
    async fn an_unknown_verdict_and_a_missing_verdict_both_drop_the_frame() {
        // Silence is not validity: a provider that omits an answer must not
        // have that read as "still good".
        let mut host = Host::new();
        host.register(Box::new(VerifyingProvider::new(
            "docs",
            true,
            &[("shrugged", Verdict::Unknown)],
        )));
        let shrugged = held("docs", "shrugged", Some("sha256:a"));
        let unanswered = held("docs", "never-mentioned", Some("sha256:b"));
        let outcome = host
            .verify_frames(&[shrugged.clone(), unanswered.clone()])
            .await;

        assert!(outcome.retained.is_empty());
        assert_eq!(outcome.drop_reason(&shrugged), Some(&DropReason::Unknown));
        assert_eq!(outcome.drop_reason(&unanswered), Some(&DropReason::Unknown));
        assert_eq!(outcome.requery().count(), 2);
    }

    #[tokio::test]
    async fn a_provider_without_verify_support_is_never_asked_and_falls_back_to_requery() {
        // The capability gate (V3): the host doesn't send a verify request at
        // all, it just re-queries.
        let mut host = Host::new();
        let provider = VerifyingProvider::new("docs", false, &[("anything", Verdict::Valid)]);
        let asked = provider.asked.clone();
        host.register(Box::new(provider));

        let frame = held("docs", "anything", Some("sha256:a"));
        let outcome = host.verify_frames(std::slice::from_ref(&frame)).await;

        assert!(asked.lock().unwrap().is_empty(), "must not be asked");
        assert!(outcome.retained.is_empty());
        assert_eq!(
            outcome.drop_reason(&frame),
            Some(&DropReason::VerifyUnsupported)
        );
        assert_eq!(outcome.requery().count(), 1);
    }

    #[tokio::test]
    async fn a_frame_without_a_digest_is_unverifiable_and_never_sent() {
        // §1 D4: no digest, no revalidation — and the request only carries
        // answerable identities.
        let mut host = Host::new();
        let provider = VerifyingProvider::new("docs", true, &[("bare", Verdict::Valid)]);
        let asked = provider.asked.clone();
        host.register(Box::new(provider));

        let bare = held("docs", "bare", None);
        let digested = held("docs", "digested", Some("sha256:a"));
        let outcome = host.verify_frames(&[bare.clone(), digested.clone()]).await;

        assert_eq!(outcome.drop_reason(&bare), Some(&DropReason::NoDigest));
        let asked = asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![digested],
            "only verifiable identities go on the wire"
        );
    }

    #[tokio::test]
    async fn a_failed_verify_drops_that_providers_frames_without_touching_another() {
        // Per-provider isolation, same contract as a query fan-out leg.
        let mut host = Host::new();
        host.register(Box::new(VerifyingProvider::failing("broken")));
        host.register(Box::new(VerifyingProvider::new(
            "healthy",
            true,
            &[("good", Verdict::Valid)],
        )));

        let broken = held("broken", "any", Some("sha256:a"));
        let good = held("healthy", "good", Some("sha256:b"));
        let outcome = host.verify_frames(&[broken.clone(), good.clone()]).await;

        assert_eq!(outcome.retained, vec![good], "one failure must not poison");
        assert!(matches!(
            outcome.drop_reason(&broken),
            Some(DropReason::VerifyFailed(_))
        ));
        assert!(outcome.requery().any(|frame| frame == &broken));
    }

    #[tokio::test]
    async fn frames_from_an_unregistered_provider_are_dropped_not_ignored() {
        let host = Host::new();
        let orphan = held("never-registered", "f", Some("sha256:a"));
        let outcome = host.verify_frames(std::slice::from_ref(&orphan)).await;
        assert!(outcome.retained.is_empty());
        assert_eq!(
            outcome.drop_reason(&orphan),
            Some(&DropReason::UnknownProvider)
        );
    }

    #[tokio::test]
    async fn held_frames_are_grouped_into_one_request_per_provider() {
        // Verification costs bytes, not tokens — so it must not cost a round
        // trip per frame either.
        let mut host = Host::new();
        let docs = VerifyingProvider::new(
            "docs",
            true,
            &[("a", Verdict::Valid), ("b", Verdict::Valid)],
        );
        let asked = docs.asked.clone();
        host.register(Box::new(docs));

        let outcome = host
            .verify_frames(&[
                held("docs", "a", Some("sha256:1")),
                held("docs", "b", Some("sha256:2")),
            ])
            .await;
        assert_eq!(outcome.retained.len(), 2);
        // Both identities arrived together in a single verify call.
        assert_eq!(asked.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_partition_is_total_so_every_held_frame_is_accounted_for() {
        let mut host = Host::new();
        host.register(Box::new(VerifyingProvider::new(
            "docs",
            true,
            &[("keep", Verdict::Valid), ("drop", Verdict::Gone)],
        )));
        let input = vec![
            held("docs", "keep", Some("sha256:1")),
            held("docs", "drop", Some("sha256:2")),
            held("docs", "nodigest", None),
            held("elsewhere", "orphan", Some("sha256:3")),
        ];
        let outcome = host.verify_frames(&input).await;
        assert_eq!(
            outcome.retained.len() + outcome.dropped.len(),
            input.len(),
            "every held identity must land in exactly one bucket"
        );
        for frame in &input {
            assert!(
                outcome.retained.contains(frame) || outcome.was_dropped(frame),
                "{frame:?} was silently lost"
            );
        }
    }

    #[tokio::test]
    async fn verifying_an_empty_held_set_is_a_no_op() {
        let host = Host::new();
        let outcome = host.verify_frames(&[]).await;
        assert_eq!(outcome, VerifyOutcome::default());
    }
}
