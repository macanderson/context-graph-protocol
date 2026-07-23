//! The replay oracles — pure functions over a parsed [`Journal`] that hold a
//! harness's recording to the loop invariants
//! (`docs/sketches/host-trace.md` §"The oracles").
//!
//! Every oracle is deliberately independent: each walks the journal itself,
//! so a check can be read, tested, and trusted in isolation, and a failure in
//! one cannot mask a failure in another. Journals are small; clarity wins
//! over a shared single pass.
//!
//! The suite is adversarial in the same way `contextgraph-conformance` is:
//! the crate ships a golden journal that passes everything and one fixture
//! per check that trips exactly that check (`tests/fixture_suite.rs`),
//! proving each oracle catches the broken harness it exists for.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use contextgraph_types::{FrameId, Representation, Verdict};

use crate::event::{EventBody, RenderedFrame, SessionOutcome};
use crate::journal::Journal;
use crate::report::{CheckResult, TraceReport};

/// The stable check names, so reports and callers agree on identifiers.
pub const CHECK_SEQUENCE: &str = "sequence-integrity";
pub const CHECK_TURN_LOOP: &str = "turn-loop-pairing";
pub const CHECK_ASSEMBLY_BUDGET: &str = "assembly-budget-honesty";
pub const CHECK_STALENESS: &str = "staleness-at-use";
pub const CHECK_CITATION: &str = "citation-at-use";
pub const CHECK_COMPOSITION: &str = "deterministic-composition";
pub const CHECK_EFFECT_ONCE: &str = "effect-exactly-once";
pub const CHECK_RESUME: &str = "resume-integrity";

/// Every check this suite runs, in report order.
pub const ALL_CHECKS: &[&str] = &[
    CHECK_SEQUENCE,
    CHECK_TURN_LOOP,
    CHECK_ASSEMBLY_BUDGET,
    CHECK_STALENESS,
    CHECK_CITATION,
    CHECK_COMPOSITION,
    CHECK_EFFECT_ONCE,
    CHECK_RESUME,
];

/// Run every oracle over the journal, returning a typed report. Never
/// panics: every defect becomes a failing check whose evidence names the
/// exact `seq` numbers involved.
pub fn run_oracles(journal: &Journal) -> TraceReport {
    TraceReport {
        target: journal.describe(),
        checks: vec![
            check_sequence_integrity(journal),
            check_turn_loop_pairing(journal),
            check_assembly_budget_honesty(journal),
            check_staleness_at_use(journal),
            check_citation_at_use(journal),
            check_deterministic_composition(journal),
            check_effect_exactly_once(journal),
            check_resume_integrity(journal),
        ],
    }
}

