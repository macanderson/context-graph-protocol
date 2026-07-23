//! The trace event vocabulary (`docs/sketches/host-trace.md` §"The shape").
//!
//! One [`TraceEvent`] per journal line. The vocabulary is deliberately
//! minimal — each event exists because an oracle in [`crate::oracle`] consumes
//! it, and nothing else is recorded. Frames are named by
//! [`FrameId`] and verify observations carry the wire [`Verdict`], so the
//! journal reuses the protocol's own identity spine instead of inventing a
//! parallel one. **No frame body ever travels in the journal** — identities
//! and costs only, the same economy `context/verify` runs on.

use contextgraph_types::{FrameId, Representation, Verdict};
use serde::{Deserialize, Serialize};

/// The trace-format identifier a recorder SHOULD stamp into
/// [`EventBody::SessionStart::trace_format`], so an oracle suite can refuse a
/// journal written to a vocabulary it does not understand.
pub const TRACE_FORMAT: &str = "contextgraph-trace/0.1-sketch";

/// One journal line: a dense sequence number, a timestamp in the protocol's
/// RFC 3339 UTC profile (`SPEC.md` §F4), the session it belongs to, the open
/// turn (when inside one), and the event body flattened alongside.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Dense and strictly increasing from 1, continuing across a
    /// crash-resume. Density is what makes "this journal is complete"
    /// checkable at all.
    pub seq: u64,
    /// RFC 3339 UTC timestamp, same profile as frame temporal fields.
    pub at: String,
    /// The session this recording belongs to. One journal records one
    /// session, including its resumes.
    pub session: String,
    /// The open turn's number. Present on `turn_start`/`turn_end` (naming the
    /// turn they bound) and on every event inside a turn; absent between
    /// turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    #[serde(flatten)]
    pub body: EventBody,
}

/// How a session ended — when it ended at all. A journal whose last event is
/// not `session_end` records a crash, and that absence is load-bearing: the
/// oracles treat dangling work before a [`EventBody::Resume`] as expected and
/// the same work *replayed after* one as the defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutcome {
    /// The harness finished its task and tore down deliberately.
    Completed,
    /// The harness stopped deliberately without finishing (user interrupt,
    /// budget exhaustion). Unresolved tool calls are permitted here.
    Aborted,
}

/// The outcome of executing (or declining) one model-requested tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    Error,
    /// The harness declined to execute the call (permission gate, policy). A
    /// rejected call needs no [`EventBody::ToolCall`] — declining is a
    /// resolution, not an execution.
    Rejected,
}

/// One frame at its point of use: the identity that names its exact bytes,
/// what rendering it was given, what the harness declared it cost, and the
/// label a human would see it cited under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedFrame {
    pub frame: FrameId,
    /// How the frame was carried into the prompt. Absent ⇒ `full`, matching
    /// the frame wire shape.
    #[serde(default, skip_serializing_if = "Representation::is_full")]
    pub representation: Representation,
    /// The inline token cost the harness accounted for this frame at
    /// assembly. A `reference` frame inlines nothing, so it MUST be 0.
    pub token_cost: u32,
    /// The citation label at the point of use — §F3's "never a bare uuid",
    /// held where it actually matters: the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_label: Option<String>,
}

