//! Shared Context Graph Protocol stdio skeleton for the reference providers
//! (`contextgraph-ripgrep`, `contextgraph-treesitter`) — issue #18.
//!
//! Both reference providers speak the exact same wire protocol as the bundled
//! conformance fixture (`contextgraph-example-docs`): a newline-delimited
//! [`Envelope`] loop over stdin/stdout that handshakes, answers `context/query`
//! and `context/verify`, tolerates a malformed line, and shuts down cleanly.
//! The *only* thing that differs between them is where the frames come from —
//! a `ripgrep` content search versus a symbol-graph extraction — so that
//! provider-specific part is a [`FrameSource`] and everything protocol-shaped
//! lives here, verified once.
//!
//! A source hands the kit already-honest frames (built via [`FileFrame`]
//! or [`DerivedFrame`], which set an exact `content_digest`, an honest
//! [`budget_tokens`] cost, and — for file-backed frames — a `file` provenance
//! digest equal to the on-disk bytes a host re-reads at `uri`+`range`, `SPEC.md`
//! §6.2). The kit then enforces the query contract on top: it filters by
//! `kinds` (§Q1), sorts anchored frames first (§G4), drops content not yet true
//! at an `as_of` pin (§6.1), respects `max_frames`/`max_tokens` with honest
//! `truncated`/`dropped_estimate` (§B1/§B4), rejects a wrong-dimension query
//! embedding (§E1), echoes correlation ids (§H4), and answers `context/verify`
//! from the digests it actually served (§4).

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use contextgraph_host::wire::Envelope;
use contextgraph_types::capability::fingerprint_dimensions;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQuery, ContextQueryResult, DataFlow, EgressScope, ErrorCode,
    FrameKind, FrameVerdict, PROTOCOL_VERSION, Provenance, ProviderInfo, QueryCapability,
    Representation, Verdict, VerifyRequest, VerifyResponse, budget_tokens,
};

/// The embedding space these reference providers declare (`SPEC.md` §E1). The
/// dimension (8) is what a query embedding's length must match; a
/// contradicting length names a different vector space and is rejected
/// `bad_request`. The value is deliberately small — these providers do lexical
/// and structural retrieval, not vector search, so they index no real model's
/// space; declaring a fingerprint at all is what makes the §E1 guarantee
/// probeable rather than vacuously skipped.
pub const EMBEDDING_FINGERPRINT: &str = "contextgraph-reference/8/none";

/// Provider identity plus the frame kinds it serves. The rest of the capability
/// set is identical across the reference providers, so the kit fills it in
/// ([`capabilities`]).
pub struct ProviderConfig {
    /// Stable provider name, surfaced at the handshake.
    pub name: &'static str,
    /// Crate version, surfaced at the handshake.
    pub version: &'static str,
    /// The [`FrameKind`]s this provider serves, e.g. `["snippet"]`. Declared so
    /// the §Q1 `kinds-filter` probe has a kind to narrow to.
    pub kinds: Vec<&'static str>,
}

/// A source of honest, conformance-ready frames for a query.
///
/// The implementor's whole job is to return candidate frames most-relevant
/// first; the kit applies every query-contract constraint on top. Each frame
/// **MUST** already be honest — build it with [`FileFrame`] or
/// [`DerivedFrame`] so `token_cost`, `content_digest`, and any `file`
/// provenance digest are correct by construction.
pub trait FrameSource {
    /// This provider's identity and declared kinds.
    fn config(&self) -> ProviderConfig;
    /// Candidate frames for `query`, ordered most-relevant first.
    fn candidates(&mut self, query: &ContextQuery) -> Vec<ContextFrame>;
}