/// `sequence-integrity` — the recording itself is trustworthy: `seq` is dense
/// from 1, there is one session, timestamps are in the protocol profile
/// (`SPEC.md` §F4), turn markers balance, and nothing follows `session_end`.
///
/// Every other oracle leans on this one: density is what makes "the journal
/// is complete" checkable, and the turn discipline (a turn does not survive a
/// crash — a `resume` implicitly closes it) is what lets the loop oracles
/// distinguish a crash from a bug.
pub fn check_sequence_integrity(journal: &Journal) -> CheckResult {
    let mut violations = Vec::new();
    let events = &journal.events;

    let first = &events[0];
    if first.seq != 1 {
        violations.push(format!("first event has seq {}, not 1", first.seq));
    }
    if !matches!(first.body, EventBody::SessionStart { .. }) {
        violations.push(format!(
            "recording opens with `{}`, not `session_start`",
            first.body.kind()
        ));
    }

    let session = &first.session;
    let mut open_turn: Option<u64> = None;
    let mut highest_turn: u64 = 0;
    let mut ended_at: Option<u64> = None;

    for (index, event) in events.iter().enumerate() {
        if index > 0 {
            let previous = events[index - 1].seq;
            if event.seq != previous + 1 {
                violations.push(format!(
                    "seq {} follows seq {previous} — the sequence must be dense",
                    event.seq
                ));
            }
            if matches!(event.body, EventBody::SessionStart { .. }) {
                violations.push(format!("second `session_start` at seq {}", event.seq));
            }
        }
        if let Some(end_seq) = ended_at {
            violations.push(format!(
                "`{}` at seq {} follows `session_end` at seq {end_seq}",
                event.body.kind(),
                event.seq
            ));
        }
        if &event.session != session {
            violations.push(format!(
                "seq {} belongs to session '{}' but the journal records '{session}'",
                event.seq, event.session
            ));
        }
        if !contextgraph_types::is_protocol_timestamp(&event.at) {
            violations.push(format!(
                "seq {} timestamp '{}' is not an RFC 3339 UTC timestamp (§F4 profile)",
                event.seq, event.at
            ));
        }

        match &event.body {
            EventBody::TurnStart => match (event.turn, open_turn) {
                (None, _) => {
                    violations.push(format!("`turn_start` at seq {} names no turn", event.seq))
                }
                (Some(turn), Some(open)) => violations.push(format!(
                    "turn {turn} started at seq {} while turn {open} is still open",
                    event.seq
                )),
                (Some(turn), None) => {
                    if turn <= highest_turn {
                        violations.push(format!(
                            "turn {turn} started at seq {} but turn numbers must strictly increase (highest so far: {highest_turn})",
                            event.seq
                        ));
                    }
                    highest_turn = highest_turn.max(turn);
                    open_turn = Some(turn);
                }
            },
            EventBody::TurnEnd => match (event.turn, open_turn) {
                (Some(turn), Some(open)) if turn == open => open_turn = None,
                (turn, open) => violations.push(format!(
                    "`turn_end` at seq {} names turn {turn:?} but the open turn is {open:?}",
                    event.seq
                )),
            },
            // A turn does not survive a crash: a resume implicitly closes
            // any open turn. Session-level events carry no turn.
            EventBody::Resume { .. } => {
                open_turn = None;
                if event.turn.is_some() {
                    violations.push(format!(
                        "`resume` at seq {} carries a turn — resumes are session-level",
                        event.seq
                    ));
                }
            }
            EventBody::SessionStart { .. } | EventBody::SessionEnd { .. } => {
                if event.turn.is_some() {
                    violations.push(format!(
                        "`{}` at seq {} carries a turn — session lifecycle events are session-level",
                        event.body.kind(),
                        event.seq
                    ));
                }
                if let EventBody::SessionEnd { outcome } = &event.body {
                    // A deliberate completion closes its turn first; only an
                    // abort may leave one open.
                    if let Some(open) = open_turn
                        && *outcome == SessionOutcome::Completed
                    {
                        violations.push(format!(
                            "session completed at seq {} with turn {open} still open",
                            event.seq
                        ));
                    }
                    ended_at = Some(event.seq);
                }
            }
            _ => {
                // Inside a turn every event carries the open turn; between
                // turns, none does.
                if event.turn != open_turn {
                    violations.push(format!(
                        "`{}` at seq {} carries turn {:?} but the open turn is {:?}",
                        event.body.kind(),
                        event.seq,
                        event.turn,
                        open_turn
                    ));
                }
            }
        }
    }

    CheckResult::from_violations(
        CHECK_SEQUENCE,
        violations,
        format!(
            "{} event(s), dense 1..={}, one session, timestamps well-formed, turn markers balanced",
            events.len(),
            events.last().map(|event| event.seq).unwrap_or(0)
        ),
    )
}

