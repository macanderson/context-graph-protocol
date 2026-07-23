//! Prompt ingestion as a local provider ([ADR 0006]).
//!
//! The one input CGP never disciplined is the largest: the text a user pastes
//! into a prompt. A realistic turn mixes four different things under one blob —
//! a log, a table, a directory reference, and the actual ask — and only the last
//! is *intent*. Pasted whole, it is re-sent verbatim every turn (no cache, no
//! dedup), its cost is never accounted, nothing is content-addressed, and the
//! model is handed material it must itself decide is mostly irrelevant.
//!
//! This module is the ingestion-side dual of [`compose_context`](crate::compose):
//! host-side reference behavior, **not** wire protocol. It turns a paste into an
//! ordinary [`ContextProvider`]:
//!
//! - **intent** passes through *verbatim* as [`ContextQuery::goal`] — the one
//!   thing the mechanism must never rewrite;
//! - **directory references** become [`ContextQuery::anchors`] (zero tokens; the
//!   graph provider resolves them better than pasted text could);
//! - **evidence** (logs, tables, code, notes) becomes content-addressed frames,
//!   served [`compact`](Representation::Compact) by default with the full bytes
//!   retrievable losslessly by re-querying for [`full`](Representation::Full).
//!
//! The guarantee is not "zero wasted tokens" — relevance is only knowable
//! downstream. It is **bounded default cost with lossless retrieval**: the model
//! sees a distilled, budgeted rendering; the full bytes stay content-addressed
//! and pullable. Every emitted frame is honest by construction — `token_cost`
//! and the inline `content_digest` are recomputed for the exact representation
//! served (§B3), and every frame satisfies its
//! [`representation_invariants`](ContextFrame::representation_invariants).
//!
//! [ADR 0006]: https://github.com/macanderson/context-graph-protocol/blob/main/docs/adr/0006-prompt-ingestion-as-a-local-provider.md

use std::collections::{BTreeSet, HashSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use contextgraph_types::{
    Capabilities, ContentFidelity, ContentRef, ContextFrame, ContextQuery, ContextQueryResult,
    DataFlow, EgressScope, FrameKind, FrameVerdict, InlineContentRequirement, Provenance,
    ProviderInfo, QueryCapability, Representation, Transform, Verdict, VerifyRequest,
    VerifyResponse, budget_tokens, is_protocol_timestamp,
};

use crate::error::HostError;
use crate::provider::{ContextProvider, frame_kind_name};

/// Below this canonical cost a compact rendering is not worth producing — the
/// artifact is served verbatim (fidelity `exact`). ~256 source bytes.
const COMPACT_MIN_TOKENS: u32 = 64;
/// Lines of context kept on each side of an alert line when distilling a log.
const LOG_CONTEXT: usize = 2;
/// Head/tail lines kept when a log has no alert lines to anchor on.
const LOG_HEAD: usize = 8;
const LOG_TAIL: usize = 4;
/// Data rows shown in a distilled table sample.
const TABLE_SAMPLE: usize = 5;
/// Head/tail lines kept when distilling an oversized code block.
const CODE_HEAD: usize = 20;
const CODE_TAIL: usize = 8;
/// Version stamped into every [`Transform`] this module emits, so a consumer can
/// tell which distiller produced an inline rendering.
const TRANSFORM_VERSION: &str = "1";
/// The transform implementation identity.
const TRANSFORM_IMPL: &str = "contextgraph-host/ingest";
/// Default provider id / consent key for an ingested paste.
pub const DEFAULT_PROVIDER_ID: &str = "prompt-ingest";

// ---------------------------------------------------------------------------
// Content addressing
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        // `sha256:<64 lowercase hex>` — lowercase is mandated by §F5, and the
        // whole dedup/cache story depends on the same bytes hashing identically.
        hex.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        hex.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    hex
}

/// A protocol content digest over `s`: `sha256:<64 lowercase hex>` (§F5).
fn sha256_digest(s: &str) -> String {
    format!("sha256:{}", sha256_hex(s.as_bytes()))
}

/// The 12-hex-character short form used to build a stable, content-addressed
/// frame id. Same bytes ⇒ same id ⇒ one deduplicated frame.
fn short_hash(digest: &str) -> &str {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    &hex[..hex.len().min(12)]
}

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

/// A user's paste, decomposed into the three things it actually is.
///
/// `intent` is sacrosanct — it becomes [`ContextQuery::goal`] byte-for-byte and
/// is never mediated. `anchors` are focal URIs the host already knows (open
/// files, mentioned symbols); path references discovered inside `attachments`
/// are appended to them. `attachments` are the pasted evidence blobs, each
/// segmented and content-addressed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteIngest {
    /// The user's own words — passed through verbatim as the query goal.
    pub intent: String,
    /// Focal URIs the host already considers relevant.
    #[serde(default)]
    pub anchors: Vec<String>,
    /// Raw pasted evidence blobs (a log, a table, a code block, …).
    #[serde(default)]
    pub attachments: Vec<String>,
}

