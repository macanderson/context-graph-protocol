//! Tests for prompt ingestion (ADR 0006). The load-bearing properties: every
//! emitted frame is honest (§B3) and structurally valid for its representation;
//! the same paste content-addresses to the same frame; and the provider plugs
//! into the ordinary `ContextProvider` contract for query and verify.

use super::*;
use contextgraph_types::FrameId;

// ---- SHA-256 known-answer vectors ----
//
// A wrong digest silently kills dedup, so the hasher is pinned to the standard
// vectors — including the padding boundaries (empty, one block, the two-block
// case) where a hand-rolled SHA-256 would break. We use `sha2`, but the KATs
// guard our hex encoding and the `sha256:` framing regardless.

#[test]
fn sha256_matches_the_standard_known_answer_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // 56 bytes: forces a second padding block (the classic FIPS-180 vector).
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // A multi-block input well over 64 bytes.
    let million_a = "a".repeat(1000);
    assert_eq!(
        sha256_hex(million_a.as_bytes()).len(),
        64,
        "hex must always be 64 lowercase chars"
    );
}

#[test]
fn a_digest_is_well_formed_and_lowercase() {
    let digest = sha256_digest("hello");
    assert!(contextgraph_types::is_well_formed_digest(&digest));
    assert_eq!(digest, digest.to_lowercase());
}

// ---- Segmentation ----

fn kinds(attachment: &str) -> Vec<SegmentKind> {
    split_blocks(attachment).iter().map(classify).collect()
}

#[test]
fn a_log_block_is_classified_as_a_log() {
    let log = "\
2026-07-20 18:00:01 INFO  starting retry loop
2026-07-20 18:00:02 WARN  attempt 1 failed, backing off
2026-07-20 18:00:05 WARN  attempt 2 failed, backing off
2026-07-20 18:00:11 ERROR giving up after 3 attempts";
    assert_eq!(kinds(log), vec![SegmentKind::Log]);
}

#[test]
fn a_pipe_table_is_classified_as_a_table() {
    let table = "\
| id | name  | active |
|----|-------|--------|
| 1  | alice | true   |
| 2  | bob   | false  |";
    assert_eq!(kinds(table), vec![SegmentKind::Table]);
}

#[test]
fn a_fenced_block_is_code_regardless_of_content() {
    let code = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
    assert_eq!(kinds(code), vec![SegmentKind::Code]);
}

#[test]
fn a_bare_path_is_a_path_reference_not_a_frame() {
    for path in [
        "./src/net",
        "../lib/mod.rs",
        "/repo/src",
        "net.rs",
        "file:///a/b",
    ] {
        assert_eq!(kinds(path), vec![SegmentKind::PathRef], "{path}");
    }
    // A URL is not a workspace anchor.
    assert_eq!(kinds("https://example.com/x"), vec![SegmentKind::Prose]);
    // A sentence with spaces is prose, even if it mentions a slash.
    assert_eq!(kinds("see the a/b split here"), vec![SegmentKind::Prose]);
}

#[test]
fn ordinary_prose_stays_prose() {
    let prose = "The retry loop gives up too early. I think the backoff is wrong \
                 and we exhaust attempts before the service recovers.";
    assert_eq!(kinds(prose), vec![SegmentKind::Prose]);
}

#[test]
fn blocks_are_split_on_blank_lines_and_fences() {
    let mixed = "\
first paragraph of prose

2026-07-20 18:00:01 ERROR boom
2026-07-20 18:00:02 ERROR boom again

```
raw code here
more code
```";
    assert_eq!(
        kinds(mixed),
        vec![SegmentKind::Prose, SegmentKind::Log, SegmentKind::Code]
    );
}

// ---- Segmentation: stack traces ----

#[test]
fn a_traceback_is_its_own_kind_in_every_common_dialect() {
    // Python puts the exception last, Java first, Rust in a panic header — the
    // detector has to recognize all three or the "paste a traceback" case,
    // which is most of what anyone pastes, falls back to the log distiller.
    for trace in [PYTHON_TRACEBACK, JAVA_TRACEBACK, RUST_PANIC] {
        assert_eq!(kinds(trace), vec![SegmentKind::StackTrace], "{trace}");
    }
}

#[test]
fn a_log_that_merely_contains_a_traceback_is_still_a_log() {
    // The precedence guard: a few `at …` lines inside eighty log lines do not
    // make the paste a traceback, and the salience distiller is the right one
    // for the whole block.
    let mut log = build_big_log(80);
    log.push_str("\njava.lang.IllegalStateException: pool exhausted");
    log.push_str("\n\tat com.acme.pool.Pool.borrow(Pool.java:118)");
    log.push_str("\n\tat com.acme.net.Client.send(Client.java:64)");
    assert_eq!(kinds(&log), vec![SegmentKind::Log]);
}

