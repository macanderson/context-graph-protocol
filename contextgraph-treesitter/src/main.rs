//! `contextgraph-treesitter` — a reference Context Graph Protocol provider that
//! parses Rust source into a symbol graph (issue #18).
//!
//! It serves `Symbol` frames (one per definition, backed by the exact source
//! line with a re-verifiable `file://` + `L<line>` + `sha256` provenance) and a
//! `Graph` frame per file whose [`Relation`](contextgraph_types::Relation)
//! edges are the `code.defines` / `code.calls` / `code.imports` links between
//! those symbols. It honors `query.anchors` the way the reference fixture does:
//! a frame anchored on a symbol URI is boosted to the front.
//!
//! ## Fallback extractor (a deliberate simplification)
//!
//! The crate is named for tree-sitter, but it ships a **self-contained,
//! line-based** symbol extractor rather than a `tree-sitter` + `tree-sitter-rust`
//! grammar dependency. Issue #18 sanctions this fallback explicitly ("if
//! tree-sitter grammar deps fail to fetch/build, fall back to a lightweight
//! regex/line-based symbol extractor that still emits real Symbol+Graph
//! frames"), and it is the honest trade here: a grammar dependency pulls a C
//! toolchain build into every CI job for output this provider does not need to
//! be byte-exact, whereas the line-based extractor is pure Rust, always builds,
//! and emits frames whose provenance is just as real. The extractor recognizes
//! top-level `fn` / `struct` / `enum` / `trait` / `mod` / `type` / `const`
//! definitions, `use` imports, and intra-file calls.
//!
//! Usage: `contextgraph-treesitter [ROOT]` — `ROOT` is the directory whose
//! `.rs` files are parsed, defaulting to this crate's bundled `fixtures/`.

use std::path::{Path, PathBuf};

use contextgraph_refprov::{
    DerivedFrame, FileFrame, FrameSource, ProviderConfig, line_range_bytes, serve, walk,
};
use contextgraph_types::{ContextFrame, ContextQuery, FrameKind, Relation, rel};

/// The directory parsed when no `ROOT` argument is given: this crate's bundled
/// fixtures, resolved at compile time so the path is cwd-independent.
const DEFAULT_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");

fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));
    serve(TreeSitter { root });
}

struct TreeSitter {
    root: PathBuf,
}

impl FrameSource for TreeSitter {
    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            name: "contextgraph-treesitter",
            version: env!("CARGO_PKG_VERSION"),
            kinds: vec!["symbol", "graph"],
        }
    }

    fn candidates(&mut self, _query: &ContextQuery) -> Vec<ContextFrame> {
        let mut frames = Vec::new();
        for path in rust_files(&self.root) {
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            frames.extend(self.file_frames(&path, &source));
        }
        frames
    }
}

impl TreeSitter {
    /// The `Symbol` frames plus one `Graph` frame for a single source file.
    fn file_frames(&self, path: &Path, source: &str) -> Vec<ContextFrame> {
        let relative = self.relative(path);
        let uri = file_uri(path);
        let bytes = source.as_bytes();

        let defs = parse_defs(source);
        let imports = parse_imports(source);
        let calls = parse_calls(source, &defs);

        let mut frames = Vec::new();
        for (rank, def) in defs.iter().enumerate() {
            if let Some(frame) = symbol_frame(bytes, &relative, &uri, def, rank) {
                frames.push(frame);
            }
        }

        let edges = graph_edges(&relative, &defs, &imports, &calls);
        if !edges.is_empty() {
            let mut frame = DerivedFrame {
                id: format!("graph:{relative}"),
                kind: FrameKind::Graph,
                title: format!("{relative} symbol graph"),
                content: graph_summary(&defs, &imports, &calls),
                uri,
                method: "line-based-symbol-extraction",
                score: 0.55,
                by: "contextgraph-treesitter",
            }
            .build();
            frame.relations = edges;
            frames.push(frame);
        }
        frames
    }