impl PasteIngest {
    /// A paste with just intent and one evidence blob — the common case.
    pub fn new(intent: impl Into<String>, attachment: impl Into<String>) -> Self {
        Self {
            intent: intent.into(),
            anchors: Vec::new(),
            attachments: vec![attachment.into()],
        }
    }
}

/// Knobs for [`ingest_paste`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestConfig {
    /// The provider's host-facing id and consent key.
    pub provider_id: String,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            provider_id: DEFAULT_PROVIDER_ID.to_string(),
        }
    }
}

/// The classification a segment received. Deterministic and heuristic — the
/// same posture as `validate.rs`, reproducible from the bytes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    /// A log or trace capture → an `episode` frame.
    Log,
    /// Delimited tabular data → a `fact` frame.
    Table,
    /// A source-code block → a `snippet` frame.
    Code,
    /// Free text the user attached as evidence → a `doc` frame.
    Prose,
    /// A filesystem path or directory reference → a query anchor, not a frame.
    PathRef,
}

impl SegmentKind {
    fn frame_kind(self) -> Option<FrameKind> {
        match self {
            SegmentKind::Log => Some(FrameKind::Episode),
            SegmentKind::Table => Some(FrameKind::Fact),
            SegmentKind::Code => Some(FrameKind::Snippet),
            SegmentKind::Prose => Some(FrameKind::Doc),
            SegmentKind::PathRef => None,
        }
    }

    fn citation_label(self) -> &'static str {
        match self {
            SegmentKind::Log => "pasted log",
            SegmentKind::Table => "pasted table",
            SegmentKind::Code => "pasted code",
            SegmentKind::Prose => "pasted note",
            SegmentKind::PathRef => "pasted path",
        }
    }

    /// A static per-kind relevance prior. Ranking is provider-private; this is a
    /// defensible default, always in `[0, 1]` (§F1).
    fn score(self) -> f32 {
        match self {
            SegmentKind::Log => 0.8,
            SegmentKind::Code => 0.75,
            SegmentKind::Table => 0.7,
            SegmentKind::Prose => 0.5,
            SegmentKind::PathRef => 0.0,
        }
    }
}

/// What one classified segment became — the payload of a [`SegmentReport`], and
/// the "visible and correctable" surface a host UI renders as a pill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SegmentOutcome {
    /// Routed to [`ContextQuery::anchors`] — zero content, zero tokens.
    Anchor { uri: String },
    /// Turned into a content-addressed frame.
    Frame {
        id: String,
        /// The representation the default query serves it as.
        representation: Representation,
        /// Budget cost of the inline (distilled) rendering the model sees.
        inline_tokens: u32,
        /// Budget cost of the full source — what the compact rendering saved.
        source_tokens: u32,
    },
    /// Byte-identical to an earlier segment; collapsed to one frame.
    Duplicate { id: String },
}

/// One line of the ingestion report: what a segment was classified as and what
/// it became. Surfaced so a host never transforms input invisibly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentReport {
    pub kind: SegmentKind,
    /// A one-line human summary for the UI pill (e.g. `"log · 75 lines"`).
    pub summary: String,
    pub became: SegmentOutcome,
}

/// The result of [`ingest_paste`]: a ready-to-fan-out query, the local provider
/// that answers it, and the classification report.
pub struct IngestBundle {
    /// `goal` = the intent verbatim; `anchors` include discovered paths;
    /// `representation_preferences` prefer compact, then full.
    pub query: ContextQuery,
    /// The local, egress-free provider serving the pasted evidence.
    pub provider: IngestProvider,
    /// One entry per segment, in paste order.
    pub report: Vec<SegmentReport>,
}

