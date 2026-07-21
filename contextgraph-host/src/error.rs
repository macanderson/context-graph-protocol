//! `HostError` — the one typed error the host runtime raises
//! (`SPEC.md` §10 "fail loud"). Everything a fan-out or a single
//! provider exchange can go wrong with is a named variant here; nothing in
//! the hot path panics. `contextgraph-host` owns its own error type rather than
//! borrowing `stella`'s so the crate stays industry-facing and dependency-
//! light (`SPEC.md` §1 — depends only on `contextgraph-types` + transport
//! crates).

use contextgraph_types::DataFlow;

/// Anything the host runtime can surface while talking to a provider.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// The provider speaks an incompatible protocol family
    /// (`SPEC.md`). Reported the instant the handshake ack
    /// arrives — never a hang (task deliverable 1).
    #[error(
        "protocol version mismatch: host speaks {host}, provider {provider} speaks {provider_version}"
    )]
    VersionMismatch {
        host: String,
        provider: String,
        provider_version: String,
    },

    /// A line/body could not be encoded to or decoded from the wire envelope
    /// (`SPEC.md` §2). A malformed provider message is a
    /// clean error, never a host crash (task deliverable 5).
    #[error("wire encode/decode error: {0}")]
    Wire(String),

    /// The underlying transport (stdio pipe or HTTP) failed.
    #[error("transport error talking to provider {id}: {message}")]
    Transport { id: String, message: String },

    /// The provider's child process closed its stream mid-exchange — it
    /// crashed. Isolated to this provider; never poisons a `query_all`
    /// (task deliverable 5).
    #[error("provider {id} crashed or closed its stream mid-exchange")]
    ProviderCrashed { id: String },

    /// The provider took longer than the host's per-provider budget.
    #[error("provider {id} timed out after {timeout_ms}ms")]
    Timeout { id: String, timeout_ms: u64 },

    /// The provider reported an error over the wire (an `error` envelope).
    #[error("provider {id} reported an error: {message}")]
    Provider { id: String, message: String },

    /// The provider declares `egress` and has no recorded consent, so the
    /// host refuses to transmit a query to it (`SPEC.md`
    /// SPEC.md §4 — a host MUST NOT auto-enable egress providers). The query
    /// payload never left the host.
    #[error(
        "provider {id} declares egress and requires one-time consent naming what leaves before it can be queried"
    )]
    ConsentRequired { id: String, data_flow: DataFlow },

    /// A message of the wrong kind arrived where the protocol expected a
    /// specific envelope (e.g. a `frames` reply to a `query`).
    #[error("expected a `{expected}` envelope from provider {id}, got `{got}`")]
    UnexpectedEnvelope {
        id: String,
        expected: String,
        got: String,
    },

    /// A provider that declared `correlation` answered without echoing the
    /// request's `id`, or echoed the wrong one (`SPEC.md` §H4).
    ///
    /// Fatal to the exchange rather than a warning: once replies cannot be
    /// matched to requests, a pipelining host could hand one caller's frames to
    /// another, and silently mixing evidence between tasks is worse than
    /// failing the query.
    #[error("provider {id} broke request correlation: expected id `{expected}`, got `{got}`")]
    CorrelationMismatch {
        id: String,
        expected: String,
        got: String,
    },

    /// No provider is registered under the given id.
    #[error("no provider registered with id `{0}`")]
    UnknownProvider(String),

    /// Spawning the provider child process failed.
    #[error("failed to spawn provider process: {0}")]
    Spawn(String),
}