#[test]
fn a_sentence_that_mentions_an_error_does_not_open_a_stack_trace() {
    // A header is at most two tokens before the colon, precisely so a clause
    // cannot open a trace — otherwise a paragraph quoting an exception name
    // would drag the surrounding prose into the trace distiller.
    assert!(!is_exception_header(
        "we looked and there was an Error: the retry budget"
    ));
    assert!(!is_exception_header(
        "Error handling: see the retry section"
    ));
    assert!(is_exception_header(
        "java.lang.IllegalStateException: pool exhausted"
    ));
    assert!(is_exception_header(
        "Uncaught TypeError: x is not a function"
    ));
    // A trace pasted out of a log still opens one, ceremony and all.
    assert!(is_exception_header(
        "2026-07-20 18:00:01 ERROR [worker-3] java.lang.IllegalStateException: pool exhausted"
    ));
    // Frame-shaped lines with no header are never a trace.
    let prose = "\
we looked at the config and there was an error
at least three services were affected
at some point the pool recovered on its own";
    assert_ne!(kinds(prose), vec![SegmentKind::StackTrace]);
}

#[test]
fn distilling_a_trace_keeps_the_exception_and_the_top_frames() {
    let deep = deep_java_traceback(30);
    let distilled = distill_stack_trace(&deep);

    // The line that says what went wrong survives — it is the whole reason the
    // trace was pasted.
    assert!(distilled.starts_with("java.lang.IllegalStateException: pool exhausted"));
    // The top of the stack (where the fault is) survives; the tail does not.
    assert!(distilled.contains("frame00"));
    assert!(!distilled.contains("frame29"));
    // And the loss is *stated*, not silent.
    assert!(
        distilled.contains(&format!("… ({} more frames)", 30 - STACK_FRAMES)),
        "distilled trace must count the frames it dropped:\n{distilled}"
    );
    assert!(budget_tokens(&distilled) < budget_tokens(&deep));
}

#[test]
fn distilling_a_short_trace_changes_nothing() {
    // Fewer frames than the cap: nothing to elide, so the distiller must return
    // the source unchanged rather than reformat it (which would cost a
    // representation flip for no saving).
    assert_eq!(distill_stack_trace(PYTHON_TRACEBACK), PYTHON_TRACEBACK);
}

#[test]
fn a_python_frame_keeps_its_source_line() {
    // Python's source line is a continuation of the `File "…"` above it, and
    // dropping it would leave a frame naming a line nobody can see.
    let deep = deep_python_traceback(20);
    let distilled = distill_stack_trace(&deep);
    assert!(distilled.contains("File \"app.py\", line 0, in step0"));
    assert!(
        distilled.contains("step1()"),
        "kept frames keep their source"
    );
    // Python's trailing exception line is *after* every frame, and survives.
    assert!(distilled.ends_with("ValueError: backoff exhausted"));
}

// ---- Segmentation: tables ----

#[test]
fn a_csv_is_classified_as_a_table() {
    let csv = "\
endpoint,calls,p99_ms
/v1/query,1841,210
/v1/resolve,92,1204
/v1/handshake,1841,8";
    assert_eq!(kinds(csv), vec![SegmentKind::Table]);
}

#[test]
fn a_comma_spliced_paragraph_is_not_a_csv() {
    // Every line has exactly one comma, so the delimiter count agrees perfectly
    // — the cell-shape guard is the only thing standing between prose and the
    // table distiller.
    let prose = "\
the retry loop gives up too early, I think the backoff is wrong
we exhaust the attempts, and the service is still starting
nothing in the log explains it, which is why I pasted it";
    assert_eq!(kinds(prose), vec![SegmentKind::Prose]);
}

#[test]
fn a_whitespace_aligned_table_is_classified_as_a_table() {
    let aligned = "\
NAME             READY   STATUS    RESTARTS
api-7d9f          1/1    Running   0
worker-22c        0/1    Pending   4
indexer-91a       1/1    Running   0";
    assert_eq!(kinds(aligned), vec![SegmentKind::Table]);
}