/// Turn a decomposed paste into a query + a local provider + a report.
///
/// Intent is preserved verbatim; evidence is segmented, content-addressed, and
/// deduplicated by content. The returned [`IngestBundle::provider`] plugs into a
/// [`Host`](crate::Host) like any other provider.
pub fn ingest_paste(input: PasteIngest, config: IngestConfig) -> IngestBundle {
    let PasteIngest {
        intent,
        mut anchors,
        attachments,
    } = input;

    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut report: Vec<SegmentReport> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for attachment in &attachments {
        for block in split_blocks(attachment) {
            let text = block.text();
            if text.trim().is_empty() {
                continue;
            }
            let kind = classify(&block);

            if kind == SegmentKind::PathRef {
                let uri = text.trim().to_string();
                report.push(SegmentReport {
                    kind,
                    summary: format!("anchor · {uri}"),
                    became: SegmentOutcome::Anchor { uri: uri.clone() },
                });
                if !anchors.contains(&uri) {
                    anchors.push(uri);
                }
                continue;
            }

            let artifact = Artifact::build(kind, text);
            if seen.contains(&artifact.id) {
                report.push(SegmentReport {
                    kind,
                    summary: format!("duplicate · deduplicated to {}", artifact.id),
                    became: SegmentOutcome::Duplicate { id: artifact.id },
                });
                continue;
            }
            seen.insert(artifact.id.clone());
            report.push(SegmentReport {
                kind,
                summary: artifact.summary.clone(),
                became: SegmentOutcome::Frame {
                    id: artifact.id.clone(),
                    representation: Representation::Compact,
                    inline_tokens: budget_tokens(&artifact.inline_content),
                    source_tokens: budget_tokens(&artifact.full_content),
                },
            });
            artifacts.push(artifact);
        }
    }

    // Canonical id order: deterministic query output, stable across runs.
    artifacts.sort_by(|a, b| a.id.cmp(&b.id));

    let provider = IngestProvider::new(config.provider_id, artifacts);
    let query = ContextQuery {
        goal: intent,
        query_text: None,
        embedding: None,
        kinds: Vec::new(),
        anchors,
        max_frames: provider.artifacts.len() as u32,
        max_tokens: provider.default_budget_tokens(),
        as_of: None,
        representation_preferences: vec![Representation::Compact, Representation::Full],
    };

    IngestBundle {
        query,
        provider,
        report,
    }
}

// ---------------------------------------------------------------------------
// Segmentation
// ---------------------------------------------------------------------------

/// A raw block of a paste before it is classified: a run of non-blank lines, or
/// the body of a fenced code region.
struct RawBlock {
    lines: Vec<String>,
    fenced_code: bool,
}

impl RawBlock {
    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// Push `buf` as a block (moving its lines out) if it is non-empty.
fn flush_block(buf: &mut Vec<String>, fenced_code: bool, blocks: &mut Vec<RawBlock>) {
    if !buf.is_empty() {
        blocks.push(RawBlock {
            lines: std::mem::take(buf),
            fenced_code,
        });
    }
}

/// Split a paste into blocks: fenced ```code``` regions are atomic; everything
/// else is grouped into paragraphs separated by blank lines.
fn split_blocks(text: &str) -> Vec<RawBlock> {
    let mut blocks = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut fence: Vec<String> = Vec::new();
    let mut in_fence = false;

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_fence {
                flush_block(&mut fence, true, &mut blocks);
                in_fence = false;
            } else {
                flush_block(&mut current, false, &mut blocks);
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            fence.push(line.to_string());
        } else if line.trim().is_empty() {
            flush_block(&mut current, false, &mut blocks);
        } else {
            current.push(line.to_string());
        }
    }
    // Unterminated fence: keep what we captured rather than dropping it.
    flush_block(&mut fence, in_fence, &mut blocks);
    flush_block(&mut current, false, &mut blocks);
    blocks
}

/// Classify a block. Order matters: the most specific, least-ambiguous shapes
/// are tested first.
fn classify(block: &RawBlock) -> SegmentKind {
    if block.fenced_code {
        return SegmentKind::Code;
    }
    let lines: Vec<&str> = block.lines.iter().map(String::as_str).collect();
    if lines.len() == 1 && looks_like_path(lines[0]) {
        return SegmentKind::PathRef;
    }
    if looks_like_table(&lines) {
        return SegmentKind::Table;
    }
    if looks_like_log(&lines) {
        return SegmentKind::Log;
    }
    if looks_like_code(&lines) {
        return SegmentKind::Code;
    }
    SegmentKind::Prose
}

const PATH_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "rb", "java", "kt", "c", "h", "cc", "cpp", "hpp",
    "cs", "md", "toml", "json", "yaml", "yml", "txt", "sh", "sql", "lock", "cfg", "ini",
];

/// Whether a single line is a bare filesystem path or directory reference.
fn looks_like_path(line: &str) -> bool {
    let s = line.trim();
    if s.is_empty() || s.chars().any(char::is_whitespace) {
        return false;
    }
    // A network URL is not a workspace anchor; a `file://` URI is.
    if s.starts_with("http://") || s.starts_with("https://") {
        return false;
    }
    if s.starts_with("file://") {
        return true;
    }
    let rooted =
        s.starts_with("./") || s.starts_with("../") || s.starts_with("~/") || s.starts_with('/');
    let has_extension = s
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, ext)| PATH_EXTENSIONS.contains(&ext));
    // A slash makes it a path; a rooted prefix or a known extension makes a
    // slashless token (`net.rs`, `src`) a path too.
    (s.contains('/') && (rooted || has_extension || s.matches('/').count() >= 1))
        || (rooted && !s.contains(' '))
        || has_extension
}