/// Run the stdio protocol loop for `source` until EOF or `shutdown`.
///
/// This is the whole provider process: it never returns except via `shutdown`
/// (which exits `0`) or a closed pipe (the host went away).
pub fn serve(mut source: impl FrameSource) {
    let config = source.config();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut line = String::new();
    // The digests this process actually served, keyed by frame id, so a
    // `context/verify` is answered from what was really returned rather than
    // from a rubber stamp (`SPEC.md` §4). Accumulates across queries; ids are
    // deterministic, so re-serving a frame overwrites with the same digest.
    let mut served: HashMap<String, String> = HashMap::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or a broken pipe — the host is gone.
            Ok(_) => {}
        }

        let envelope = match serde_json::from_str::<Envelope>(line.trim_end()) {
            Ok(envelope) => envelope,
            Err(_) => {
                // A malformed line: stay alive and say so with a structured
                // `bad_request` code (`SPEC.md` §R1) rather than dying.
                write_envelope(
                    &mut stdout,
                    &Envelope::Error {
                        id: None,
                        code: Some(ErrorCode::BadRequest),
                        message: "line was not a valid CGP envelope".into(),
                    },
                );
                continue;
            }
        };

        match envelope {
            Envelope::Handshake { .. } => {
                write_envelope(
                    &mut stdout,
                    &Envelope::HandshakeAck {
                        protocol_version: PROTOCOL_VERSION.to_string(),
                        provider: provider_info(&config),
                        capabilities: capabilities(&config),
                    },
                );
            }
            Envelope::Query { id, query } => {
                let reply = handle_query(&mut source, &mut served, id, query);
                write_envelope(&mut stdout, &reply);
            }
            Envelope::Verify { request } => {
                write_envelope(
                    &mut stdout,
                    &Envelope::Verified {
                        response: verify(&served, &request),
                    },
                );
            }
            Envelope::Shutdown => std::process::exit(0),
            // handshake_ack / frames / verified / error are host→provider-invalid
            // inputs; a provider ignores them.
            _ => {}
        }
    }
}

/// Answer one `context/query`, enforcing the full query contract on top of the
/// source's candidates. Records the digests it returns so a subsequent
/// `context/verify` can vouch for them.
fn handle_query(
    source: &mut impl FrameSource,
    served: &mut HashMap<String, String>,
    id: Option<String>,
    query: ContextQuery,
) -> Envelope {
    // §E1: a query embedding whose length contradicts the declared fingerprint
    // dimension is from a different space; reject it `bad_request` rather than
    // score meaningless similarity. Checked before any work.
    if let Some(error) = embedding_dimension_error(&query, id.clone()) {
        return error;
    }

    let mut frames = source.candidates(&query);

    // §Q1: a non-empty `kinds` is a filter, not a hint.
    if !query.kinds.is_empty() {
        frames.retain(|frame| query.kinds.contains(&frame.kind));
    }
    // §G4: anchored frames first (stable partition preserves relative order).
    if !query.anchors.is_empty() {
        frames.sort_by_key(|frame| !is_anchored(frame, &query.anchors));
    }
    // §6.1: honor an `as_of` pin — content not yet true then is not returned.
    // The timestamp profile is one spelling per instant, so a lexicographic
    // compare on the UTC strings is a chronological one.
    if let Some(as_of) = query.as_of.as_deref() {
        frames.retain(|frame| !frame.valid_from.as_deref().is_some_and(|vf| vf > as_of));
    }

    let (frames, truncated, dropped_estimate) =
        apply_budget(frames, query.max_frames, query.max_tokens);

    for frame in &frames {
        if let Some(digest) = &frame.content_digest {
            served.insert(frame.id.clone(), digest.clone());
        }
    }

    Envelope::Frames {
        id,
        result: ContextQueryResult {
            frames,
            truncated,
            dropped_estimate,
            ..Default::default()
        },
    }
}