#[test]
fn a_pipe_delimited_log_is_an_episode_not_a_fact() {
    // The documented precedence fix: these rows share a delimiter count with a
    // table, but a clock at the head of every line is the stronger signal, and
    // classifying it as a table would describe a log as if it were data.
    let piped = "\
2026-07-20T18:00:01Z | INFO  | starting retry loop
2026-07-20T18:00:02Z | WARN  | attempt 1 failed
2026-07-20T18:00:05Z | WARN  | attempt 2 failed
2026-07-20T18:00:11Z | ERROR | giving up";
    assert_eq!(kinds(piped), vec![SegmentKind::Log]);

    // …while a table *about* logs, whose rows open with the delimiter rather
    // than a clock, still reads as a table.
    let table = "\
| timestamp            | level | message   |
|----------------------|-------|-----------|
| 2026-07-20T18:00:01Z | INFO  | starting  |
| 2026-07-20T18:00:11Z | ERROR | giving up |";
    assert_eq!(kinds(table), vec![SegmentKind::Table]);
}

#[test]
fn a_space_padded_log_is_not_read_as_two_columns() {
    // The padding after a fixed-width level is exactly a whitespace column
    // break, which is why aligned-table detection is tried *after* the log
    // heuristics rather than before them.
    let padded = "\
2026-07-20 18:00:01 INFO  starting retry loop
2026-07-20 18:00:02 WARN  attempt 1 failed, backing off
2026-07-20 18:00:05 WARN  attempt 2 failed, backing off
2026-07-20 18:00:11 ERROR giving up after 3 attempts";
    assert_eq!(kinds(padded), vec![SegmentKind::Log]);
}

// ---- Distillation: column types ----

#[test]
fn column_types_name_units_and_holes() {
    let table = "\
| region | revenue    | growth | tier  | closed_at            |
|--------|------------|--------|-------|----------------------|
| emea   | $1,204.50  | 12%    | true  | 2026-07-20T18:00:00Z |
| apac   | $980.00    | -3.5%  | false |                      |
| namer  | $12,000.00 | 0%     | true  | 2026-07-19T09:00:00Z |";
    let distilled = distill_table(table);

    // A number whose unit lives in the cell is neither an int nor free text.
    assert!(distilled.contains("revenue (currency)"), "{distilled}");
    assert!(distilled.contains("growth (percent)"), "{distilled}");
    assert!(distilled.contains("tier (bool)"), "{distilled}");
    // A hole in a sampled column is worth a single character to report: the
    // model is reading five rows and inferring about thousands.
    assert!(distilled.contains("closed_at (timestamp?)"), "{distilled}");
}

#[test]
fn a_column_type_is_inferred_from_values_not_from_null_markers() {
    assert_eq!(infer_column_type(&["1", "2", "3"]), "int");
    assert_eq!(infer_column_type(&["1", "", "3"]), "int?");
    assert_eq!(infer_column_type(&["1.5", "NULL", "3.25"]), "float?");
    assert_eq!(infer_column_type(&["-", "n/a", ""]), "empty");
    // Thousands grouping is presentation, not a different value…
    assert_eq!(infer_column_type(&["1,234", "9"]), "int");
    // …but three cells that lost their delimiter are not one number.
    assert_eq!(infer_column_type(&["1,2,3"]), "text");
    assert_eq!(infer_column_type(&["$5", "(1,200.00)", "9.99 USD"]), "text");
    assert_eq!(infer_column_type(&["$5", "-$1,200.00"]), "currency");
}

#[test]
fn a_csv_distills_to_a_shape_a_sample_and_types() {
    let csv = "\
endpoint,calls,p99_ms
/v1/query,1841,210
/v1/resolve,92,1204
/v1/handshake,1841,8";
    let distilled = distill_table(csv);
    assert!(distilled.starts_with("[3 rows × 3 columns]"), "{distilled}");
    assert!(distilled.contains("calls (int)"), "{distilled}");
}

// ---- Distillation: timestamps (§F4) ----

#[test]
fn common_log_timestamps_normalize_to_the_f4_profile() {
    // Real logs almost never emit F4 already; before this, the temporal window
    // was effectively never populated.
    for (line, expected) in [
        ("2026-07-20T18:00:01Z boot", "2026-07-20T18:00:01Z"),
        ("2026-07-20 18:00:01 INFO boot", "2026-07-20T18:00:01Z"),
        ("2026-07-20T18:00:01 INFO boot", "2026-07-20T18:00:01Z"),
        ("2026/07/20 18:00:01 INFO boot", "2026-07-20T18:00:01Z"),
        // logback and .NET spell the fraction with a comma.
        (
            "2026-07-20 18:00:01,123 INFO boot",
            "2026-07-20T18:00:01.123Z",
        ),
        ("2026-07-20 18:00:01.5 INFO boot", "2026-07-20T18:00:01.5Z"),
        ("[2026-07-20 18:00:01] INFO boot", "2026-07-20T18:00:01Z"),
        ("2026-07-20 18:00:01 UTC INFO boot", "2026-07-20T18:00:01Z"),
        (
            "2026-07-20T18:00:01+00:00 INFO boot",
            "2026-07-20T18:00:01Z",
        ),
    ] {
        assert_eq!(
            leading_instant(Some(line)).as_deref(),
            Some(expected),
            "{line}"
        );
    }
}