/// Whether the block is delimited tabular data (pipe or tab separated).
fn looks_like_table(lines: &[&str]) -> bool {
    if lines.len() < 2 {
        return false;
    }
    for delimiter in ['|', '\t'] {
        let counts: Vec<usize> = lines.iter().map(|l| l.matches(delimiter).count()).collect();
        if let Some(common) = most_common(&counts)
            && common >= 1
        {
            let agree = counts.iter().filter(|&&c| c == common).count();
            // ≥70% of rows share the same column count.
            if agree * 10 >= lines.len() * 7 {
                return true;
            }
        }
    }
    false
}

const LOG_LEVELS: &[&str] = &[
    "ERROR", "ERR", "WARN", "WARNING", "INFO", "DEBUG", "TRACE", "FATAL", "CRITICAL", "CRIT",
    "PANIC", "PANICKED", "SEVERE", "NOTICE",
];
const ALERT_LEVELS: &[&str] = &[
    "ERROR", "ERR", "WARN", "WARNING", "FATAL", "CRITICAL", "CRIT", "PANIC", "PANICKED", "SEVERE",
];
const STACK_MARKERS: &[&str] = &[
    "at ",
    "File \"",
    "Traceback",
    "panicked at",
    "-->",
    "Caused by",
    "thread '",
];

/// Whether at least half of the non-empty lines look like log or trace lines.
fn looks_like_log(lines: &[&str]) -> bool {
    let non_empty: Vec<&&str> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() {
        return false;
    }
    let matched = non_empty.iter().filter(|l| is_log_line(l)).count();
    matched * 2 >= non_empty.len()
}

fn is_log_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return false;
    }
    if STACK_MARKERS.iter().any(|m| t.starts_with(m)) {
        return true;
    }
    if has_level_token(t, LOG_LEVELS) {
        return true;
    }
    let first = t.split_whitespace().next().unwrap_or("");
    if first.starts_with('[') {
        return true;
    }
    // A leading timestamp-ish token: begins with a digit and carries a `:` or
    // `-` (a clock or a date).
    first.chars().next().is_some_and(|c| c.is_ascii_digit())
        && (first.contains(':') || first.contains('-'))
}

fn is_alert_line(line: &str) -> bool {
    has_level_token(line.trim_start(), ALERT_LEVELS)
}

/// Whether any whole word in `s` (uppercased) is in `set`. Whole-word matching
/// keeps `"information"` from matching `INFO`.
fn has_level_token(s: &str, set: &[&str]) -> bool {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| set.contains(&w.to_ascii_uppercase().as_str()))
}

/// A conservative unfenced-code check: ≥3 lines, most of them structurally
/// code-shaped. Misclassification only routes a `snippet` to `doc` or back;
/// both are full evidence frames, so the bar is set to avoid eating prose.
fn looks_like_code(lines: &[&str]) -> bool {
    if lines.len() < 3 {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "fn ",
        "def ",
        "class ",
        "import ",
        "const ",
        "let ",
        "var ",
        "pub ",
        "function ",
        "#include",
        "package ",
        "func ",
        "return ",
        "if ",
        "for ",
        "while ",
        "@",
    ];
    let codey = lines
        .iter()
        .filter(|l| {
            let t = l.trim();
            let te = l.trim_end();
            te.ends_with(';')
                || te.ends_with('{')
                || te.ends_with('}')
                || te.ends_with("=>")
                || te.ends_with("):")
                || PREFIXES.iter().any(|p| t.starts_with(p))
        })
        .count();
    codey * 2 >= lines.len()
}

fn most_common(values: &[usize]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (value, count)
    for &v in values {
        let count = values.iter().filter(|&&x| x == v).count();
        match best {
            Some((_, bc)) if bc >= count => {}
            _ => best = Some((v, count)),
        }
    }
    best.map(|(v, _)| v)
}

// ---------------------------------------------------------------------------
// Distillation
// ---------------------------------------------------------------------------

