//! Journal parsing — NDJSON in, ordered [`TraceEvent`]s out.
//!
//! Parsing is deliberately **strict**: a journal is the harness's own
//! recording, so a malformed line means the recorder is broken, and a broken
//! recorder must fail the run loudly rather than have its unparseable lines
//! quietly skipped — a lenient parser here would let a harness escape the
//! oracles by garbling exactly the events that would convict it.

use crate::event::{EventBody, TraceEvent};

/// Why a journal failed to parse. Carries the 1-based line number so the
/// failure is actionable against the file.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("journal line {line} is not a valid trace event: {source}")]
    Malformed {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("journal contains no events")]
    Empty,
}

/// A parsed journal: the recording of one session, including its resumes, in
/// file order. The oracles ([`crate::oracle::run_oracles`]) judge it; this
/// type only carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct Journal {
    pub events: Vec<TraceEvent>,
}

impl Journal {
    /// Parse an NDJSON journal. Blank lines are permitted (trailing newline,
    /// human editing); anything else that does not parse as a [`TraceEvent`]
    /// is an error naming its line.
    pub fn from_ndjson(input: &str) -> Result<Self, JournalError> {
        let mut events = Vec::new();
        for (index, line) in input.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event: TraceEvent =
                serde_json::from_str(line).map_err(|source| JournalError::Malformed {
                    line: index + 1,
                    source,
                })?;
            events.push(event);
        }
        if events.is_empty() {
            return Err(JournalError::Empty);
        }
        Ok(Self { events })
    }

    /// A one-line human description of the recording, for the report header:
    /// the session id, the agent/harness when the journal opens with a
    /// `session_start`, and the event count.
    pub fn describe(&self) -> String {
        let session = self
            .events
            .first()
            .map(|event| event.session.as_str())
            .unwrap_or("<empty>");
        match self.events.first().map(|event| &event.body) {
            Some(EventBody::SessionStart { agent, harness, .. }) => format!(
                "session {session} — agent {agent} (harness {harness}), {} event(s)",
                self.events.len()
            ),
            _ => format!("session {session}, {} event(s)", self.events.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_LINES: &str = concat!(
        r#"{"seq":1,"at":"2026-07-23T09:00:00Z","session":"sess_1","event":"session_start","agent":"example-agent","harness":"stella/0.9"}"#,
        "\n",
        r#"{"seq":2,"at":"2026-07-23T09:00:01Z","session":"sess_1","event":"session_end","outcome":"completed"}"#,
        "\n",
    );

    #[test]
    fn a_well_formed_journal_parses_in_file_order() {
        let journal = Journal::from_ndjson(TWO_LINES).unwrap();
        assert_eq!(journal.events.len(), 2);
        assert_eq!(journal.events[0].seq, 1);
        assert_eq!(journal.events[1].body.kind(), "session_end");
        assert_eq!(
            journal.describe(),
            "session sess_1 — agent example-agent (harness stella/0.9), 2 event(s)"
        );
    }

    #[test]
    fn blank_lines_are_permitted_but_garbage_names_its_line() {
        let with_blank = format!("\n{TWO_LINES}\n");
        assert!(Journal::from_ndjson(&with_blank).is_ok());

        let with_garbage = format!("{TWO_LINES}not json {{{{\n");
        let error = Journal::from_ndjson(&with_garbage).unwrap_err();
        // Strictness is the point: a recorder that garbles a line must fail
        // the run, not have the line skipped.
        assert!(matches!(error, JournalError::Malformed { line: 3, .. }));
    }

    #[test]
    fn an_empty_journal_is_an_error_not_a_vacuous_pass() {
        assert!(matches!(
            Journal::from_ndjson("\n\n"),
            Err(JournalError::Empty)
        ));
    }
}