/// Answer `context/verify` honestly, comparing each presented digest against
/// the one this process last served for that frame id (`SPEC.md` §4). Never
/// served ⇒ `gone`; no digest presented ⇒ `unknown`; equal ⇒ `valid`; different
/// ⇒ `stale`, offering the current digest as the replacement.
fn verify(served: &HashMap<String, String>, request: &VerifyRequest) -> VerifyResponse {
    VerifyResponse::new(
        request
            .frames
            .iter()
            .map(|frame| {
                let verdict = match served.get(&frame.frame_id) {
                    None => Verdict::Gone,
                    Some(current) => match frame.content_digest.as_deref() {
                        None => Verdict::Unknown,
                        Some(presented) if presented == current => Verdict::Valid,
                        Some(_) => Verdict::Stale {
                            replacement_digest: Some(current.clone()),
                        },
                    },
                };
                FrameVerdict::new(frame.clone(), verdict)
            })
            .collect(),
    )
}

/// The `bad_request` reply for a query embedding whose length contradicts the
/// declared fingerprint dimension (`SPEC.md` §E1), or `None` when the query
/// carries no embedding or one of the correct length.
fn embedding_dimension_error(query: &ContextQuery, id: Option<String>) -> Option<Envelope> {
    let embedding = query.embedding.as_ref()?;
    let expected = fingerprint_dimensions(EMBEDDING_FINGERPRINT)?;
    if embedding.len() == expected {
        return None;
    }
    Some(Envelope::Error {
        id,
        code: Some(ErrorCode::BadRequest),
        message: format!(
            "query embedding has {} dimensions; this provider indexes {} ({EMBEDDING_FINGERPRINT}) (§E1)",
            embedding.len(),
            expected
        ),
    })
}

/// Whether `frame` is anchored by any of `anchors` (`SPEC.md` §G4): its own
/// `uri` at zero hops, or any labelled edge's `target_uri` at one hop.
fn is_anchored(frame: &ContextFrame, anchors: &[String]) -> bool {
    frame
        .uri
        .as_deref()
        .is_some_and(|uri| anchors.iter().any(|anchor| anchor == uri))
        || frame
            .relations
            .iter()
            .any(|relation| anchors.contains(&relation.target_uri))
}

/// Enforce `max_frames` and `max_tokens` on an ordered frame list, returning the
/// kept frames plus honest `(truncated, dropped_estimate)` (`SPEC.md` §B1/§B4).
/// Frames are kept in order until either cap would be exceeded; everything past
/// that is dropped and counted.
fn apply_budget(
    frames: Vec<ContextFrame>,
    max_frames: u32,
    max_tokens: u32,
) -> (Vec<ContextFrame>, bool, Option<u32>) {
    let total = frames.len();
    let mut kept = Vec::new();
    let mut tokens: u64 = 0;
    for frame in frames {
        if kept.len() as u32 >= max_frames {
            break;
        }
        let next = tokens + frame.token_cost as u64;
        if next > max_tokens as u64 {
            break;
        }
        tokens = next;
        kept.push(frame);
    }
    let dropped = total - kept.len();
    (kept, dropped > 0, (dropped > 0).then_some(dropped as u32))
}

/// The capability set every reference provider advertises. Only
/// [`kinds`](ProviderConfig::kinds) varies; everything else is fixed so all 13
/// conformance checks *run* (a capability a provider does not declare makes its
/// check skip, which the external harness scores as a failure).
fn capabilities(config: &ProviderConfig) -> Capabilities {
    Capabilities {
        query: QueryCapability {
            kinds: config
                .kinds
                .iter()
                .map(|kind| (*kind).to_string())
                .collect(),
        },
        correlation: true,
        graph: true,
        embeddings_fingerprint: Some(EMBEDDING_FINGERPRINT.to_string()),
        verify: true,
        representations: vec![],
        resolve: false,
    }
}