/// The event bodies. Serialized internally tagged on `event` and flattened
/// into the [`TraceEvent`] envelope, so a journal line reads
/// `{"seq":5,…,"event":"tool_call","call_id":"call_1",…}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventBody {
    /// The recording opens: which agent, under which harness, is being traced.
    SessionStart {
        /// The agent under trace (adapter-declared, e.g. `example-agent`).
        agent: String,
        /// The harness identity and version (e.g. `stella/0.9`).
        harness: String,
        /// The model in use, when the adapter knows it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// The trace vocabulary this journal is written to — see
        /// [`TRACE_FORMAT`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_format: Option<String>,
    },
    /// The session tore down deliberately. A journal without one records a
    /// crash.
    SessionEnd { outcome: SessionOutcome },
    /// The harness came back after a crash. `last_seq_seen` is the highest
    /// `seq` the resumed harness actually recovered — the journal knows what
    /// it recorded, so the delta between the two is *quantified* work loss.
    /// A resume implicitly closes any open turn and orphans any unresolved
    /// tool calls; resumed work starts a new turn.
    Resume { last_seq_seen: u64 },
    /// A turn opens. The envelope `turn` names it.
    TurnStart,
    /// The open turn closes. The envelope `turn` must match.
    TurnEnd,
    /// The harness composed a prompt and sent it. Recording assembly *as* the
    /// model request is deliberate: the journal claims what was sent is what
    /// was assembled, and every context guarantee is checked at this moment —
    /// the point of use.
    PromptAssembled {
        /// The context budget the harness announced for this prompt, in
        /// budget tokens (`SPEC.md` §7).
        budget_tokens: u32,
        /// The harness's own total of the rendered frame costs — checked
        /// against the per-frame declarations, so the arithmetic can't drift
        /// from the itemization.
        declared_total_tokens: u64,
        /// Digest of the composed context section (algorithm
        /// recorder-declared, e.g. `sha256:<hex>`). Optional; when present,
        /// an unchanged frame set MUST reproduce it byte-identically
        /// (`docs/context-reuse.md` §1 prefix stability, finally checkable).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        composition_digest: Option<String>,
        /// The frames rendered into this prompt, in composition order.
        frames: Vec<RenderedFrame>,
    },
    /// The model answered, requesting zero or more tool calls by id. The ids
    /// are the loop's contract: each must be resolved exactly once before the
    /// next `prompt_assembled`.
    ModelResponse {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<String>,
    },
    /// The harness began executing a model-requested call. Executing a call
    /// the model never requested is the phantom-execution defect.
    ToolCall { call_id: String, tool: String },
    /// A requested call was resolved — executed to completion, errored, or
    /// declined ([`ToolStatus::Rejected`]).
    ToolResult { call_id: String, status: ToolStatus },
    /// The host observed a `context/verify` answer for a frame it holds
    /// (`docs/context-reuse.md` §4). Rendering the same identity after a
    /// `stale`/`gone` verdict is the citing-dead-evidence defect.
    VerifyObserved { frame: FrameId, verdict: Verdict },
    /// The harness performed an externally visible action (file write,
    /// network call, command). `effect_id` names an *intended-once* effect: a
    /// deliberate re-execution is a new id, so the same id twice is the
    /// crash-replay bug by construction.
    SideEffect {
        effect_id: String,
        /// e.g. `file_write`, `network`, `command`.
        kind: String,
        /// The tool call this effect was performed under, when there is one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
    },
}

impl EventBody {
    /// The event's wire name, for evidence strings and log lines.
    pub fn kind(&self) -> &'static str {
        match self {
            EventBody::SessionStart { .. } => "session_start",
            EventBody::SessionEnd { .. } => "session_end",
            EventBody::Resume { .. } => "resume",
            EventBody::TurnStart => "turn_start",
            EventBody::TurnEnd => "turn_end",
            EventBody::PromptAssembled { .. } => "prompt_assembled",
            EventBody::ModelResponse { .. } => "model_response",
            EventBody::ToolCall { .. } => "tool_call",
            EventBody::ToolResult { .. } => "tool_result",
            EventBody::VerifyObserved { .. } => "verify_observed",
            EventBody::SideEffect { .. } => "side_effect",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_line_roundtrips_with_the_body_flattened() {
        let event = TraceEvent {
            seq: 5,
            at: "2026-07-23T09:00:10Z".into(),
            session: "sess_1".into(),
            turn: Some(1),
            body: EventBody::ToolCall {
                call_id: "call_1".into(),
                tool: "write_file".into(),
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        // Flattened: the body's fields sit beside the envelope's, tagged by
        // `event` — one flat object per line, greppable by field name.
        assert!(json.contains("\"event\":\"tool_call\""));
        assert!(json.contains("\"call_id\":\"call_1\""));
        assert!(!json.contains("\"body\""));
        let back: TraceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn absent_turn_and_optional_fields_are_omitted_not_null() {
        let event = TraceEvent {
            seq: 1,
            at: "2026-07-23T09:00:00Z".into(),
            session: "sess_1".into(),
            turn: None,
            body: EventBody::SessionStart {
                agent: "example-agent".into(),
                harness: "stella/0.9".into(),
                model: None,
                trace_format: Some(TRACE_FORMAT.into()),
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("\"turn\""));
        assert!(!json.contains("\"model\""));
        assert!(json.contains(TRACE_FORMAT));
    }

    #[test]
    fn a_rendered_full_frame_omits_representation_like_the_frame_wire_shape() {
        let rendered = RenderedFrame {
            frame: FrameId::new("docs", "frm_1", Some("sha256:9f2c".into())),
            representation: Representation::Full,
            token_cost: 120,
            citation_label: Some("workspace.ts L120-160".into()),
        };
        let json = serde_json::to_string(&rendered).unwrap();
        assert!(!json.contains("representation"));
        let back: RenderedFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rendered);
    }

    #[test]
    fn verify_observed_carries_the_wire_verdict_shape() {
        let event = TraceEvent {
            seq: 9,
            at: "2026-07-23T09:00:14Z".into(),
            session: "sess_1".into(),
            turn: None,
            body: EventBody::VerifyObserved {
                frame: FrameId::new("docs", "frm_1", Some("sha256:9f2c".into())),
                verdict: Verdict::Stale {
                    replacement_digest: None,
                },
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        // The verdict serializes exactly as `context/verify` answers do —
        // `{"status":"stale"}` — so an adapter can copy it straight through.
        assert!(json.contains("\"verdict\":{\"status\":\"stale\"}"));
        let back: TraceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