#[test]
fn a_shape_that_cannot_be_spelled_in_f4_yields_no_window_at_all() {
    // Each of these is recognizably a timestamp — and each would need invented
    // information (a year, a day, an offset conversion) to become an instant.
    // The rule from #10 is absolute: emit F4 or emit nothing.
    for line in [
        "Jul 20 18:00:01 host sshd[123]: accepted", // syslog: no year
        "18:00:01.123 INFO boot",                   // bare clock: no day
        "[18:00:01] INFO boot",                     // ditto, bracketed
        "2026-07-20 INFO boot",                     // date, no clock
        "2026-07-20T18:00:01+02:00 INFO boot",      // an offset we won't convert
        "2026-07-20T18:00:01-05:00 INFO boot",      // ditto
        "2026-02-30 18:00:01 INFO boot",            // February has no 30th
        "2026-07-20 25:00:01 INFO boot",            // no such hour
        "last tuesday, around lunch",               // not a timestamp at all
    ] {
        assert_eq!(leading_instant(Some(line)), None, "{line}");
    }
}

#[test]
fn every_normalized_timestamp_survives_the_protocol_validator() {
    // The guard, stated as a property rather than a case list: whatever the
    // parser assembles, an invalid F4 string must never escape it.
    let corpus = [
        "2026-07-20 18:00:01 INFO x",
        "2026-13-01 18:00:01 INFO x",
        "2026-07-20 18:60:01 INFO x",
        "2026-07-20T18:00:01.000000001Z x",
        "0000-00-00 00:00:00 x",
        "9999-12-31 23:59:59 x",
        "Jul 20 18:00:01 x",
        "[worker-3] 2026-07-20 18:00:01 x",
    ];
    for line in corpus {
        if let Some(instant) = leading_instant(Some(line)) {
            assert!(
                contextgraph_types::is_protocol_timestamp(&instant),
                "{line} normalized to a non-F4 string: {instant}"
            );
        }
    }
}

#[test]
fn a_syslog_line_still_reads_as_a_log_even_though_it_carries_no_window() {
    let syslog = "\
Jul 20 18:00:01 gateway sshd[1201]: accepted publickey
Jul 20 18:00:04 gateway sshd[1201]: session opened
Jul 20 18:00:09 gateway kernel: WARN dropping oversized frame";
    assert_eq!(kinds(syslog), vec![SegmentKind::Log]);
    let (_, valid_from, valid_to) = distill_log(syslog);
    assert_eq!((valid_from, valid_to), (None, None));
}

#[test]
fn a_reverse_chronological_log_gets_an_ordered_window() {
    // `journalctl -r` renders newest-first; assigning the ends positionally
    // would hand a reader a window that runs backwards.
    let newest_first = "\
2026-07-20 18:00:11 ERROR giving up
2026-07-20 18:00:05 WARN attempt 2 failed
2026-07-20 18:00:01 INFO starting";
    let (_, valid_from, valid_to) = distill_log(newest_first);
    assert_eq!(valid_from.as_deref(), Some("2026-07-20T18:00:01Z"));
    assert_eq!(valid_to.as_deref(), Some("2026-07-20T18:00:11Z"));
}

// ---- Distillation: duplicate collapse ----

#[test]
fn a_spammy_retry_loop_collapses_before_salient_lines_are_chosen() {
    // 200 identical lines around one that differs. Without collapsing, the
    // repeated line wins every selection slot and the compact rendering says
    // nothing the first line didn't.
    let mut log = String::new();
    for _ in 0..100 {
        log.push_str("2026-07-20 18:00:01 INFO retrying connection to upstream\n");
    }
    log.push_str("2026-07-20 18:02:00 ERROR giving up after 100 attempts\n");
    for _ in 0..100 {
        log.push_str("2026-07-20 18:03:00 INFO shutting down worker pool\n");
    }
    let (distilled, _, _) = distill_log(log.trim_end());

    assert!(distilled.contains("… (×100)"), "{distilled}");
    assert!(distilled.contains("ERROR giving up"), "{distilled}");
    // The header still counts what the user actually pasted.
    assert!(distilled.starts_with("[201-line log, 1 error/warn line(s)]"));
    // And the saving is real: three distinct lines instead of two hundred.
    assert!(budget_tokens(&distilled) * 10 < budget_tokens(log.trim_end()));
}