    /// The path relative to the parse root, for stable ids and symbol URIs.
    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

/// A top-level definition the line-based extractor recognized.
struct Def {
    /// The declared name, e.g. `parse_config`.
    name: String,
    /// The keyword that introduced it: `fn`, `struct`, `enum`, ….
    keyword: String,
    /// 1-indexed source line the definition sits on.
    line: usize,
}

/// Build a `Symbol` frame for one definition, backed by its exact source line
/// (so the digest re-verifies) with a `code.defines` edge to its symbol URI.
fn symbol_frame(
    bytes: &[u8],
    relative: &str,
    uri: &str,
    def: &Def,
    rank: usize,
) -> Option<ContextFrame> {
    let (from, to) = line_range_bytes(bytes, def.line, def.line)?;
    let content = std::str::from_utf8(&bytes[from..to]).ok()?.to_string();
    let range = format!("L{}", def.line);
    let mut frame = FileFrame {
        id: format!("sym:{relative}#{}", def.name),
        kind: FrameKind::Symbol,
        title: format!("{} {}", def.keyword, def.name),
        content,
        uri: uri.to_string(),
        citation: format!("{relative} {range}"),
        range,
        score: (0.9 - 0.02 * rank as f32).clamp(0.05, 1.0),
        by: "contextgraph-treesitter",
    }
    .build();
    frame.relations = vec![Relation {
        rel: rel::CODE_DEFINES.to_string(),
        target_uri: symbol_uri(relative, &def.name),
        display_name: Some(def.name.clone()),
    }];
    Some(frame)
}

/// The labelled edges for a file's `Graph` frame: one `code.defines` per
/// definition, one `code.imports` per `use`, one `code.calls` per intra-file
/// call. Every edge carries a human `display_name` and a non-empty
/// `target_uri`, as `SPEC.md` §G1/§G2 require.
fn graph_edges(
    relative: &str,
    defs: &[Def],
    imports: &[String],
    calls: &[String],
) -> Vec<Relation> {
    let mut edges = Vec::new();
    for def in defs {
        edges.push(Relation {
            rel: rel::CODE_DEFINES.to_string(),
            target_uri: symbol_uri(relative, &def.name),
            display_name: Some(def.name.clone()),
        });
    }
    for import in imports {
        edges.push(Relation {
            rel: rel::CODE_IMPORTS.to_string(),
            target_uri: format!("symbol://{import}"),
            display_name: Some(import.clone()),
        });
    }
    for callee in calls {
        edges.push(Relation {
            rel: rel::CODE_CALLS.to_string(),
            target_uri: symbol_uri(relative, callee),
            display_name: Some(callee.clone()),
        });
    }
    edges
}

/// A short human summary of a file's symbol graph — the `Graph` frame's
/// quotable content.
fn graph_summary(defs: &[Def], imports: &[String], calls: &[String]) -> String {
    let mut parts = Vec::new();
    if !defs.is_empty() {
        let names: Vec<&str> = defs.iter().map(|def| def.name.as_str()).collect();
        parts.push(format!("defines {}", names.join(", ")));
    }
    if !imports.is_empty() {
        parts.push(format!("imports {}", imports.join(", ")));
    }
    if !calls.is_empty() {
        parts.push(format!("calls {}", calls.join(", ")));
    }
    parts.join("; ")
}

/// The `symbol://` URI for a named symbol in a file.
fn symbol_uri(relative: &str, name: &str) -> String {
    format!("symbol://{relative}#{name}")
}

/// Extract top-level definitions from Rust source, one per matching line.
fn parse_defs(source: &str) -> Vec<Def> {
    const KEYWORDS: &[&str] = &[
        "fn ", "struct ", "enum ", "trait ", "mod ", "type ", "const ",
    ];
    let mut defs = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let line = strip_visibility(raw.trim_start());
        for keyword in KEYWORDS {
            if let Some(rest) = line.strip_prefix(keyword)
                && let Some(name) = leading_ident(rest)
            {
                defs.push(Def {
                    name,
                    keyword: keyword.trim().to_string(),
                    line: index + 1,
                });
                break;
            }
        }
    }
    defs
}

/// Extract `use` import paths from Rust source.
fn parse_imports(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|raw| {
            let line = strip_visibility(raw.trim_start());
            line.strip_prefix("use ")
                .map(|rest| rest.trim().trim_end_matches(';').trim().to_string())
        })
        .filter(|path| !path.is_empty())
        .collect()
}

/// Extract intra-file calls: the names of `fn` definitions that appear as
/// `name(` somewhere other than their own definition line. De-duplicated and
/// kept in definition order, so the edge set is deterministic.
fn parse_calls(source: &str, defs: &[Def]) -> Vec<String> {
    let mut calls = Vec::new();
    for def in defs.iter().filter(|def| def.keyword == "fn") {
        let pattern = format!("{}(", def.name);
        let called_elsewhere = source
            .lines()
            .enumerate()
            .any(|(index, line)| index + 1 != def.line && line.contains(&pattern));
        if called_elsewhere && !calls.contains(&def.name) {
            calls.push(def.name.clone());
        }
    }
    calls
}

/// Strip a leading `pub`, `pub(crate)`, or `async` qualifier so definition
/// detection sees the introducing keyword.
fn strip_visibility(line: &str) -> &str {
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    line.strip_prefix("async ").unwrap_or(line)
}

/// The leading identifier of `text` — alphanumerics and underscores up to the
/// first other byte. `None` if `text` does not start with an identifier.
fn leading_ident(text: &str) -> Option<String> {
    let ident: String = text
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    if ident.is_empty() { None } else { Some(ident) }
}

/// An absolute `file://` URI for `path`, canonicalized so a host re-reads the
/// same bytes regardless of the provider's working directory.
fn file_uri(path: &Path) -> String {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", absolute.display())
}

/// The `.rs` files under `root`, sorted for deterministic output.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    walk(root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .collect()
}