/// `turn-loop-pairing` — the tool loop's contract: every call the model
/// requested is resolved exactly once *before the next prompt is assembled*,
/// nothing is executed that the model never requested (phantom execution),
/// and no result arrives for a call that was never made or was already
/// resolved.
///
/// The crash carve-outs are deliberate: unresolved calls before a `resume`
/// were orphaned by the crash (expected — the *replay* of their side effects
/// is `effect-exactly-once`'s territory), and a journal that simply stops
/// mid-turn records a crash, not a defect. Only a session that claims
/// `completed` with dangling calls fails here.
pub fn check_turn_loop_pairing(journal: &Journal) -> CheckResult {
    let mut violations = Vec::new();
    // Unresolved model-requested calls: call id → the seq that requested it.
    let mut pending: BTreeMap<String, u64> = BTreeMap::new();
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    let mut executed: BTreeSet<String> = BTreeSet::new();
    let mut ever_requested: BTreeSet<String> = BTreeSet::new();
    let mut total_requested: usize = 0;

    for event in &journal.events {
        match &event.body {
            EventBody::ModelResponse { tool_calls } => {
                for call_id in tool_calls {
                    if !ever_requested.insert(call_id.clone()) {
                        violations.push(format!(
                            "call id `{call_id}` requested again at seq {} — call ids are unique per session",
                            event.seq
                        ));
                        continue;
                    }
                    total_requested += 1;
                    pending.insert(call_id.clone(), event.seq);
                }
            }
            EventBody::ToolCall { call_id, tool } => {
                if executed.contains(call_id) {
                    violations.push(format!(
                        "call `{call_id}` executed again at seq {} — one execution per request",
                        event.seq
                    ));
                } else if resolved.contains(call_id) {
                    violations.push(format!(
                        "call `{call_id}` executed at seq {} after it was already resolved",
                        event.seq
                    ));
                } else if !pending.contains_key(call_id) {
                    violations.push(format!(
                        "`{tool}` executed at seq {} under call id `{call_id}` which the model never requested (phantom execution)",
                        event.seq
                    ));
                } else {
                    executed.insert(call_id.clone());
                }
            }
            EventBody::ToolResult { call_id, .. } => {
                if pending.remove(call_id).is_some() {
                    resolved.insert(call_id.clone());
                } else if resolved.contains(call_id) {
                    violations.push(format!(
                        "call `{call_id}` resolved again at seq {} — exactly one result per call",
                        event.seq
                    ));
                } else {
                    violations.push(format!(
                        "result at seq {} for call `{call_id}` which was never requested (orphan result)",
                        event.seq
                    ));
                }
            }
            EventBody::PromptAssembled { .. } => {
                if !pending.is_empty() {
                    let dangling: Vec<String> = pending
                        .iter()
                        .map(|(call_id, requested_at)| {
                            format!("`{call_id}` (requested at seq {requested_at})")
                        })
                        .collect();
                    violations.push(format!(
                        "prompt assembled at seq {} with {} unresolved tool call(s): {}",
                        event.seq,
                        dangling.len(),
                        dangling.join(", ")
                    ));
                    pending.clear();
                }
            }
            // The crash orphaned whatever was in flight; resumed work starts
            // a new turn with new calls.
            EventBody::Resume { .. } => pending.clear(),
            EventBody::SessionEnd { outcome }
                if *outcome == SessionOutcome::Completed && !pending.is_empty() =>
            {
                let dangling: Vec<&str> = pending.keys().map(|call_id| call_id.as_str()).collect();
                violations.push(format!(
                    "session completed at seq {} with unresolved tool call(s): {}",
                    event.seq,
                    dangling.join(", ")
                ));
            }
            _ => {}
        }
    }

    CheckResult::from_violations(
        CHECK_TURN_LOOP,
        violations,
        format!(
            "{total_requested} call(s) requested, each resolved exactly once before the next prompt"
        ),
    )
}

/// `assembly-budget-honesty` — §B1/§B3 held at the point of assembly, where
/// the harness is the declaring party: the itemized frame costs must sum to
/// the total the harness declared, the sum must fit the budget it announced,
/// and a `reference` frame — which inlines nothing — must cost 0.
pub fn check_assembly_budget_honesty(journal: &Journal) -> CheckResult {
    let mut violations = Vec::new();
    let mut prompts: usize = 0;

    for event in &journal.events {
        let EventBody::PromptAssembled {
            budget_tokens,
            declared_total_tokens,
            frames,
            ..
        } = &event.body
        else {
            continue;
        };
        prompts += 1;
        let itemized: u64 = frames.iter().map(|frame| u64::from(frame.token_cost)).sum();
        if itemized != *declared_total_tokens {
            violations.push(format!(
                "prompt at seq {}: itemized frame costs sum to {itemized} but the harness declared {declared_total_tokens} — the arithmetic drifted from the itemization",
                event.seq
            ));
        }
        if itemized > u64::from(*budget_tokens) {
            violations.push(format!(
                "prompt at seq {}: rendered frame costs sum to {itemized} against the announced budget of {budget_tokens} (§B1 at assembly)",
                event.seq
            ));
        }
        for rendered in frames {
            if rendered.representation == Representation::Reference && rendered.token_cost > 0 {
                violations.push(format!(
                    "prompt at seq {}: reference frame {} declares token_cost {} — a reference inlines nothing, so it costs 0",
                    event.seq,
                    frame_label(&rendered.frame),
                    rendered.token_cost
                ));
            }
        }
    }

    CheckResult::from_violations(
        CHECK_ASSEMBLY_BUDGET,
        violations,
        format!(
            "{prompts} prompt(s) assembled; itemized costs match declared totals and fit their budgets"
        ),
    )
}

