//! Prompt ingestion, end to end ([ADR 0006] / `docs/prompt-ingestion.md`).
//!
//! Run it:
//!
//! ```text
//! cargo run -p contextgraph-host --example ingest_paste
//! cargo run -p contextgraph-host --example ingest_paste -- path/to/paste.txt
//! ```
//!
//! It walks the whole path a paste takes:
//!
//! 1. decompose the paste into intent + evidence (what a host UI hands over);
//! 2. print the [`SegmentReport`] pills — the "visible and correctable"
//!    guarantee, made concrete: nothing is transformed invisibly;
//! 3. register the returned [`IngestProvider`] in an ordinary [`Host`] and fan
//!    the query out, so the paste goes through the same consent gate, timeout,
//!    and budget audit as any other provider;
//! 4. print the composed context block, the usage report, and the proof that a
//!    second identical turn composes to the same bytes;
//! 5. pull one frame back in `full` — the rehydration path behind `resolve`.
//!
//! Every number printed is computed, not narrated: the token counts are the
//! frames' own declared `token_cost`, checked against §B3 before printing.
//!
//! [ADR 0006]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0006-prompt-ingestion-as-a-local-provider.md

use contextgraph_host::{
    ContextProvider, Host, IngestConfig, PasteIngest, SegmentOutcome, ingest_paste,
};
use contextgraph_types::{ContextQuery, Representation, budget_tokens};

#[tokio::main]
async fn main() {
    // A paste is intent plus evidence. A real host fills these from the
    // composer: `intent` is what the user typed, `attachments` are the blobs
    // they dropped in. Passing a file path swaps the evidence for your own.
    let paste = match std::env::args().nth(1) {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
            PasteIngest::new("figure out why the retry loop gives up", text)
        }
        None => sample_paste(),
    };

    let source_tokens = budget_tokens(&paste.attachments.join("\n"));
    println!("== the paste ==");
    println!("intent      : {}", paste.intent);
    println!("attachments : {}", paste.attachments.len());
    println!("raw cost    : {source_tokens} tokens if pasted verbatim\n");

    let bundle = ingest_paste(paste, IngestConfig::default());

    // ---- 1. The pills ----------------------------------------------------
    // A host renders these as correctable chips above the composer. Printing
    // them is the whole "never transform input invisibly" guarantee.
    println!("== segment report ==");
    for segment in &bundle.report {
        let became = match &segment.became {
            SegmentOutcome::Anchor { uri } => format!("→ anchor {uri}"),
            SegmentOutcome::Frame {
                id,
                representation,
                inline_tokens,
                source_tokens,
            } => format!(
                "→ frame {id} as {representation:?}, {inline_tokens} of {source_tokens} tokens inlined"
            ),
            SegmentOutcome::Duplicate { id } => format!("→ deduplicated into {id}"),
        };
        println!("  [{:?}] {} {became}", segment.kind, segment.summary);
    }

    // Intent is the one thing that is never mediated — it left the paste
    // byte-for-byte and is now the query's goal.
    println!("\n== the query ==");
    println!("goal        : {}", bundle.query.goal);
    println!("anchors     : {:?}", bundle.query.anchors);
    println!("budget      : {} tokens", bundle.query.max_tokens);
    println!(
        "prefers     : {:?}",
        bundle.query.representation_preferences
    );

    // ---- 2. An ordinary provider in an ordinary host ---------------------
    let query = bundle.query.clone();
    let provider_id = bundle.provider.id().to_string();
    let mut host = Host::new();
    host.register(Box::new(bundle.provider));

    let fanout = host.query_all(&query).await;
    assert_eq!(fanout.failures().count(), 0, "local provider cannot fail");
    // Local-only, so no consent receipt was needed; and nothing was dropped for
    // lying about cost, which is the audit every provider faces.
    assert_eq!(fanout.budget_liars().count(), 0);

    println!("\n== composed context block ==");
    let composed = fanout.compose();
    print!("{composed}");

    let served = fanout.total_accepted_tokens();
    println!("\n== accounting ==");
    println!("frames served : {}", fanout.accepted_frames().count());
    println!("tokens served : {served} (of a {source_tokens}-token paste)");
    if source_tokens > 0 {
        println!(
            "compaction    : {:.0}% of the raw paste",
            100.0 * served as f64 / source_tokens as f64
        );
    }
    for frame in fanout.accepted_frames() {
        // §B3 is not taken on trust here: the printed cost is re-derived.
        assert!(
            frame.declares_honest_token_cost(),
            "frame {} lied about its cost",
            frame.id
        );
    }

    let report = fanout.usage_report(&query, "2026-07-20T18:05:00Z");
    assert!(report.is_consistent(), "usage totals must re-sum");
    println!(
        "usage report  : {} tokens across {} provider(s), arithmetic consistent",
        report.budget_consumed,
        report.providers.len()
    );

    // ---- 3. Byte stability -----------------------------------------------
    // The same paste, a second turn: identical bytes, so the prompt prefix (and
    // its cache) survives.
    let again = host.query_all(&query).await.compose();
    assert_eq!(composed, again);
    println!("re-compose    : byte-identical ({} bytes)", again.len());

    // ---- 4. Pulling the full bytes back ----------------------------------
    // The `resolve` capability, exercised: re-query preferring `full` and the
    // provider rehydrates the exact source from its content-addressed store.
    let full = ContextQuery {
        representation_preferences: vec![Representation::Full],
        max_frames: 1,
        max_tokens: u32::MAX,
        ..query.clone()
    };
    if let Some(frame) = host
        .provider(&provider_id)
        .expect("the ingest provider is registered")
        .query(&full)
        .await
        .expect("local query cannot fail")
        .frames
        .first()
    {
        println!("\n== pulling [full] on the top-ranked frame ==");
        println!("id            : {}", frame.id);
        println!(
            "cost          : {} tokens full, vs {} compact",
            frame.token_cost,
            fanout
                .accepted_frames()
                .find(|f| f.id == frame.id)
                .map_or(0, |f| f.token_cost)
        );
        println!("fidelity      : {:?}", frame.content_fidelity);
        println!(
            "digest        : {}",
            frame.content_digest.as_deref().unwrap_or("-")
        );
        let content = frame.content.as_deref().unwrap_or("");
        println!("--- first lines of the rehydrated source ---");
        for line in content.lines().take(4) {
            println!("{line}");
        }
        println!("… ({} lines total)", content.lines().count());
    }
}