#[test]
fn collapsing_never_lies_about_how_many_lines_were_elided() {
    // The elision counts quote *source* lines even though selection now works
    // over collapsed runs — otherwise a gap containing one 50-line run would
    // report "1 line elided" and the arithmetic would stop adding up.
    let mut log = String::from("2026-07-20 18:00:00 INFO start\n");
    for _ in 0..50 {
        log.push_str("2026-07-20 18:00:01 DEBUG polling\n");
    }
    for i in 0..30 {
        log.push_str(&format!("2026-07-20 18:00:30 DEBUG step {i}\n"));
    }
    log.push_str("2026-07-20 18:01:00 ERROR boom\n");
    log.push_str("2026-07-20 18:01:01 INFO done");
    let (distilled, _, _) = distill_log(&log);

    assert!(distilled.starts_with("[83-line log, 1 error/warn line(s)]"));
    // The gap swallowed the 50-line run plus 28 distinct lines.
    assert!(distilled.contains("… (78 lines elided) …"), "{distilled}");
}

// ---- Segmentation: path precision ----

#[test]
fn a_scheme_less_url_is_not_a_workspace_anchor() {
    // A dotted first segment is a hostname far more often than a directory, and
    // routing one to `anchors` sends the graph provider looking for a folder
    // named `example.com`.
    for not_a_path in ["example.com/path", "github.com/acme/repo", "www.foo.io/x"] {
        assert_eq!(kinds(not_a_path), vec![SegmentKind::Prose], "{not_a_path}");
    }
    // The refinement must not cost the ordinary cases.
    for path in ["./src/net", "src/net.rs", "docs/adr/0006-x.md", "net.rs"] {
        assert_eq!(kinds(path), vec![SegmentKind::PathRef], "{path}");
    }
}

// ---- Honesty invariants: the whole point ----

fn ingest(intent: &str, attachments: Vec<&str>) -> IngestBundle {
    ingest_paste(
        PasteIngest {
            intent: intent.to_string(),
            anchors: vec![],
            attachments: attachments.into_iter().map(String::from).collect(),
        },
        IngestConfig::default(),
    )
}

/// Every frame the provider can emit, in every representation it advertises.
async fn all_served_frames(bundle: &IngestBundle) -> Vec<ContextFrame> {
    let mut out = Vec::new();
    for representation in [
        Representation::Full,
        Representation::Compact,
        Representation::Reference,
    ] {
        let query = ContextQuery {
            representation_preferences: vec![representation],
            max_frames: u32::MAX,
            max_tokens: u32::MAX,
            ..bundle.query.clone()
        };
        out.extend(bundle.provider.query(&query).await.unwrap().frames);
    }
    out
}

#[tokio::test]
async fn every_emitted_frame_declares_an_honest_token_cost() {
    // §B3: `token_cost == ceil(utf8_len(content)/4)` for the exact bytes emitted
    // — checked across full, compact, AND reference, because each inlines
    // different content.
    let big_log = build_big_log(200);
    let deep_trace = deep_java_traceback(40);
    let bundle = ingest(
        "why does the retry loop give up",
        vec![
            &big_log,
            &deep_trace,
            SAMPLE_TABLE,
            SAMPLE_CSV,
            SAMPLE_CODE,
            "a short note about the bug",
        ],
    );
    let frames = all_served_frames(&bundle).await;
    assert!(!frames.is_empty());
    for frame in &frames {
        assert!(
            frame.declares_honest_token_cost(),
            "frame {} ({:?}) lied about cost: declared {}, canonical {}",
            frame.id,
            frame.representation,
            frame.token_cost,
            frame.expected_inline_token_cost(),
        );
    }
}

#[tokio::test]
async fn every_emitted_frame_satisfies_its_representation_invariants() {
    let big_log = build_big_log(200);
    let deep_trace = deep_java_traceback(40);
    let bundle = ingest(
        "fix it",
        vec![
            &big_log,
            &deep_trace,
            SAMPLE_TABLE,
            SAMPLE_CSV,
            SAMPLE_CODE,
            "note",
        ],
    );
    for frame in all_served_frames(&bundle).await {
        frame
            .representation_invariants()
            .unwrap_or_else(|e| panic!("frame {} invalid: {e}", frame.id));
        assert!(frame.has_valid_score());
        assert!(frame.has_valid_temporal_fields());
        // Pasted evidence must never masquerade as re-readable file provenance.
        assert!(frame.provenance_with_unusable_digests().is_empty());
        assert!(frame.provenance.iter().all(|p| p.kind == "derivation"));
    }
}