/// `staleness-at-use` — the reuse rule (`docs/context-reuse.md` §4, V2) held
/// at the point of use: a frame whose exact identity was last verified
/// `stale` or `gone` must never be rendered again.
///
/// The identity is the full `(provider, frame, digest)` triple, so an honest
/// refresh is invisible here: a re-queried frame carries the source's *new*
/// digest and therefore a different identity. `stale` means the digest
/// changed, so a same-identity render afterwards is either the host reusing
/// the body it was told to drop or the provider contradicting itself — a
/// defect either way. A later `valid` verdict for the identity clears it
/// (verify-after-doubt is exactly how revalidation is supposed to work), and
/// `unknown` does not convict: the host may have re-queried and been
/// re-served the identical bytes, which the journal cannot distinguish.
pub fn check_staleness_at_use(journal: &Journal) -> CheckResult {
    let mut observations: usize = 0;
    let mut rendered: usize = 0;
    // Latest verdict per exact identity: (wire status, whether reuse is dead, seq).
    let mut latest: HashMap<FrameId, (&'static str, bool, u64)> = HashMap::new();
    let mut violations = Vec::new();

    for event in &journal.events {
        match &event.body {
            EventBody::VerifyObserved { frame, verdict } => {
                observations += 1;
                let dead = matches!(verdict, Verdict::Stale { .. } | Verdict::Gone);
                latest.insert(frame.clone(), (verdict.status(), dead, event.seq));
            }
            EventBody::PromptAssembled { frames, .. } => {
                for RenderedFrame { frame, .. } in frames {
                    rendered += 1;
                    if let Some((status, true, verified_at)) = latest.get(frame) {
                        violations.push(format!(
                            "frame {} rendered at seq {} was verified `{status}` at seq {verified_at} — the host MUST NOT keep serving the body it holds (§4 V2)",
                            frame_label(frame),
                            event.seq
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    if observations == 0 {
        return CheckResult::skip(
            CHECK_STALENESS,
            "no verify observations recorded — nothing to hold rendered frames against",
        );
    }
    CheckResult::from_violations(
        CHECK_STALENESS,
        violations,
        format!(
            "{rendered} rendered frame(s) checked against {observations} verify observation(s); none cited dead evidence"
        ),
    )
}

/// `citation-at-use` — §F3's "never a bare uuid", held where it actually
/// matters: a frame rendered into a prompt must carry a non-empty citation
/// label at that moment, not merely have carried one at the provider
/// boundary.
pub fn check_citation_at_use(journal: &Journal) -> CheckResult {
    let mut rendered: usize = 0;
    let mut violations = Vec::new();

    for event in &journal.events {
        let EventBody::PromptAssembled { frames, .. } = &event.body else {
            continue;
        };
        for frame in frames {
            rendered += 1;
            let labelled = frame
                .citation_label
                .as_deref()
                .is_some_and(|label| !label.trim().is_empty());
            if !labelled {
                violations.push(format!(
                    "frame {} rendered at seq {} without a citation label (§F3 at the point of use)",
                    frame_label(&frame.frame),
                    event.seq
                ));
            }
        }
    }

    CheckResult::from_violations(
        CHECK_CITATION,
        violations,
        format!("{rendered} rendered frame(s), every one carrying a citation label"),
    )
}

/// `deterministic-composition` — prefix stability
/// (`docs/context-reuse.md` §1), finally checkable: two prompts rendering the
/// identical frame set (same identities, same representations, same order)
/// must compose to the identical `composition_digest`. A harness whose
/// composition wobbles under an unchanged set is silently destroying the
/// prompt-cache economics the canonical order exists to buy.
///
/// Skipped when the journal records no composition digests — the field is
/// optional precisely so a recorder can adopt the vocabulary before wiring
/// composed-prefix hashing.
pub fn check_deterministic_composition(journal: &Journal) -> CheckResult {
    // Frame-set key → (digest, seq first composed). The key covers identity
    // + representation + order; declared cost is excluded deliberately —
    // identity names the bytes, and §1 is about the rendered bytes.
    let mut compositions: HashMap<String, (String, u64)> = HashMap::new();
    let mut digested: usize = 0;
    let mut violations = Vec::new();

    for event in &journal.events {
        let EventBody::PromptAssembled {
            composition_digest: Some(digest),
            frames,
            ..
        } = &event.body
        else {
            continue;
        };
        digested += 1;
        let key = frame_set_key(frames);
        match compositions.get(&key) {
            None => {
                compositions.insert(key, (digest.clone(), event.seq));
            }
            Some((first_digest, first_seq)) if first_digest != digest => {
                violations.push(format!(
                    "the frame set rendered at seq {} is identical to seq {first_seq} but composed to a different digest — an unchanged set must render byte-identically (§1)",
                    event.seq
                ));
            }
            Some(_) => {}
        }
    }

    if digested == 0 {
        return CheckResult::skip(
            CHECK_COMPOSITION,
            "no composition digests recorded — prefix stability not exercised by this journal",
        );
    }
    CheckResult::from_violations(
        CHECK_COMPOSITION,
        violations,
        format!("{digested} digest-carrying prompt(s); identical frame sets composed identically"),
    )
}

/// `effect-exactly-once` — the crash-replay double-side-effect bug, by
/// construction: `effect_id` names an *intended-once* effect (a deliberate
/// re-execution is a new id), so the same id performed twice is a defect —
/// and the evidence says whether it was replayed across a `resume` boundary
/// (the classic durability bug) or duplicated within one live run.
pub fn check_effect_exactly_once(journal: &Journal) -> CheckResult {
    let mut first_performed: BTreeMap<String, u64> = BTreeMap::new();
    let mut resume_seqs: Vec<u64> = Vec::new();
    let mut effects: usize = 0;
    let mut violations = Vec::new();

    for event in &journal.events {
        match &event.body {
            EventBody::Resume { .. } => resume_seqs.push(event.seq),
            EventBody::SideEffect {
                effect_id, kind, ..
            } => {
                effects += 1;
                match first_performed.get(effect_id) {
                    None => {
                        first_performed.insert(effect_id.clone(), event.seq);
                    }
                    Some(first_seq) => {
                        let across_resume = resume_seqs.iter().find(|resume_seq| {
                            **resume_seq > *first_seq && **resume_seq < event.seq
                        });
                        let boundary = match across_resume {
                            Some(resume_seq) => {
                                format!(" — replayed across the resume at seq {resume_seq}")
                            }
                            None => " — duplicated within one live run".to_string(),
                        };
                        violations.push(format!(
                            "effect `{effect_id}` ({kind}) first performed at seq {first_seq} was performed again at seq {}{boundary}",
                            event.seq
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    CheckResult::from_violations(
        CHECK_EFFECT_ONCE,
        violations,
        format!("{effects} side effect(s), every effect id performed exactly once"),
    )
}

/// `resume-integrity` — a `resume` must recover exactly what the journal
/// records. `last_seq_seen` above the recorded prefix is a recovery of events
/// that never happened (a corrupt recovery); below it is quantified work
/// loss — the resumed harness is blind to events its own durable record
/// holds, which is how a harness re-does work it already did.
pub fn check_resume_integrity(journal: &Journal) -> CheckResult {
    let mut resumes: usize = 0;
    let mut violations = Vec::new();

    for (index, event) in journal.events.iter().enumerate() {
        let EventBody::Resume { last_seq_seen } = &event.body else {
            continue;
        };
        resumes += 1;
        if index == 0 {
            violations.push(format!(
                "resume at seq {} with no prior recorded events — there is nothing to resume",
                event.seq
            ));
            continue;
        }
        let recorded_through = journal.events[index - 1].seq;
        if *last_seq_seen > recorded_through {
            violations.push(format!(
                "resume at seq {} claims to have recovered through seq {last_seq_seen} but the journal records only through seq {recorded_through} — a recovery of events that never happened",
                event.seq
            ));
        } else if *last_seq_seen < recorded_through {
            violations.push(format!(
                "resume at seq {} recovered only through seq {last_seq_seen} of {recorded_through} recorded — {} recorded event(s) invisible to the resumed harness (quantified work loss)",
                event.seq,
                recorded_through - last_seq_seen
            ));
        }
    }

    if resumes == 0 {
        return CheckResult::skip(
            CHECK_RESUME,
            "no resume recorded — durability not exercised by this journal",
        );
    }
    CheckResult::from_violations(
        CHECK_RESUME,
        violations,
        format!("{resumes} resume(s), each recovering exactly the recorded prefix"),
    )
}

/// `provider/frame` — the human-readable name a violation cites a frame by.
fn frame_label(frame: &FrameId) -> String {
    format!("{}/{}", frame.provider_id, frame.frame_id)
}

/// A stable key for a rendered frame set: identity + representation + order.
fn frame_set_key(frames: &[RenderedFrame]) -> String {
    frames
        .iter()
        .map(|rendered| {
            format!(
                "{}\u{1}{}\u{1}{}\u{1}{:?}",
                rendered.frame.provider_id,
                rendered.frame.frame_id,
                rendered.frame.content_digest.as_deref().unwrap_or(""),
                rendered.representation
            )
        })
        .collect::<Vec<_>>()
        .join("\u{2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ToolStatus, TraceEvent};

    fn event(seq: u64, turn: Option<u64>, body: EventBody) -> TraceEvent {
        TraceEvent {
            seq,
            at: "2026-07-23T09:00:00Z".into(),
            session: "sess_1".into(),
            turn,
            body,
        }
    }

    fn start() -> EventBody {
        EventBody::SessionStart {
            agent: "example-agent".into(),
            harness: "stella/0.9".into(),
            model: None,
            trace_format: None,
        }
    }

    fn rendered(digest: &str, label: Option<&str>) -> RenderedFrame {
        RenderedFrame {
            frame: FrameId::new("docs", "frm_1", Some(digest.into())),
            representation: Representation::Full,
            token_cost: 10,
            citation_label: label.map(Into::into),
        }
    }

    fn prompt(frames: Vec<RenderedFrame>) -> EventBody {
        let total: u64 = frames.iter().map(|frame| u64::from(frame.token_cost)).sum();
        EventBody::PromptAssembled {
            budget_tokens: 4096,
            declared_total_tokens: total,
            composition_digest: None,
            frames,
        }
    }

    #[test]
    fn dangling_calls_at_a_crash_are_expected_but_at_completion_are_a_defect() {
        // A journal that stops mid-call records a crash, not a bug.
        let crashed = Journal {
            events: vec![
                event(1, None, start()),
                event(2, Some(1), EventBody::TurnStart),
                event(
                    3,
                    Some(1),
                    EventBody::ModelResponse {
                        tool_calls: vec!["call_1".into()],
                    },
                ),
            ],
        };
        assert_eq!(
            check_turn_loop_pairing(&crashed).status,
            crate::report::CheckStatus::Pass
        );

        // The same dangling call under a deliberate `completed` is the bug.
        let mut events = crashed.events.clone();
        events.push(event(4, Some(1), EventBody::TurnEnd));
        events.push(event(
            5,
            None,
            EventBody::SessionEnd {
                outcome: SessionOutcome::Completed,
            },
        ));
        let completed = Journal { events };
        let result = check_turn_loop_pairing(&completed);
        assert_eq!(result.status, crate::report::CheckStatus::Fail);
        assert!(result.evidence.contains("call_1"), "{}", result.evidence);
    }

    #[test]
    fn a_rejected_call_needs_no_execution_to_be_resolved() {
        // Declining is a resolution, not an execution — a permission gate
        // that answers `rejected` without a `tool_call` is a healthy loop.
        let journal = Journal {
            events: vec![
                event(1, None, start()),
                event(2, Some(1), EventBody::TurnStart),
                event(
                    3,
                    Some(1),
                    EventBody::ModelResponse {
                        tool_calls: vec!["call_1".into()],
                    },
                ),
                event(
                    4,
                    Some(1),
                    EventBody::ToolResult {
                        call_id: "call_1".into(),
                        status: ToolStatus::Rejected,
                    },
                ),
                event(5, Some(1), EventBody::TurnEnd),
                event(
                    6,
                    None,
                    EventBody::SessionEnd {
                        outcome: SessionOutcome::Completed,
                    },
                ),
            ],
        };
        assert_eq!(
            check_turn_loop_pairing(&journal).status,
            crate::report::CheckStatus::Pass
        );
    }

    #[test]
    fn a_valid_verdict_after_a_stale_one_clears_the_identity_for_reuse() {
        // Verify-after-doubt is how revalidation is supposed to work: only
        // the *latest* verdict convicts.
        let frame = FrameId::new("docs", "frm_1", Some("sha256:aaaa".into()));
        let journal = Journal {
            events: vec![
                event(1, None, start()),
                event(
                    2,
                    None,
                    EventBody::VerifyObserved {
                        frame: frame.clone(),
                        verdict: Verdict::Stale {
                            replacement_digest: None,
                        },
                    },
                ),
                event(
                    3,
                    None,
                    EventBody::VerifyObserved {
                        frame,
                        verdict: Verdict::Valid,
                    },
                ),
                event(4, Some(1), EventBody::TurnStart),
                event(
                    5,
                    Some(1),
                    prompt(vec![rendered("sha256:aaaa", Some("workspace.ts"))]),
                ),
                event(6, Some(1), EventBody::TurnEnd),
            ],
        };
        assert_eq!(
            check_staleness_at_use(&journal).status,
            crate::report::CheckStatus::Pass
        );
    }

    #[test]
    fn an_honest_refresh_carries_a_new_digest_and_is_not_convicted() {
        // After `stale`, the host re-queries and the new serve carries the
        // source's new digest — a different identity, invisible here.
        let stale_identity = FrameId::new("docs", "frm_1", Some("sha256:aaaa".into()));
        let journal = Journal {
            events: vec![
                event(1, None, start()),
                event(
                    2,
                    None,
                    EventBody::VerifyObserved {
                        frame: stale_identity,
                        verdict: Verdict::Stale {
                            replacement_digest: Some("sha256:bbbb".into()),
                        },
                    },
                ),
                event(3, Some(1), EventBody::TurnStart),
                event(
                    4,
                    Some(1),
                    prompt(vec![rendered("sha256:bbbb", Some("workspace.ts"))]),
                ),
                event(5, Some(1), EventBody::TurnEnd),
            ],
        };
        assert_eq!(
            check_staleness_at_use(&journal).status,
            crate::report::CheckStatus::Pass
        );
    }

    #[test]
    fn a_reference_frame_with_a_nonzero_cost_is_a_budget_lie() {
        let mut reference = rendered("sha256:aaaa", Some("runbook"));
        reference.representation = Representation::Reference;
        reference.token_cost = 40;
        let journal = Journal {
            events: vec![
                event(1, None, start()),
                event(2, Some(1), EventBody::TurnStart),
                event(3, Some(1), prompt(vec![reference])),
                event(4, Some(1), EventBody::TurnEnd),
            ],
        };
        let result = check_assembly_budget_honesty(&journal);
        assert_eq!(result.status, crate::report::CheckStatus::Fail);
        assert!(
            result.evidence.contains("inlines nothing"),
            "{}",
            result.evidence
        );
    }

    #[test]
    fn work_loss_on_resume_is_quantified_not_just_flagged() {
        let journal = Journal {
            events: vec![
                event(1, None, start()),
                event(2, Some(1), EventBody::TurnStart),
                event(3, Some(1), EventBody::TurnEnd),
                event(4, None, EventBody::Resume { last_seq_seen: 2 }),
            ],
        };
        let result = check_resume_integrity(&journal);
        assert_eq!(result.status, crate::report::CheckStatus::Fail);
        assert!(
            result.evidence.contains("1 recorded event(s) invisible"),
            "{}",
            result.evidence
        );
    }

    #[test]
    fn oracles_that_a_journal_never_exercises_are_skipped_not_passed() {
        let journal = Journal {
            events: vec![
                event(1, None, start()),
                event(
                    2,
                    None,
                    EventBody::SessionEnd {
                        outcome: SessionOutcome::Completed,
                    },
                ),
            ],
        };
        let report = run_oracles(&journal);
        assert!(report.passed());
        let skipped: Vec<&str> = report
            .checks
            .iter()
            .filter(|check| check.status == crate::report::CheckStatus::Skipped)
            .map(|check| check.name.as_str())
            .collect();
        // Honesty about coverage: an unexercised guarantee is declared, not
        // silently counted as upheld.
        assert_eq!(
            skipped,
            vec![CHECK_STALENESS, CHECK_COMPOSITION, CHECK_RESUME]
        );
    }
}
