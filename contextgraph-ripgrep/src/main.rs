//! `contextgraph-ripgrep` — a reference Context Graph Protocol provider that
//! serves `Snippet` frames from a content search over a target directory
//! (issue #18).
//!
//! It shells out to `rg` (ripgrep) when it is on `PATH` and otherwise falls
//! back to a built-in gitignore-agnostic walk; either way it re-reads the
//! matched line's exact on-disk bytes, so every frame carries real
//! [`Provenance`](contextgraph_types::Provenance): a `file://` URI, an
//! `L<line>` range, and a `sha256:<hex>` digest a host can independently
//! re-verify (`SPEC.md` §6.2). Costs are honest (`budget_tokens`) and the
//! response reports `truncated`/`dropped_estimate` when the query's
//! `max_frames`/`max_tokens` cap bites.
//!
//! Usage: `contextgraph-ripgrep [ROOT]` — `ROOT` is the directory to search,
//! defaulting to this crate's bundled `fixtures/` (which the conformance suite
//! probes). The search terms are the words of the query's `query_text` (or
//! `goal`); a query that matches nothing falls back to the first line of each
//! file, so the provider always has honest evidence to serve.

use std::path::{Path, PathBuf};
use std::process::Command;

use contextgraph_refprov::{FileFrame, FrameSource, ProviderConfig, line_range_bytes, serve, walk};
use contextgraph_types::{ContextFrame, ContextQuery, FrameKind, Relation};

/// The directory searched when no `ROOT` argument is given: this crate's
/// bundled fixtures, resolved at compile time so the path is correct regardless
/// of the process's working directory.
const DEFAULT_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");

fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));
    serve(Ripgrep { root });
}

struct Ripgrep {
    root: PathBuf,
}

impl FrameSource for Ripgrep {
    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            name: "contextgraph-ripgrep",
            version: env!("CARGO_PKG_VERSION"),
            kinds: vec!["snippet"],
        }
    }

    fn candidates(&mut self, query: &ContextQuery) -> Vec<ContextFrame> {
        let terms = extract_terms(query);
        let mut matches = find_matches(&self.root, &terms);
        if matches.is_empty() {
            // Nothing matched — serve the first real line of each file so the
            // provider still offers honest, provenance-carrying evidence.
            matches = fallback_matches(&self.root);
        }
        matches
            .into_iter()
            .enumerate()
            .filter_map(|(rank, (path, line))| self.snippet_frame(&path, line, rank))
            .collect()
    }
}

impl Ripgrep {
    /// Build a `Snippet` frame for the match at `line_no` (1-indexed) in `path`,
    /// or `None` if the line's bytes are not valid UTF-8 (so `content` cannot be
    /// the exact on-disk bytes the digest must cover) or the line is blank.
    fn snippet_frame(&self, path: &Path, line_no: usize, rank: usize) -> Option<ContextFrame> {
        let bytes = std::fs::read(path).ok()?;
        let (from, to) = line_range_bytes(&bytes, line_no, line_no)?;
        let content = std::str::from_utf8(&bytes[from..to]).ok()?.to_string();
        if content.trim().is_empty() {
            return None;
        }
        let uri = file_uri(path);
        let relative = self.relative(path);
        let range = format!("L{line_no}");
        let mut frame = FileFrame {
            id: format!("rg:{relative}#{range}"),
            kind: FrameKind::Snippet,
            title: format!("{relative} {range}"),
            content,
            uri: uri.clone(),
            citation: format!("{relative} {range}"),
            range,
            // A gently decaying score keeps the ordering deterministic and in
            // `[0, 1]` no matter how many matches there are.
            score: (0.95 - 0.03 * rank as f32).clamp(0.05, 1.0),
            by: "contextgraph-ripgrep",
        }
        .build();
        // One labelled edge locating the snippet in its file, so a graph-aware
        // host can anchor on the file (§G4) and the edge is citable by name.
        frame.relations = vec![Relation {
            rel: "cgp.match.in_file".to_string(),
            target_uri: uri,
            display_name: Some(relative),
        }];
        Some(frame)
    }