#[tokio::test]
async fn a_compact_frame_actually_shrinks_a_large_log_and_stays_rehydratable() {
    let big_log = build_big_log(300);
    let bundle = ingest("debug", vec![&big_log]);

    let compact = one_frame(&bundle, Representation::Compact).await;
    let full = one_frame(&bundle, Representation::Full).await;

    assert_eq!(compact.representation, Representation::Compact);
    assert!(
        compact.token_cost < full.token_cost,
        "compact ({}) must cost less than full ({})",
        compact.token_cost,
        full.token_cost
    );
    // The canonical hash pins the full source; the full frame's inline hash is
    // exactly that canonical hash — so a `[full]` re-query is verifiably the
    // rehydration of the same artifact.
    assert_eq!(
        compact.canonical_content_hash.as_deref(),
        full.content_digest.as_deref()
    );
    assert!(compact.canonical_token_cost.unwrap() == full.token_cost);
    assert_eq!(compact.content_fidelity, Some(ContentFidelity::Summarized));
    // The compact rendering keeps the ERROR line the log is about.
    assert!(compact.content.as_deref().unwrap().contains("ERROR"));
}

#[tokio::test]
async fn a_pasted_traceback_becomes_a_compact_episode_frame() {
    let deep = deep_java_traceback(40);
    let bundle = ingest("why did it blow up", vec![&deep]);
    let compact = one_frame(&bundle, Representation::Compact).await;

    // A trace is an event capture, not a standing fact.
    assert_eq!(compact.kind, contextgraph_types::FrameKind::Episode);
    assert_eq!(
        compact.citation_label.as_deref(),
        Some("pasted stack trace")
    );
    assert!(compact.title.starts_with("stack trace ·"));
    // Distilled, honest about it, and honest about what it cost.
    assert_eq!(compact.content_fidelity, Some(ContentFidelity::Summarized));
    assert_eq!(
        compact.transform.as_ref().map(|t| t.method.as_str()),
        Some("stack_frame_head")
    );
    assert!(compact.declares_honest_token_cost());
    assert!(compact.token_cost < compact.canonical_token_cost.unwrap());
    // The exception line is what a reader needs first, so it is what survives.
    let content = compact.content.as_deref().unwrap();
    assert!(content.contains("java.lang.IllegalStateException: pool exhausted"));
    compact.representation_invariants().unwrap();
}

#[tokio::test]
async fn a_timestamped_trace_is_bounded_to_the_instant_it_happened() {
    // A traceback is one moment, not a span — so both ends of the window are
    // the same instant, and both are F4 or neither is set.
    let trace = format!("2026-07-20 18:00:01 ERROR {JAVA_TRACEBACK}");
    let bundle = ingest("boom", vec![&trace]);
    let frame = one_frame(&bundle, Representation::Full).await;
    assert_eq!(frame.valid_from.as_deref(), Some("2026-07-20T18:00:01Z"));
    assert_eq!(frame.valid_from, frame.valid_to);
    assert!(frame.has_valid_temporal_fields());
}

#[tokio::test]
async fn a_real_world_log_now_carries_the_temporal_window_it_always_had() {
    // Before normalization this window was empty for every log that did not
    // already spell its clock in F4 — which is nearly all of them.
    let log: String = (0..120)
        .map(|i| {
            format!(
                "2026-07-20 18:{:02}:{:02},500 INFO attempt {i}\n",
                i / 60,
                i % 60
            )
        })
        .collect();
    let bundle = ingest("why", vec![log.trim_end()]);
    let frame = one_frame(&bundle, Representation::Compact).await;
    assert_eq!(
        frame.valid_from.as_deref(),
        Some("2026-07-20T18:00:00.500Z")
    );
    assert_eq!(frame.valid_to.as_deref(), Some("2026-07-20T18:01:59.500Z"));
    assert!(frame.has_valid_temporal_fields());
}

#[tokio::test]
async fn a_small_paste_is_served_verbatim_and_exact() {
    let bundle = ingest("ctx", vec!["one line, nothing to compact"]);
    let compact = one_frame(&bundle, Representation::Compact).await;
    // Nothing to distill: the "compact" rendering is the exact source.
    assert_eq!(compact.content_fidelity, Some(ContentFidelity::Exact));
    assert_eq!(
        compact.content.as_deref(),
        Some("one line, nothing to compact")
    );
    assert!(compact.declares_honest_token_cost());
    compact.representation_invariants().unwrap();
}

// ---- Intent and anchors ----

#[test]
fn intent_passes_through_verbatim_as_the_goal() {
    let intent = "why does the retry loop give up — DON'T paraphrase this: `foo->bar`";
    let bundle = ingest(intent, vec!["2026-01-01 ERROR x"]);
    assert_eq!(bundle.query.goal, intent, "intent must never be rewritten");
}