/// The distilled inline rendering of an oversized log: a header plus the alert
/// lines with context, or head/tail when there are no alerts, gaps elided.
fn distill_log(full: &str) -> (String, Option<String>, Option<String>) {
    let lines: Vec<&str> = full.lines().collect();
    let total = lines.len();
    if total == 0 {
        return (String::new(), None, None);
    }
    let alerts: Vec<usize> = (0..total).filter(|&i| is_alert_line(lines[i])).collect();

    let mut keep: BTreeSet<usize> = BTreeSet::new();
    keep.insert(0);
    keep.insert(total - 1);
    if alerts.is_empty() {
        for i in 0..LOG_HEAD.min(total) {
            keep.insert(i);
        }
        for i in total.saturating_sub(LOG_TAIL)..total {
            keep.insert(i);
        }
    } else {
        for &a in &alerts {
            let lo = a.saturating_sub(LOG_CONTEXT);
            let hi = (a + LOG_CONTEXT).min(total - 1);
            for i in lo..=hi {
                keep.insert(i);
            }
        }
    }

    let mut out = String::new();
    let alert_note = if alerts.is_empty() {
        String::new()
    } else {
        format!(", {} error/warn line(s)", alerts.len())
    };
    out.push_str(&format!("[{total}-line log{alert_note}]\n"));

    let mut prev: Option<usize> = None;
    for &i in &keep {
        if let Some(p) = prev
            && i > p + 1
        {
            out.push_str(&format!("… ({} lines elided) …\n", i - p - 1));
        }
        out.push_str(lines[i]);
        out.push('\n');
        prev = Some(i);
    }

    let valid_from = leading_timestamp(lines.first().copied());
    let valid_to = leading_timestamp(lines.last().copied());
    (out.trim_end().to_string(), valid_from, valid_to)
}

/// The first whitespace-delimited token of `line`, but only if it is already in
/// the protocol timestamp profile (§F4). Opportunistic and guarded: a log whose
/// timestamps are not F4-shaped simply yields no temporal bound rather than an
/// invalid one.
fn leading_timestamp(line: Option<&str>) -> Option<String> {
    let token = line?.split_whitespace().next()?;
    is_protocol_timestamp(token).then(|| token.to_string())
}

/// The distilled inline rendering of a table: shape, inferred column types, and
/// a small sample of rows.
fn distill_table(full: &str) -> String {
    let lines: Vec<&str> = full.lines().filter(|l| !l.trim().is_empty()).collect();
    let delimiter = if lines.iter().any(|l| l.contains('|')) {
        '|'
    } else {
        '\t'
    };

    let parse = |line: &str| -> Vec<String> {
        let mut cells: Vec<String> = line
            .split(delimiter)
            .map(|c| c.trim().to_string())
            .collect();
        // Pipe tables usually have leading/trailing delimiters → empty edges.
        if delimiter == '|' {
            if cells.first().is_some_and(|c| c.is_empty()) {
                cells.remove(0);
            }
            if cells.last().is_some_and(|c| c.is_empty()) {
                cells.pop();
            }
        }
        cells
    };

    let mut rows: Vec<Vec<String>> = lines.iter().map(|l| parse(l)).collect();
    // Drop a markdown separator row (`---|:--:|---`).
    rows.retain(|r| !r.iter().all(|c| is_separator_cell(c)));
    if rows.is_empty() {
        return full.to_string();
    }

    let header = rows.remove(0);
    let cols = header.len();
    let data = rows;

    let mut column_summaries: Vec<String> = Vec::with_capacity(cols);
    for (idx, name) in header.iter().enumerate() {
        let samples: Vec<&str> = data
            .iter()
            .filter_map(|r| r.get(idx))
            .map(String::as_str)
            .filter(|c| !c.is_empty())
            .collect();
        column_summaries.push(format!("{name} ({})", infer_column_type(&samples)));
    }

    let mut out = String::new();
    out.push_str(&format!("[{} rows × {cols} columns]\n", data.len()));
    out.push_str(&format!("columns: {}\n", column_summaries.join(", ")));
    out.push_str("sample:\n");
    out.push_str(&header.join(" | "));
    out.push('\n');
    for row in data.iter().take(TABLE_SAMPLE) {
        out.push_str(&row.join(" | "));
        out.push('\n');
    }
    if data.len() > TABLE_SAMPLE {
        out.push_str(&format!("… ({} more rows)", data.len() - TABLE_SAMPLE));
    }
    out.trim_end().to_string()
}

fn is_separator_cell(cell: &str) -> bool {
    let c = cell.trim();
    !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')
}

fn infer_column_type(samples: &[&str]) -> &'static str {
    if samples.is_empty() {
        return "text";
    }
    if samples.iter().all(|s| s.parse::<i64>().is_ok()) {
        return "int";
    }
    if samples.iter().all(|s| s.parse::<f64>().is_ok()) {
        return "float";
    }
    if samples
        .iter()
        .all(|s| matches!(s.to_ascii_lowercase().as_str(), "true" | "false"))
    {
        return "bool";
    }
    if samples.iter().all(|s| looks_like_datetime(s)) {
        return "timestamp";
    }
    "text"
}

