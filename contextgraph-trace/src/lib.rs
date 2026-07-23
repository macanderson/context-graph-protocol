//! `contextgraph-trace` — the host execution trace (journal) and its replay
//! oracles.
//!
//! **Sketch stage.** This crate implements
//! [`docs/sketches/host-trace.md`] and is deliberately **unpublished**:
//! nothing here is part of the `contextgraph/1.0` surface. It exists so the
//! shape can be exercised against real journals before any of it is proposed
//! for the spec.
//!
//! [`docs/sketches/host-trace.md`]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/sketches/host-trace.md
//!
//! The conformance suite holds a *provider* honest; nothing holds the
//! host-side agent loop honest. This crate is that missing half, split the
//! same way the rest of the protocol is:
//!
//! - **The journal** ([`TraceEvent`], [`Journal`]) — an append-only NDJSON
//!   recording a harness (or a thin adapter observing one) emits while it
//!   works: turns, prompt assemblies, tool-call pairing, verify observations,
//!   side effects, crashes and resumes. It reuses the protocol's identity
//!   spine — frames are named by [`FrameId`](contextgraph_types::FrameId),
//!   verify observations carry the wire
//!   [`Verdict`](contextgraph_types::Verdict) — and **no frame body ever
//!   travels in it**.
//! - **The oracles** ([`run_oracles`]) — pure replay checks over a parsed
//!   journal, in the conformance suite's vocabulary: named checks,
//!   pass/fail/skip, evidence naming the exact `seq` numbers. They catch the
//!   defects an outcome-graded benchmark structurally cannot see: citing
//!   evidence verified stale, budget arithmetic drifting from the
//!   itemization, phantom tool executions, side effects replayed across a
//!   crash-resume, resumes blind to their own durable record.
//!
//! The oracles never talk to the harness — they read the journal. That split
//! is what makes an eventual benchmark runner agent-agnostic: one adapter per
//! harness maps its native logs onto this vocabulary, and every check
//! downstream is shared.
//!
//! Depends on `contextgraph-types` and serde only, so the oracles stay
//! runnable anywhere the journal can be read.

mod event;
mod journal;
mod oracle;
mod report;

pub use event::{EventBody, RenderedFrame, SessionOutcome, TRACE_FORMAT, ToolStatus, TraceEvent};
pub use journal::{Journal, JournalError};
pub use oracle::{
    ALL_CHECKS, CHECK_ASSEMBLY_BUDGET, CHECK_CITATION, CHECK_COMPOSITION, CHECK_EFFECT_ONCE,
    CHECK_RESUME, CHECK_SEQUENCE, CHECK_STALENESS, CHECK_TURN_LOOP, check_assembly_budget_honesty,
    check_citation_at_use, check_deterministic_composition, check_effect_exactly_once,
    check_resume_integrity, check_sequence_integrity, check_staleness_at_use,
    check_turn_loop_pairing, run_oracles,
};
pub use report::{CheckResult, CheckStatus, TraceReport};
