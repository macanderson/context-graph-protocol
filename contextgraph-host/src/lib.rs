//! `contextgraph-host` — the Context Graph Protocol host runtime.
//!
//! An Context Graph Protocol **host** is the side of the protocol that asks for context: it
//! discovers providers, negotiates capabilities, routes a
//! [`ContextQuery`](contextgraph_types::ContextQuery) to the ones that can answer,
//! budgets and cites what comes back, and gates what may leave the machine.
//! This crate is that host runtime: today it is exercised by the Context Graph Protocol
//! conformance suite and drives the `contextgraph-inspect` tool, and it is usable by
//! any Rust agent that wants Context Graph Protocol support (`02-architecture.md` §2). Note that
//! the in-tree context providers do **not** yet route through this host —
//! they share `contextgraph-types` values via in-process calls — so this is the host
//! runtime and conformance harness for the protocol, not (yet) the path every
//! built-in source is served through.
//! `docs/specs/stella-rust-cli/06-context-protocol.md` is the normative
//! specification; every module cites the section it implements.
//!
//! # Shape
//!
//! - [`Envelope`] + [`wire`] — the versioned NDJSON message envelope and its
//!   framing (§3.1). Version mismatch is a named error, never a hang.
//! - [`ContextProvider`] — the one trait every source implements, whether
//!   in-process, a stdio child, or a remote HTTP endpoint (§3.2, §3.3).
//! - [`StdioProvider`] / [`RawStdioConnection`] — child-process transport
//!   with scrubbed-environment isolation and process-group teardown (§3.5).
//! - [`HttpProvider`] — remote streamable-HTTP transport (§3.2).
//! - [`ConsentStore`] — the gate that keeps an egress provider un-queried
//!   until the user consents, naming what leaves (§3.5).
//! - [`Host`] — registers all three provider kinds behind one handle and
//!   [`Host::query_all`] fans a query out concurrently, enforcing timeouts,
//!   consent, and budget honesty (§2.3, §7).
//!
//! # Isolation invariants (`06-context-protocol.md` §3.5)
//!
//! What is enforced today: a stdio child is spawned with a **scrubbed
//! environment** (`env_clear` plus a `PATH`/`HOME` allowlist), so it inherits
//! no credentials or secrets the host holds via environment variables; each
//! call is bounded by a timeout, and on Unix the child leads its own process
//! group so a crash or hang is contained and reaped without touching its
//! siblings. An `egress` provider is never auto-enabled. Frame content is
//! untrusted data; this crate only ever *transports* it — it never executes
//! frame content, and a host composing frames into a prompt must delimit them
//! as quoted material.
//!
//! **Not yet enforced — filesystem confinement.** A child runs with the
//! host's working directory and ordinary filesystem access; there is no cwd
//! jail, chroot, mount namespace, or seccomp sandbox. Environment scrubbing
//! blocks credentials passed *via env vars*, but a provider can still read
//! files the host user can read. Treat a stdio provider as trusted code you
//! chose to run, not as a sandboxed principal — real filesystem isolation is
//! future work.

pub mod consent;
pub mod error;
pub mod host;
pub mod http;
pub mod provider;
pub mod stdio;
pub mod wire;

pub use consent::{ConsentRecord, ConsentStore};
pub use error::HostError;
pub use host::{FanOut, Host, ProviderOutcome, ProviderResult};
pub use http::HttpProvider;
pub use provider::{ContextProvider, capability_matches, frame_kind_name};
pub use stdio::{RawStdioConnection, StdioProvider};
pub use wire::{Envelope, decode_line, encode_line, envelope_kind, versions_compatible};

/// The Context Graph Protocol protocol version this host speaks, re-exported from `contextgraph-types`
/// (`06-context-protocol.md` §3).
pub use contextgraph_types::PROTOCOL_VERSION;