#[test]
fn a_directory_reference_becomes_an_anchor_with_no_frame() {
    let bundle = ingest_paste(
        PasteIngest {
            intent: "fix".into(),
            anchors: vec!["file:///repo/src/lib.rs".into()],
            attachments: vec!["./src/net".into()],
        },
        IngestConfig::default(),
    );
    assert!(bundle.query.anchors.contains(&"./src/net".to_string()));
    assert!(
        bundle
            .query
            .anchors
            .contains(&"file:///repo/src/lib.rs".to_string())
    );
    assert!(bundle.provider.is_empty(), "a path yields no frame");
    assert!(matches!(
        bundle.report[0].became,
        SegmentOutcome::Anchor { .. }
    ));
}

// ---- Content addressing / dedup ----

#[test]
fn the_same_paste_content_addresses_to_the_same_id_and_dedups() {
    let log = build_big_log(50);
    let once = ingest("g", vec![&log]);
    let twice = ingest("g", vec![&log, &log]);

    // Identical content ⇒ identical frame id across independent ingests.
    let id_once = &once.provider.artifacts[0].id;
    let id_twice = &twice.provider.artifacts[0].id;
    assert_eq!(id_once, id_twice);
    // The second identical attachment is collapsed, not duplicated.
    assert_eq!(twice.provider.len(), 1);
    assert!(
        twice
            .report
            .iter()
            .any(|r| matches!(r.became, SegmentOutcome::Duplicate { .. }))
    );
}

// ---- Provider contract: query budget & frame cap ----

#[tokio::test]
async fn query_respects_max_frames_and_max_tokens() {
    let bundle = ingest("g", vec![SAMPLE_TABLE, SAMPLE_CODE, "note one", "note two"]);
    assert!(bundle.provider.len() >= 3);

    // Cap to two frames.
    let capped = ContextQuery {
        max_frames: 2,
        max_tokens: u32::MAX,
        ..bundle.query.clone()
    };
    let result = bundle.provider.query(&capped).await.unwrap();
    assert!(result.respects_frame_limit(2));
    assert_eq!(result.frames.len(), 2);
    assert!(result.truncated);

    // Cap to a tiny token budget: the sum stays under it (§B1).
    let tight = ContextQuery {
        max_frames: u32::MAX,
        max_tokens: 3,
        ..bundle.query.clone()
    };
    let result = bundle.provider.query(&tight).await.unwrap();
    assert!(result.respects_budget(3));
    assert!(result.frames_with_dishonest_cost().is_empty());
}

#[tokio::test]
async fn the_default_bundle_query_returns_every_frame_within_budget() {
    let bundle = ingest("g", vec![SAMPLE_TABLE, SAMPLE_CODE, "a note"]);
    let result = bundle.provider.query(&bundle.query).await.unwrap();
    assert_eq!(result.frames.len(), bundle.provider.len());
    assert!(result.respects_budget(bundle.query.max_tokens));
    assert!(!result.truncated);
}

// ---- Provider contract: verify ----

#[tokio::test]
async fn verify_vouches_for_a_held_frame_and_rejects_a_tampered_or_unknown_one() {
    let log = build_big_log(120);
    let bundle = ingest("g", vec![&log]);
    let provider = &bundle.provider;

    // Take the compact frame the host would actually hold.
    let compact = one_frame(&bundle, Representation::Compact).await;
    let held = compact.identity(provider.id());
    assert!(held.is_verifiable());

    let request = VerifyRequest::new(vec![held.clone()]);
    let response = provider.verify(&request).await.unwrap();
    assert_eq!(response.verdict_for(&held), Some(&Verdict::Valid));

    // A wrong digest on a known id ⇒ stale, carrying the current digest.
    let tampered = FrameId::new(provider.id(), &compact.id, Some("sha256:dead".into()));
    let response = provider
        .verify(&VerifyRequest::new(vec![tampered.clone()]))
        .await
        .unwrap();
    assert!(matches!(
        response.verdict_for(&tampered),
        Some(Verdict::Stale { .. })
    ));

    // An unknown id ⇒ gone (the store is authoritative-complete).
    let ghost = FrameId::new(provider.id(), "frm_notours", Some("sha256:beef".into()));
    let response = provider
        .verify(&VerifyRequest::new(vec![ghost.clone()]))
        .await
        .unwrap();
    assert_eq!(response.verdict_for(&ghost), Some(&Verdict::Gone));
}

// ---- End-to-end honesty of the realistic example ----