    /// The path relative to the search root, for stable ids and citations.
    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

/// An absolute `file://` URI for `path`, canonicalized so a host re-reads the
/// same bytes regardless of the provider's working directory.
fn file_uri(path: &Path) -> String {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", absolute.display())
}

/// The search terms: alphanumeric words of length ≥ 3 drawn from the query's
/// `query_text`, or its `goal` when no text is given. Lowercased and de-duped so
/// matching is case-insensitive and each term is tried once.
fn extract_terms(query: &ContextQuery) -> Vec<String> {
    let text = query.query_text.as_deref().unwrap_or(query.goal.as_str());
    let mut terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

/// Find `(path, line)` matches for `terms` under `root`, preferring `rg` and
/// falling back to a built-in walk. The result is sorted and de-duplicated.
fn find_matches(root: &Path, terms: &[String]) -> Vec<(PathBuf, usize)> {
    if !terms.is_empty()
        && let Some(mut matches) = ripgrep_matches(root, terms)
        && !matches.is_empty()
    {
        matches.sort();
        matches.dedup();
        return matches;
    }
    let mut matches = walk_matches(root, terms);
    matches.sort();
    matches.dedup();
    matches
}

/// Run `rg` for `terms` under `root`, returning `(path, line)` pairs, or `None`
/// if `rg` is absent or its output cannot be parsed (the caller then walks).
fn ripgrep_matches(root: &Path, terms: &[String]) -> Option<Vec<(PathBuf, usize)>> {
    let mut command = Command::new("rg");
    command.args([
        "--line-number",
        "--no-heading",
        "--color",
        "never",
        "--no-messages",
        "--text",
    ]);
    for term in terms {
        command.args(["-e", term]);
    }
    command.arg(root);
    let output = command.output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let mut matches = Vec::new();
    for line in text.lines() {
        // `rg` prints `path:line:content`; content keeps its own colons.
        let mut parts = line.splitn(3, ':');
        if let (Some(path), Some(number)) = (parts.next(), parts.next())
            && let Ok(line_no) = number.parse::<usize>()
        {
            matches.push((PathBuf::from(path), line_no));
        }
    }
    Some(matches)
}

/// Built-in fallback search: scan every text file under `root` for a line
/// containing any of `terms`. Empty `terms` yields no matches (the caller then
/// serves the first-line fallback instead of every line of every file).
fn walk_matches(root: &Path, terms: &[String]) -> Vec<(PathBuf, usize)> {
    if terms.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for path in text_files(root) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let lowered = line.to_lowercase();
            if terms.iter().any(|term| lowered.contains(term.as_str())) {
                matches.push((path.clone(), index + 1));
            }
        }
    }
    matches
}

/// The first non-blank line of each text file under `root`, so the provider
/// always has at least one honest frame to serve for a non-empty tree.
fn fallback_matches(root: &Path) -> Vec<(PathBuf, usize)> {
    let mut matches = Vec::new();
    for path in text_files(root) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if !line.trim().is_empty() {
                matches.push((path, index + 1));
                break;
            }
        }
    }
    matches
}

/// The text files under `root` the built-in walk considers, filtered by a small
/// source/prose extension allowlist so binaries are never scanned.
fn text_files(root: &Path) -> Vec<PathBuf> {
    walk(root)
        .into_iter()
        .filter(|path| is_text_file(path))
        .collect()
}

/// Whether `path` has a source/prose extension the built-in walk searches.
fn is_text_file(path: &Path) -> bool {
    const EXTENSIONS: &[&str] = &[
        "rs", "md", "mdx", "txt", "toml", "json", "py", "ts", "js", "go", "yaml", "yml", "sh",
        "cfg", "ini", "html", "css",
    ];
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext))
}