/// A reference provider reads the local workspace and serves local frames;
/// nothing leaves the machine, so it declares the `local-only` egress scope
/// alongside `egress: false` — the honest, consistent posture (`SPEC.md` §3).
fn provider_info(config: &ProviderConfig) -> ProviderInfo {
    ProviderInfo {
        name: config.name.to_string(),
        version: config.version.to_string(),
        data_flow: DataFlow {
            reads: true,
            writes: false,
            egress: false,
            egress_scopes: vec![EgressScope::LocalOnly],
        },
    }
}

fn write_envelope(stdout: &mut std::io::Stdout, envelope: &Envelope) {
    // A provider is a plain pipe writer; if the host has gone, give up quietly.
    if let Ok(line) = serde_json::to_string(envelope) {
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}

/// A protocol content digest over `bytes`: `sha256:<64 lowercase hex>`
/// (`SPEC.md` §F5). Byte-for-byte what `contextgraph_host::verify` recomputes,
/// so an unmutated file-backed frame verifies end to end (§6.2).
pub fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity("sha256:".len() + 64);
    out.push_str("sha256:");
    for byte in hash {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// The byte span of a 1-indexed, inclusive line range `[start, end]`, computed
/// **identically** to the host verifier's `extract_line_range` (`SPEC.md`
/// §6.2): each line includes its terminating `\n`, a final unterminated line
/// runs to EOF, and `end` past EOF clamps to the last line. `None` if `start`
/// is `0`, the range is inverted, or `start` is past EOF — the same cases the
/// host reports `Unreadable`.
///
/// A provider hashes exactly these bytes for its provenance digest, so the
/// host's re-read agrees to the byte.
pub fn line_range_bytes(bytes: &[u8], start: usize, end: usize) -> Option<(usize, usize)> {
    if start == 0 || end < start {
        return None;
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut line_start = 0usize;
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' {
            spans.push((line_start, i + 1));
            line_start = i + 1;
        }
    }
    if line_start < bytes.len() {
        spans.push((line_start, bytes.len()));
    }
    let count = spans.len();
    if start > count {
        return None;
    }
    let end = end.min(count);
    Some((spans[start - 1].0, spans[end - 1].1))
}

/// The core fields of a file-backed frame. Bundled into a struct rather than a
/// long argument list so the builder stays readable and `too_many_arguments`
/// never bites.
pub struct FileFrame {
    /// Provider-scoped, stable id (also the `context/verify` key).
    pub id: String,
    /// The kind this frame represents (`snippet`, `symbol`, …).
    pub kind: FrameKind,
    /// Human label — never a bare id.
    pub title: String,
    /// The **exact** UTF-8 bytes addressed by `uri`+`range`, so the frame's
    /// digest matches the host's re-read (`SPEC.md` §6.2).
    pub content: String,
    /// `file://` URI of the backing file.
    pub uri: String,
    /// Line range within the file, `L<a>` or `L<a>-<b>`.
    pub range: String,
    /// Human citation pointing at the source location, e.g. `sample.rs L7`
    /// (`SPEC.md` §F3 — never a bare id).
    pub citation: String,
    /// Provider-normalized relevance in `[0, 1]`.
    pub score: f32,
    /// The provider that produced this frame (recorded on provenance).
    pub by: &'static str,
}

impl FileFrame {
    /// Build a `full` frame with an honest [`budget_tokens`] cost, a real
    /// `content_digest`, and one `file` provenance entry whose digest equals
    /// `content`'s bytes — so a host re-reading `uri` over `range` confirms both
    /// (`SPEC.md` §6.2/§F5). The caller sets `relations` afterward.
    pub fn build(self) -> ContextFrame {
        let digest = sha256_digest(self.content.as_bytes());
        let token_cost = budget_tokens(&self.content);
        let citation_label = Some(self.citation);
        ContextFrame {
            id: self.id,
            kind: self.kind,
            title: self.title,
            content: Some(self.content),
            content_digest: Some(digest.clone()),
            uri: Some(self.uri.clone()),
            representation: Representation::Full,
            content_fidelity: None,
            canonical_content_hash: None,
            content_ref: None,
            transform: None,
            minimum_content_fidelity: None,
            inline_content_requirement: None,
            score: self.score,
            token_cost,
            canonical_token_cost: None,
            tokenizer_ref: None,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: vec![Provenance {
                kind: "file".to_string(),
                uri: Some(self.uri),
                range: Some(self.range),
                digest: Some(digest),
                method: None,
                by: Some(self.by.to_string()),
            }],
            citation_label,
            embedding: None,
            relations: Vec::new(),
        }
    }
}

/// A frame whose content is *derived* rather than a verbatim file slice — a
/// symbol-graph summary, say. It carries an honest cost and a real
/// `content_digest`, but a `derivation` provenance link (which §F5 does not
/// bind, since it names no re-readable bytes) instead of `file` provenance. The
/// caller sets `relations` afterward.
pub struct DerivedFrame {
    /// Provider-scoped, stable id.
    pub id: String,
    /// The kind this frame represents (typically `graph`).
    pub kind: FrameKind,
    /// Human label.
    pub title: String,
    /// The derived rendering the host may quote.
    pub content: String,
    /// `file://` URI of the resource the content was derived from (surfaced as
    /// the frame's `uri` for anchoring; the `derivation` provenance carries no
    /// digest).
    pub uri: String,
    /// The derivation method, e.g. `line-based-symbol-extraction`.
    pub method: &'static str,
    /// Provider-normalized relevance in `[0, 1]`.
    pub score: f32,
    /// The provider that produced this frame.
    pub by: &'static str,
}

impl DerivedFrame {
    /// Build the frame.
    pub fn build(self) -> ContextFrame {
        let digest = sha256_digest(self.content.as_bytes());
        let token_cost = budget_tokens(&self.content);
        let citation_label = Some(self.title.clone());
        ContextFrame {
            id: self.id,
            kind: self.kind,
            title: self.title,
            content: Some(self.content),
            content_digest: Some(digest),
            uri: Some(self.uri),
            representation: Representation::Full,
            content_fidelity: None,
            canonical_content_hash: None,
            content_ref: None,
            transform: None,
            minimum_content_fidelity: None,
            inline_content_requirement: None,
            score: self.score,
            token_cost,
            canonical_token_cost: None,
            tokenizer_ref: None,
            valid_from: None,
            valid_to: None,
            recorded_at: None,
            provenance: vec![Provenance {
                kind: "derivation".to_string(),
                uri: None,
                range: None,
                digest: None,
                method: Some(self.method.to_string()),
                by: Some(self.by.to_string()),
            }],
            citation_label,
            embedding: None,
            relations: Vec::new(),
        }
    }
}

/// A depth-first list of every file under `root`, skipping `.git`, `target`,
/// `node_modules`, and hidden directories. Sorted, so provider output over a
/// tree is deterministic.
pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(entry) = stack.pop() {
        if entry.is_dir() {
            if is_skippable_dir(&entry) {
                continue;
            }
            if let Ok(read_dir) = std::fs::read_dir(&entry) {
                for child in read_dir.flatten() {
                    stack.push(child.path());
                }
            }
        } else if entry.is_file() {
            files.push(entry);
        }
    }
    files.sort();
    files
}