/// The ADR's motivating paste: a noisy log, a table, a traceback, a directory
/// reference, and the actual ask — four different things wearing one trenchcoat.
fn sample_paste() -> PasteIngest {
    let mut log = String::new();
    for i in 0..90 {
        let level = if i == 61 {
            "ERROR"
        } else if i % 29 == 0 {
            "WARN "
        } else {
            "INFO "
        };
        // Deliberately *not* in the F4 profile: this is what real logs look
        // like, and normalizing it is what populates the frame's temporal
        // window.
        log.push_str(&format!(
            "2026-07-20 18:{:02}:{:02},250 {level} retry attempt {i} for /v1/resolve\n",
            i / 60,
            i % 60
        ));
    }
    // A retry loop that repeats itself: the duplicate-collapse case.
    for _ in 0..25 {
        log.push_str("2026-07-20 18:01:31,250 WARN  upstream still draining, backing off\n");
    }

    // A CSV with a currency column, a percent column, and one hole — the shapes
    // the column summary exists to report. Long enough that summarizing it
    // genuinely pays; the distiller declines to compact a table that would cost
    // as much in summary as it does in full.
    let mut table = String::from("endpoint,calls,p99_ms,error_rate,cost\n");
    table.push_str("/v1/query,1841,210,0.2%,$18.41\n");
    table.push_str("/v1/resolve,92,1204,44.6%,$0.92\n");
    for i in 0..30 {
        table.push_str(&format!(
            "/v1/route{i:02},{},{},0.{}%,${}.{:02}\n",
            100 + i * 7,
            9 + i,
            i % 9,
            i,
            i % 100
        ));
    }
    // A missing cell in the last row: the hole that makes `cost` nullable.
    table.push_str("/v1/debug,3,7,0%,");

    let trace = "\
java.lang.IllegalStateException: connection pool exhausted after 3 attempts
\tat com.acme.pool.Pool.borrow(Pool.java:118)
\tat com.acme.pool.Pool.acquire(Pool.java:74)
\tat com.acme.net.Client.connect(Client.java:91)
\tat com.acme.net.Client.send(Client.java:64)
\tat com.acme.net.Retry.attempt(Retry.java:31)
\tat com.acme.net.Retry.run(Retry.java:19)
\tat com.acme.api.ResolveHandler.handle(ResolveHandler.java:88)
\tat com.acme.api.Router.dispatch(Router.java:214)
\tat com.acme.api.Router.route(Router.java:150)
\tat com.acme.server.Worker.serve(Worker.java:77)
\tat com.acme.server.Worker.loop(Worker.java:41)
\tat com.acme.server.Worker.start(Worker.java:22)
\tat java.base/java.lang.Thread.run(Thread.java:840)";

    PasteIngest {
        intent: "figure out why the retry loop gives up".to_string(),
        anchors: vec![],
        attachments: vec![
            log.trim_end().to_string(),
            table,
            trace.to_string(),
            "./src/net".to_string(),
            "the backoff may be too aggressive for a service that slow-starts".to_string(),
        ],
    }
}