fn looks_like_datetime(s: &str) -> bool {
    if is_protocol_timestamp(s) {
        return true;
    }
    // Loose `YYYY-MM-DD`-ish: starts with four digits then a dash.
    let b = s.as_bytes();
    b.len() >= 8 && b[..4].iter().all(u8::is_ascii_digit) && b.get(4) == Some(&b'-')
}

/// The distilled inline rendering of an oversized code block: head and tail with
/// the middle elided.
fn distill_code(full: &str) -> String {
    let lines: Vec<&str> = full.lines().collect();
    let total = lines.len();
    if total <= CODE_HEAD + CODE_TAIL {
        return full.to_string();
    }
    let mut out = String::new();
    for line in &lines[..CODE_HEAD] {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!(
        "… ({} lines elided) …\n",
        total - CODE_HEAD - CODE_TAIL
    ));
    for line in &lines[total - CODE_TAIL..] {
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

/// One content-addressed piece of pasted evidence. Immutable: its bytes and
/// therefore its hashes never change, which is what makes `verify` exact.
struct Artifact {
    id: String,
    kind: FrameKind,
    title: String,
    citation_label: String,
    score: f32,
    /// The exact source bytes, stored so a `full` re-query rehydrates losslessly.
    full_content: String,
    /// `sha256:<hex>` over `full_content` — the store key and the id seed.
    address_hash: String,
    /// The inline rendering the model sees by default. Equal to `full_content`
    /// when the artifact was too small to be worth compacting.
    inline_content: String,
    transform: Transform,
    fidelity: ContentFidelity,
    /// Whether `inline_content` is a genuine distillation (vs. verbatim).
    compacted: bool,
    valid_from: Option<String>,
    valid_to: Option<String>,
    summary: String,
}

impl Artifact {
    fn build(kind: SegmentKind, full_content: String) -> Self {
        let frame_kind = kind
            .frame_kind()
            .expect("PathRef is routed to anchors before build");
        let address_hash = sha256_digest(&full_content);
        let id = format!("frm_{}", short_hash(&address_hash));

        // Distill, then decide whether the distillation actually pays.
        let (distilled, verbatim_transform, distilled_transform, distilled_fidelity, vf, vt) =
            match kind {
                SegmentKind::Log => {
                    let (inline, vf, vt) = distill_log(&full_content);
                    (
                        inline,
                        verbatim_transform(),
                        transform("extractive_summary"),
                        ContentFidelity::Summarized,
                        vf,
                        vt,
                    )
                }
                SegmentKind::Table => (
                    distill_table(&full_content),
                    verbatim_transform(),
                    transform("tabular_sample"),
                    ContentFidelity::Summarized,
                    None,
                    None,
                ),
                SegmentKind::Code => (
                    distill_code(&full_content),
                    verbatim_transform(),
                    transform("truncation"),
                    ContentFidelity::Summarized,
                    None,
                    None,
                ),
                // Prose is never distilled — it is the user's own attached words.
                SegmentKind::Prose | SegmentKind::PathRef => (
                    full_content.clone(),
                    verbatim_transform(),
                    verbatim_transform(),
                    ContentFidelity::Exact,
                    None,
                    None,
                ),
            };

        let full_tokens = budget_tokens(&full_content);
        let worth_compacting = kind != SegmentKind::Prose
            && full_tokens > COMPACT_MIN_TOKENS
            && budget_tokens(&distilled) < full_tokens;

        let (inline_content, transform, fidelity, compacted) = if worth_compacting {
            (distilled, distilled_transform, distilled_fidelity, true)
        } else {
            (
                full_content.clone(),
                verbatim_transform,
                ContentFidelity::Exact,
                false,
            )
        };

        let line_count = full_content.lines().count();
        let title = match kind {
            SegmentKind::Log => format!("log · {line_count} lines"),
            SegmentKind::Table => format!("table · {line_count} lines"),
            SegmentKind::Code => format!("code · {line_count} lines"),
            SegmentKind::Prose => "note".to_string(),
            SegmentKind::PathRef => "path".to_string(),
        };
        let summary = if compacted {
            format!(
                "{title} · {} → {} tokens",
                full_tokens,
                budget_tokens(&inline_content)
            )
        } else {
            format!("{title} · {full_tokens} tokens")
        };

        Self {
            id,
            kind: frame_kind,
            title,
            citation_label: kind.citation_label().to_string(),
            score: kind.score(),
            full_content,
            address_hash,
            inline_content,
            transform,
            fidelity,
            compacted,
            valid_from: vf,
            valid_to: vt,
            summary,
        }
    }

    /// The default budget cost of this artifact (its compact/inline rendering).
    fn inline_tokens(&self) -> u32 {
        budget_tokens(&self.inline_content)
    }

    fn content_ref(&self, provider_id: &str) -> ContentRef {
        ContentRef {
            provider_id: provider_id.to_string(),
            // Opaque resolver handle, distinct from any source `uri`.
            uri: format!("context://{provider_id}/artifacts/{}", self.address_hash),
            expires_at: None,
        }
    }

    /// Provenance for pasted evidence: kind `derivation`, *not* `file`. Pasted
    /// text has no URI a host can re-read, so a `file` digest would be a lie and
    /// would trip §F5. The real hash lives in `canonical_content_hash`.
    fn provenance(&self) -> Provenance {
        Provenance {
            kind: "derivation".to_string(),
            uri: None,
            range: None,
            digest: None,
            method: Some("paste".to_string()),
            by: Some(TRANSFORM_IMPL.to_string()),
        }
    }

    /// The digests a host might legitimately hold for a frame this artifact
    /// served — its full-source hash, plus the inline hash of a real compaction.
    fn served_digests(&self) -> Vec<String> {
        let mut digests = vec![self.address_hash.clone()];
        if self.compacted {
            let inline = sha256_digest(&self.inline_content);
            if inline != self.address_hash {
                digests.push(inline);
            }
        }
        digests
    }

    fn apply_common(&self, frame: &mut ContextFrame) {
        frame.citation_label = Some(self.citation_label.clone());
        frame.provenance = vec![self.provenance()];
        frame.inline_content_requirement =
            Some(InlineContentRequirement::ResolvableReferenceAllowed);
        frame.valid_from = self.valid_from.clone();
        frame.valid_to = self.valid_to.clone();
    }

    /// A `full` frame: the exact source bytes inline. This is the rehydration
    /// path — the callable answer to a `[full]` representation preference.
    fn as_full(&self) -> ContextFrame {
        let content = self.full_content.clone();
        let cost = budget_tokens(&content);
        let mut frame = ContextFrame::full(
            self.id.clone(),
            self.kind,
            self.title.clone(),
            content,
            self.score,
            cost,
        );
        frame.content_digest = Some(self.address_hash.clone());
        frame.content_fidelity = Some(ContentFidelity::Exact);
        self.apply_common(&mut frame);
        frame
    }

    /// A `compact` frame: the distilled inline rendering plus the resolver handle
    /// and the canonical hash. `token_cost` and `content_digest` are recomputed
    /// over the inline bytes actually emitted (§B3).
    fn as_compact(&self, provider_id: &str) -> ContextFrame {
        let inline = self.inline_content.clone();
        let cost = budget_tokens(&inline);
        let mut frame = ContextFrame::full(
            self.id.clone(),
            self.kind,
            self.title.clone(),
            inline.clone(),
            self.score,
            cost,
        );
        frame.representation = Representation::Compact;
        frame.content_digest = Some(sha256_digest(&inline));
        frame.canonical_content_hash = Some(self.address_hash.clone());
        frame.canonical_token_cost = Some(budget_tokens(&self.full_content));
        frame.transform = Some(self.transform.clone());
        frame.content_ref = Some(self.content_ref(provider_id));
        frame.content_fidelity = Some(self.fidelity);
        self.apply_common(&mut frame);
        frame
    }

    /// A `reference` frame: no inline content, only the resolver handle and the
    /// canonical hash. `token_cost` is 0 — nothing is inlined.
    fn as_reference(&self, provider_id: &str) -> ContextFrame {
        let mut frame = ContextFrame::reference(
            self.id.clone(),
            self.kind,
            self.title.clone(),
            self.content_ref(provider_id),
            self.address_hash.clone(),
            self.score,
        );
        frame.canonical_token_cost = Some(budget_tokens(&self.full_content));
        frame.content_fidelity = Some(ContentFidelity::Omitted);
        self.apply_common(&mut frame);
        frame
    }

    fn as_representation(&self, provider_id: &str, representation: Representation) -> ContextFrame {
        match representation {
            Representation::Full => self.as_full(),
            Representation::Compact => self.as_compact(provider_id),
            Representation::Reference => self.as_reference(provider_id),
        }
    }
}

fn transform(method: &str) -> Transform {
    Transform {
        method: method.to_string(),
        implementation: TRANSFORM_IMPL.to_string(),
        version: TRANSFORM_VERSION.to_string(),
    }
}

fn verbatim_transform() -> Transform {
    transform("verbatim")
}

// ---------------------------------------------------------------------------
// The provider
// ---------------------------------------------------------------------------

/// A local, egress-free [`ContextProvider`] serving one paste's evidence.
///
/// It advertises `full`/`compact`/`reference` and `resolve`, and answers a
/// `[full]`-preference query straight from its immutable artifact store — the
/// working rehydration path behind the `resolve` capability (see [ADR 0006] on
/// why this is not an ADR 0004 dead flag). Because artifacts are
/// content-addressed and immutable, `verify` is exact.
pub struct IngestProvider {
    id: String,
    info: ProviderInfo,
    capabilities: Capabilities,
    artifacts: Vec<Artifact>,
}

impl IngestProvider {
    fn new(id: impl Into<String>, artifacts: Vec<Artifact>) -> Self {
        let id = id.into();
        let mut kinds: Vec<String> = artifacts
            .iter()
            .map(|a| frame_kind_name(a.kind).to_string())
            .collect();
        kinds.sort();
        kinds.dedup();

        let info = ProviderInfo {
            name: DEFAULT_PROVIDER_ID.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            // Local-only: the whole point is that a typed paste never leaves the
            // machine, so the provider is auto-permitted (§C1 gates egress only).
            data_flow: DataFlow {
                reads: true,
                writes: false,
                egress: false,
                egress_scopes: vec![EgressScope::LocalOnly],
            },
        };
        let capabilities = Capabilities {
            query: QueryCapability { kinds },
            correlation: false,
            graph: false,
            embeddings_fingerprint: None,
            verify: true,
            representations: vec![
                Representation::Full,
                Representation::Compact,
                Representation::Reference,
            ],
            resolve: true,
        };
        Self {
            id,
            info,
            capabilities,
            artifacts,
        }
    }

    /// Sum of the default (compact) budget cost of every artifact — the
    /// `max_tokens` the bundle query uses so a default fan-out returns them all.
    fn default_budget_tokens(&self) -> u32 {
        self.artifacts.iter().map(Artifact::inline_tokens).sum()
    }

    /// How many artifacts this provider holds.
    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }
}

#[async_trait]
impl ContextProvider for IngestProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    async fn query(&self, query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
        // The first supported representation the host prefers; `[full]` by
        // default. A `[full]` preference is the rehydration path.
        let representation = query
            .select_representation(&[
                Representation::Full,
                Representation::Compact,
                Representation::Reference,
            ])
            .unwrap_or(Representation::Full);

        let mut candidates: Vec<ContextFrame> = self
            .artifacts
            .iter()
            .filter(|a| query.kinds.is_empty() || query.kinds.contains(&a.kind))
            .map(|a| a.as_representation(&self.id, representation))
            .collect();

        // Rank by score, breaking ties by id so the output is deterministic.
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });

        // Greedy fit under the query's budget and frame cap (§B1, §B4). Every
        // frame's `token_cost` is honest (§B3), so the host's audit passes.
        let mut frames: Vec<ContextFrame> = Vec::new();
        let mut used: u64 = 0;
        let mut dropped: u32 = 0;
        for frame in candidates {
            if frames.len() as u32 >= query.max_frames {
                dropped += 1;
                continue;
            }
            let cost = frame.token_cost as u64;
            if used + cost > query.max_tokens as u64 {
                dropped += 1;
                continue;
            }
            used += cost;
            frames.push(frame);
        }

        Ok(ContextQueryResult {
            frames,
            truncated: dropped > 0,
            dropped_estimate: (dropped > 0).then_some(dropped),
        })
    }

    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, HostError> {
        let verdicts = request
            .frames
            .iter()
            .map(|held| {
                let verdict = match self.artifacts.iter().find(|a| a.id == held.frame_id) {
                    // Immutable + content-addressed: a matching digest is
                    // provably still valid, no source re-read required.
                    Some(artifact) => match &held.content_digest {
                        Some(digest) if artifact.served_digests().contains(digest) => {
                            Verdict::Valid
                        }
                        Some(_) => Verdict::Stale {
                            replacement_digest: Some(artifact.address_hash.clone()),
                        },
                        // Digestless identities are filtered by the host before
                        // `verify`; if one arrives anyway, we cannot vouch.
                        None => Verdict::Unknown,
                    },
                    // The store is authoritative-complete for this session, so an
                    // unknown id is genuinely not ours to serve.
                    None => Verdict::Gone,
                };
                FrameVerdict::new(held.clone(), verdict)
            })
            .collect();
        Ok(VerifyResponse::new(verdicts))
    }
}

#[cfg(test)]
mod tests;