/// Whether a directory is one a source walk should not descend into. The root
/// itself has no `file_name` when passed as `.`, so it is never skipped.
fn is_skippable_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(name)
            if name == ".git" || name == "target" || name == "node_modules" || name.starts_with('.')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_answer_vectors() {
        // Anchor the digest to ground truth, not just to itself.
        assert_eq!(
            sha256_digest(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn line_range_bytes_matches_the_host_extraction_semantics() {
        // Four newline-terminated lines; a line's bytes include its trailing \n.
        let content = b"line one\nline two\nline three\nline four\n";
        let (from, to) = line_range_bytes(content, 2, 3).expect("valid range");
        assert_eq!(&content[from..to], b"line two\nline three\n");

        // Single line.
        let (from, to) = line_range_bytes(content, 1, 1).expect("valid range");
        assert_eq!(&content[from..to], b"line one\n");

        // End past EOF clamps to the last line.
        let (from, to) = line_range_bytes(content, 4, 99).expect("valid range");
        assert_eq!(&content[from..to], b"line four\n");

        // A final unterminated line runs to EOF.
        let unterminated = b"a\nb";
        let (from, to) = line_range_bytes(unterminated, 2, 2).expect("valid range");
        assert_eq!(&unterminated[from..to], b"b");

        // Degenerate ranges are rejected, exactly like the host verifier.
        assert!(line_range_bytes(content, 0, 1).is_none());
        assert!(line_range_bytes(content, 3, 2).is_none());
        assert!(line_range_bytes(content, 99, 99).is_none());
    }

    #[test]
    fn a_file_backed_frame_declares_an_honest_cost_and_matching_digest() {
        let content = "fn main() {}\n";
        let frame = FileFrame {
            id: "sym:x".to_string(),
            kind: FrameKind::Symbol,
            title: "fn main".to_string(),
            content: content.to_string(),
            uri: "file:///tmp/x.rs".to_string(),
            range: "L1".to_string(),
            citation: "x.rs L1".to_string(),
            score: 0.5,
            by: "test",
        }
        .build();
        assert_eq!(frame.token_cost, budget_tokens(content));
        assert!(frame.declares_honest_token_cost());
        assert!(frame.has_usable_content_digest());
        // The provenance digest equals the content digest, so a host re-reading
        // the exact bytes confirms both.
        assert_eq!(frame.provenance[0].digest, frame.content_digest);
        assert!(frame.representation_invariants().is_ok());
    }

    #[test]
    fn apply_budget_reports_honest_truncation() {
        let make = |id: &str| {
            FileFrame {
                id: id.to_string(),
                kind: FrameKind::Snippet,
                title: id.to_string(),
                content: "abcd".to_string(), // 1 budget token
                uri: "file:///tmp/x".to_string(),
                range: "L1".to_string(),
                citation: "x L1".to_string(),
                score: 0.5,
                by: "test",
            }
            .build()
        };
        let frames = vec![make("a"), make("b"), make("c")];
        // Frame cap bites first.
        let (kept, truncated, dropped) = apply_budget(frames, 2, 4096);
        assert_eq!(kept.len(), 2);
        assert!(truncated);
        assert_eq!(dropped, Some(1));

        // A generous cap keeps everything and reports no truncation.
        let frames = vec![make("a"), make("b")];
        let (kept, truncated, dropped) = apply_budget(frames, 8, 4096);
        assert_eq!(kept.len(), 2);
        assert!(!truncated);
        assert_eq!(dropped, None);
    }

    #[test]
    fn verify_distinguishes_served_unchanged_stale_and_gone() {
        let mut served = HashMap::new();
        served.insert("frm".to_string(), "sha256:aa".to_string());

        let ask = |frame_id: &str, digest: Option<&str>| {
            VerifyRequest::new(vec![contextgraph_types::FrameId::new(
                "p",
                frame_id,
                digest.map(str::to_string),
            )])
        };

        assert_eq!(
            verify(&served, &ask("frm", Some("sha256:aa"))).verdicts[0].verdict,
            Verdict::Valid
        );
        assert_eq!(
            verify(&served, &ask("frm", Some("sha256:bb"))).verdicts[0].verdict,
            Verdict::Stale {
                replacement_digest: Some("sha256:aa".to_string())
            }
        );
        assert_eq!(
            verify(&served, &ask("missing", Some("sha256:aa"))).verdicts[0].verdict,
            Verdict::Gone
        );
        assert_eq!(
            verify(&served, &ask("frm", None)).verdicts[0].verdict,
            Verdict::Unknown
        );
    }
}