#[tokio::test]
async fn the_motivating_example_produces_a_clean_bundle() {
    // The exact scenario from the ADR: a log, a table, a directory, and intent.
    let log = build_big_log(75);
    let bundle = ingest_paste(
        PasteIngest {
            intent: "figure out why the retry loop gives up".into(),
            anchors: vec![],
            attachments: vec![log.clone(), SAMPLE_TABLE.into(), "./src/net".into()],
        },
        IngestConfig::default(),
    );

    // Intent verbatim, directory became an anchor, two evidence frames.
    assert_eq!(bundle.query.goal, "figure out why the retry loop gives up");
    assert!(bundle.query.anchors.contains(&"./src/net".to_string()));
    assert_eq!(bundle.provider.len(), 2);

    // The provider is local-only and needs no consent.
    assert!(!bundle.provider.info().data_flow.egress);
    assert!(bundle.provider.info().data_flow.scopes_consistent());
    assert!(bundle.provider.capabilities().representations_consistent());

    // Everything it serves is honest and composes.
    let result = bundle.provider.query(&bundle.query).await.unwrap();
    assert!(result.respects_budget(bundle.query.max_tokens));
    assert!(result.frames_with_dishonest_cost().is_empty());
    let composed =
        crate::compose::compose_context(result.frames.iter().map(|f| (bundle.provider.id(), f)));
    assert!(composed.contains("<frame"));
}

// ---- fixtures ----

const SAMPLE_TABLE: &str = "\
| id | name  | score | active |
|----|-------|-------|--------|
| 1  | alice | 0.91  | true   |
| 2  | bob   | 0.42  | false  |
| 3  | carol | 0.77  | true   |";

const SAMPLE_CODE: &str =
    "```rust\nfn retry() {\n    for _ in 0..3 {\n        attempt();\n    }\n}\n```";

const SAMPLE_CSV: &str = "\
endpoint,calls,p99_ms,errors
/v1/query,1841,210,3
/v1/resolve,92,1204,41
/v1/handshake,1841,8,0";

const PYTHON_TRACEBACK: &str = "\
Traceback (most recent call last):
  File \"app.py\", line 42, in main
    run()
  File \"app.py\", line 17, in run
    raise ValueError(\"backoff exhausted\")
ValueError: backoff exhausted";

const JAVA_TRACEBACK: &str = "\
java.lang.IllegalStateException: connection pool exhausted
\tat com.acme.pool.Pool.borrow(Pool.java:118)
\tat com.acme.net.Client.send(Client.java:64)
\tat com.acme.net.Retry.attempt(Retry.java:31)";

const RUST_PANIC: &str = "\
thread 'main' panicked at src/net/retry.rs:88:9:
attempt to subtract with overflow
stack backtrace:
   0: rust_begin_unwind
   1: core::panicking::panic_fmt
   2: contextgraph_host::net::retry::backoff";

/// A Java-shaped traceback with `frames` frames, each nameable in an assertion.
fn deep_java_traceback(frames: usize) -> String {
    let mut out = String::from("java.lang.IllegalStateException: pool exhausted\n");
    for i in 0..frames {
        out.push_str(&format!("\tat com.acme.frame{i:02}.call(Frame.java:{i})\n"));
    }
    out.trim_end().to_string()
}

/// A Python-shaped traceback: the exception is *last*, and every frame carries a
/// source line underneath it.
fn deep_python_traceback(frames: usize) -> String {
    let mut out = String::from("Traceback (most recent call last):\n");
    for i in 0..frames {
        out.push_str(&format!("  File \"app.py\", line {i}, in step{i}\n"));
        out.push_str(&format!("    step{}()\n", i + 1));
    }
    out.push_str("ValueError: backoff exhausted");
    out
}

fn build_big_log(lines: usize) -> String {
    let mut out = String::new();
    for i in 0..lines {
        let level = if i == lines / 2 {
            "ERROR"
        } else if i % 7 == 0 {
            "WARN "
        } else {
            "INFO "
        };
        out.push_str(&format!(
            "2026-07-20T18:{:02}:{:02}Z {level} attempt {i} of the retry loop\n",
            i / 60 % 60,
            i % 60
        ));
    }
    out.trim_end().to_string()
}

async fn one_frame(bundle: &IngestBundle, representation: Representation) -> ContextFrame {
    let query = ContextQuery {
        representation_preferences: vec![representation],
        max_frames: 1,
        max_tokens: u32::MAX,
        ..bundle.query.clone()
    };
    bundle
        .provider
        .query(&query)
        .await
        .unwrap()
        .frames
        .into_iter()
        .next()
        .expect("at least one frame")
}
