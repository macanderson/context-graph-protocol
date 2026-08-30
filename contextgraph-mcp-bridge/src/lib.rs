//! `contextgraph-mcp-bridge` — the MCP → Context Graph Protocol bridge
//! (issue #19, direction 1).
//!
//! An MCP **client** wrapped as a CGP **provider**: it speaks just enough of the
//! Model Context Protocol (`initialize` + `resources/list` + `resources/read`)
//! to a wrapped MCP server, maps each MCP resource to a
//! [`ContextFrame`](contextgraph_types::ContextFrame), and answers CGP
//! `context/query`/`context/verify` over stdio — so every MCP resource server
//! becomes a **budgeted, cited, consent-gated** context source with zero changes
//! to it.
//!
//! ## What the mapping buys
//!
//! MCP hands an agent a blob of resource text and a URI. CGP asks for more, and
//! the bridge supplies it from what the MCP protocol already carries:
//!
//! - **provenance** — every frame records where it came from:
//!   `{ type: "mcp-resource", uri: <mcp uri>, by: <server name> }`. When the
//!   wrapped resource is a local `file://`, a second `file` provenance carries a
//!   real `sha256` digest a host can independently re-read and verify
//!   (`SPEC.md` §6.2).
//! - **honest token cost** — `token_cost` is the canonical byte count of the
//!   served content ([`budget_tokens`](contextgraph_types::budget_tokens)), so a
//!   host budgets the resource truthfully rather than guessing.
//! - **a relevance score** — a simple lexical overlap between the query and the
//!   resource, normalized into `[0, 1]`.
//! - **consent posture** — the transport-honesty rule applied transitively: a
//!   bridge wrapping a **remote** MCP server declares `egress: true` with an
//!   off-machine [`EgressScope`](contextgraph_types::EgressScope), so a host
//!   gates it behind consent exactly as it would any egress provider. A
//!   local/filesystem MCP server stays `egress: false`.
//!
//! ## Why the bridge is a *full* CGP provider
//!
//! "CGP conformant" means green on the whole conformance suite for the
//! capabilities you declare, and the suite treats a *skipped* check as a
//! non-pass. So the bridge does not cherry-pick: it negotiates `correlation`,
//! `graph`, `verify`, and an `embeddings_fingerprint`, and honors each — the
//! same surface the reference `contextgraph-example-docs` provider passes on.
//! The result is a bridge that is conformant, not merely functional.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use contextgraph_host::wire::Envelope;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQuery, ContextQueryResult, DataFlow, EgressScope, ErrorCode,
    FrameKind, FrameVerdict, PROTOCOL_VERSION, Provenance, ProviderInfo, Relation, Representation,
    Verdict, VerifyRequest, VerifyResponse, budget_tokens,
};
use contextgraph_types::{
    capability::QueryCapability, capability::fingerprint_dimensions, frame::rel,
};

/// The MCP protocol revision the bridge speaks to the wrapped server. MCP
/// negotiates the version at `initialize`; a server that answers a different one
/// still works here because the three methods the bridge uses are stable across
/// revisions.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// The embedding space the bridge declares it will accept query vectors in
/// (`<model-id>/<dimensions>[/<normalization>]`, `SPEC.md` §E1). The bridge
/// scores lexically rather than by vector similarity — as the reference provider
/// also does — but declaring the fingerprint lets it *reject* a query embedding
/// from a different space instead of scoring meaningless similarity.
const BRIDGE_FINGERPRINT: &str = "contextgraph-mcp-bridge/16/none";

// ─────────────────────────────── configuration ──────────────────────────────

/// How the bridge presents the wrapped MCP server's egress posture — the
/// transport-honesty rule applied transitively (`SPEC.md` §4).
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// The MCP server program to spawn and wrap.
    pub program: String,
    /// Arguments passed to that program.
    pub args: Vec<String>,
    /// Whether the wrapped MCP server is off-machine. `true` ⇒ the bridge
    /// declares `egress: true` with an off-machine scope and a host gates it
    /// behind consent; `false` ⇒ a local/filesystem server, `egress: false`.
    pub remote: bool,
    /// The off-machine [`EgressScope`] a remote server's content falls under.
    /// Ignored when `remote` is false.
    pub egress_scope: EgressScope,
}

impl BridgeConfig {
    /// A local (non-egress) bridge wrapping `program` with `args`.
    pub fn local(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            remote: false,
            egress_scope: EgressScope::ThirdPartyIndex,
        }
    }

    /// The [`DataFlow`] this configuration declares. A remote server's content
    /// leaves the machine (`egress: true` + an off-machine scope); a local one
    /// stays put (`egress: false` + `local-only`).
    pub fn data_flow(&self) -> DataFlow {
        if self.remote {
            DataFlow {
                reads: true,
                writes: false,
                egress: true,
                egress_scopes: vec![self.egress_scope.clone()],
            }
        } else {
            DataFlow {
                reads: true,
                writes: false,
                egress: false,
                egress_scopes: vec![EgressScope::LocalOnly],
            }
        }
    }
}

// ─────────────────────────────── the MCP client ─────────────────────────────

/// One resource the wrapped MCP server serves, after `resources/read`.
#[derive(Debug, Clone)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub text: String,
}

/// A minimal MCP client over a child process's stdio: JSON-RPC 2.0, one message
/// per line (MCP's stdio framing). It implements exactly the three methods the
/// bridge needs — `initialize`, `resources/list`, `resources/read` — by hand,
/// which is why the bridge carries no MCP SDK dependency.
pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    /// The wrapped server's declared name, from its `initialize` reply. Used as
    /// the `by` of every frame's provenance.
    pub server_name: String,
}

impl McpClient {
    /// Spawn the wrapped MCP server and complete the MCP `initialize` handshake.
    pub fn spawn(program: &str, args: &[String]) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The wrapped server's diagnostics flow to the bridge's own stderr,
            // never mistaken for a JSON-RPC message.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("could not spawn MCP server `{program}`: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP server has no stdin pipe".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP server has no stdout pipe".to_string())?;
        let mut client = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 0,
            server_name: "mcp-server".to_string(),
        };
        client.initialize()?;
        Ok(client)
    }

    fn initialize(&mut self) -> Result<(), String> {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "contextgraph-mcp-bridge", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        if let Some(name) = result
            .get("serverInfo")
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str)
        {
            self.server_name = name.to_string();
        }
        // MCP requires the client to confirm initialization before other calls.
        self.notify("notifications/initialized", json!({}))?;
        Ok(())
    }

    /// `resources/list` then `resources/read` for each, so the bridge holds the
    /// full text of every resource up front.
    pub fn fetch_resources(&mut self) -> Result<Vec<McpResource>, String> {
        let listed = self.request("resources/list", json!({}))?;
        let entries = listed
            .get("resources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut resources = Vec::new();
        for entry in entries {
            let Some(uri) = entry.get("uri").and_then(Value::as_str) else {
                continue;
            };
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| uri_basename(uri));
            let mime_type = entry
                .get("mimeType")
                .and_then(Value::as_str)
                .map(str::to_string);
            let text = self.read_resource(uri)?;
            resources.push(McpResource {
                uri: uri.to_string(),
                name,
                mime_type,
                text,
            });
        }
        Ok(resources)
    }

    fn read_resource(&mut self, uri: &str) -> Result<String, String> {
        let result = self.request("resources/read", json!({ "uri": uri }))?;
        // A resource can carry several content parts; the bridge concatenates the
        // text ones, which is what an agent would paste.
        let text = result
            .get("contents")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        Ok(text)
    }

    /// Issue a JSON-RPC request and return its `result`, skipping any
    /// notifications the server interleaves.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.write_message(&message)?;
        loop {
            let line = self.read_line()?;
            let value: Value = serde_json::from_str(line.trim_end())
                .map_err(|e| format!("MCP server sent invalid JSON-RPC: {e}"))?;
            // A reply carries our id; a notification carries none — skip it.
            if value.get("id") == Some(&json!(id)) {
                if let Some(error) = value.get("error") {
                    return Err(format!("MCP `{method}` failed: {error}"));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.write_message(&message)
    }

    fn write_message(&mut self, message: &Value) -> Result<(), String> {
        let line = serde_json::to_string(message).map_err(|e| e.to_string())?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|e| format!("could not write to MCP server: {e}"))
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) => Err("MCP server closed its output before replying".to_string()),
            Ok(_) => Ok(line),
            Err(e) => Err(format!("could not read from MCP server: {e}")),
        }
    }

    /// Close the connection and reap the child, so the wrapped server never
    /// outlives the bridge.
    pub fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Backstop for the `EOF from the host` path (a normal loop exit), where
        // `run_stdio` does not call `shutdown` explicitly.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ─────────────────────────── resource → frame mapping ───────────────────────

/// The last path segment of a URI, used as a fallback frame title/id seed when
/// a resource declares no name.
fn uri_basename(uri: &str) -> String {
    uri.rsplit(['/', '#', '?'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(uri)
        .to_string()
}

/// A stable, provider-scoped frame id derived from a resource URI.
fn frame_id_for(uri: &str) -> String {
    let slug: String = uri_basename(uri)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("frm_mcp_{slug}")
}

/// `sha256:<64 lowercase hex>` over `bytes` — the protocol's content-digest form
/// (`SPEC.md` §F5).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity("sha256:".len() + 64);
    out.push_str("sha256:");
    for byte in hash {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// Map a resource's MIME type to a CGP frame kind: code-ish content is a
/// `snippet`, everything else a `doc`. The bridge declares both kinds.
fn kind_for(mime_type: Option<&str>) -> FrameKind {
    match mime_type {
        Some(m)
            if m.contains("rust")
                || m.contains("javascript")
                || m.contains("python")
                || m.contains("typescript")
                || m.starts_with("text/x-")
                || m.starts_with("application/") =>
        {
            FrameKind::Snippet
        }
        _ => FrameKind::Doc,
    }
}

/// Build one **base** frame per MCP resource. The `score` is left at zero here
/// because it is query-dependent; [`answer_query`] fills it in per request.
///
/// Every frame carries `mcp-resource` provenance (which server, which URI), and
/// a `file` provenance with a real `sha256` when the resource is a local
/// `file://` — the digest a host re-reads to verify (`SPEC.md` §6.2).
pub fn build_frames(server_name: &str, resources: &[McpResource]) -> Vec<ContextFrame> {
    resources
        .iter()
        .map(|resource| build_frame(server_name, resource))
        .collect()
}

fn build_frame(server_name: &str, resource: &McpResource) -> ContextFrame {
    let content = resource.text.clone();
    let digest = sha256_hex(content.as_bytes());
    let kind = kind_for(resource.mime_type.as_deref());

    // Birth provenance: the MCP resource this frame came from, as the issue
    // specifies. Not `file` provenance, so §F5's byte re-read does not bind it.
    let mut provenance = vec![Provenance {
        kind: "mcp-resource".into(),
        uri: Some(resource.uri.clone()),
        range: None,
        digest: None,
        method: None,
        by: Some(server_name.to_string()),
    }];
    // A local file resource is independently re-readable, so it also gets `file`
    // provenance carrying the digest a host re-hashes. The served text *is* the
    // file's bytes for a filesystem MCP server, so the digest matches on re-read.
    if resource.uri.starts_with("file://") {
        provenance.push(Provenance {
            kind: "file".into(),
            uri: Some(resource.uri.clone()),
            range: None,
            digest: Some(digest.clone()),
            method: None,
            by: Some(server_name.to_string()),
        });
    }

    ContextFrame {
        id: frame_id_for(&resource.uri),
        kind,
        title: resource.name.clone(),
        content: Some(content.clone()),
        content_digest: Some(digest),
        uri: Some(resource.uri.clone()),
        representation: Representation::Full,
        content_fidelity: None,
        canonical_content_hash: None,
        content_ref: None,
        transform: None,
        minimum_content_fidelity: None,
        inline_content_requirement: None,
        score: 0.0,
        token_cost: budget_tokens(&content),
        canonical_token_cost: None,
        tokenizer_ref: None,
        valid_from: None,
        valid_to: None,
        recorded_at: None,
        provenance,
        citation_label: Some(format!("{} (mcp:{})", resource.name, server_name)),
        embedding: None,
        // A labelled edge so the bridge is a real graph provider: the resource
        // documents its own overview. §G4 anchors a query on either a frame's
        // `uri` or a relation `target_uri`.
        relations: vec![Relation {
            rel: rel::DOC_DOCUMENTS.into(),
            target_uri: format!("{}#overview", resource.uri),
            display_name: Some(format!("{} overview", resource.name)),
        }],
    }
}

// ──────────────────────────── provider behaviour ────────────────────────────

/// The bridge's declared identity + data-flow posture (`SPEC.md` §3).
pub fn provider_info(server_name: &str, config: &BridgeConfig) -> ProviderInfo {
    ProviderInfo {
        name: format!("contextgraph-mcp-bridge:{server_name}"),
        version: env!("CARGO_PKG_VERSION").into(),
        data_flow: config.data_flow(),
    }
}

/// The capabilities the bridge negotiates. It serves `doc`/`snippet` frames,
/// pipelines on `id` (`correlation`), carries labelled edges (`graph`), can
/// revalidate held frames (`verify`), and declares the embedding space it will
/// accept query vectors in.
pub fn capabilities() -> Capabilities {
    Capabilities {
        query: QueryCapability {
            kinds: vec!["doc".into(), "snippet".into()],
        },
        correlation: true,
        graph: true,
        embeddings_fingerprint: Some(BRIDGE_FINGERPRINT.into()),
        verify: true,
        representations: vec![],
        resolve: false,
    }
}

/// Whether a frame is anchored by any of `anchors` (`SPEC.md` §G4): its own
/// `uri` (zero hops) or any relation's `target_uri` (one hop).
fn is_anchored(frame: &ContextFrame, anchors: &[String]) -> bool {
    frame
        .uri
        .as_deref()
        .is_some_and(|u| anchors.iter().any(|a| a == u))
        || frame
            .relations
            .iter()
            .any(|r| anchors.contains(&r.target_uri))
}

/// A simple lexical relevance score in `[0, 1]`: the fraction of the query's
/// content words that appear in the frame's title or content, lifted off a `0.5`
/// baseline so a matched-nothing frame is still a candidate.
fn relevance(query: &ContextQuery, frame: &ContextFrame) -> f32 {
    let mut terms: Vec<String> = Vec::new();
    for source in [Some(query.goal.as_str()), query.query_text.as_deref()]
        .into_iter()
        .flatten()
    {
        for word in source.split(|c: char| !c.is_ascii_alphanumeric()) {
            if word.len() >= 3 {
                terms.push(word.to_ascii_lowercase());
            }
        }
    }
    if terms.is_empty() {
        return 0.5;
    }
    let haystack = format!(
        "{} {}",
        frame.title,
        frame.content.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    let matched = terms.iter().filter(|term| haystack.contains(*term)).count();
    let fraction = matched as f32 / terms.len() as f32;
    (0.5 + 0.5 * fraction).clamp(0.0, 1.0)
}

/// Answer a `context/query` from the cached base frames, honoring every part of
/// the query contract the host and conformance suite check: the `kinds` filter
/// (§Q1), an `as_of` pin (§6.1), anchor ranking (§G3/§G4), and both budget axes
/// — the token budget (§B1) and the `max_frames` cap (§B4).
pub fn answer_query(base_frames: &[ContextFrame], query: &ContextQuery) -> ContextQueryResult {
    // Score each candidate for this query.
    let mut candidates: Vec<ContextFrame> = base_frames
        .iter()
        .map(|frame| {
            let mut scored = frame.clone();
            scored.score = relevance(query, frame);
            scored
        })
        .collect();

    // §Q1: a non-empty `kinds` is a filter, not a hint.
    if !query.kinds.is_empty() {
        candidates.retain(|frame| query.kinds.contains(&frame.kind));
    }
    // §6.1: an `as_of` pin excludes content not yet true at the pinned instant.
    if let Some(as_of) = query.as_of.as_deref() {
        candidates.retain(|frame| frame.valid_from.as_deref().is_none_or(|vf| vf <= as_of));
    }

    // Rank: anchored first (§G3), then by relevance, then by id for stability.
    candidates.sort_by(|a, b| {
        is_anchored(b, &query.anchors)
            .cmp(&is_anchored(a, &query.anchors))
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.id.cmp(&b.id))
    });

    // Pack within both budgets: never overspend `max_tokens` (§B1) or
    // `max_frames` (§B4). A frame that would overflow the token budget is
    // skipped, not truncated silently mid-content.
    let eligible = candidates.len();
    let mut frames = Vec::new();
    let mut tokens = 0u64;
    for frame in candidates {
        if frames.len() as u32 >= query.max_frames {
            break;
        }
        let cost = frame.token_cost as u64;
        if tokens + cost > query.max_tokens as u64 {
            continue;
        }
        tokens += cost;
        frames.push(frame);
    }

    let truncated = frames.len() < eligible;
    ContextQueryResult {
        frames,
        truncated,
        dropped_estimate: None,
        ..Default::default()
    }
}

/// Answer a `context/verify` request honestly (`SPEC.md` §9, `docs/context-reuse.md`
/// §4): compare each presented digest against the one the bridge currently
/// serves for that frame. A digest that differs is exactly what a source that
/// moved on looks like from here, so it verifies `stale`.
pub fn verify_held(base_frames: &[ContextFrame], request: &VerifyRequest) -> VerifyResponse {
    let current: HashMap<&str, Option<&str>> = base_frames
        .iter()
        .map(|frame| (frame.id.as_str(), frame.content_digest.as_deref()))
        .collect();

    VerifyResponse::new(
        request
            .frames
            .iter()
            .map(|frame| {
                let verdict = match current.get(frame.frame_id.as_str()) {
                    // Never served, or no longer served.
                    None => Verdict::Gone,
                    // Served, but the bridge holds no digest to compare against.
                    Some(None) => Verdict::Unknown,
                    Some(Some(cur)) => match frame.content_digest.as_deref() {
                        None => Verdict::Unknown,
                        Some(presented) if presented == *cur => Verdict::Valid,
                        Some(_) => Verdict::Stale {
                            replacement_digest: Some((*cur).to_string()),
                        },
                    },
                };
                FrameVerdict::new(frame.clone(), verdict)
            })
            .collect(),
    )
}

/// The `bad_request` reply for a query embedding whose length contradicts the
/// bridge's declared fingerprint dimension (`SPEC.md` §E1), or `None` when the
/// query carries no embedding or one of the right length.
fn embedding_dimension_error(query: &ContextQuery, id: Option<String>) -> Option<Envelope> {
    let embedding = query.embedding.as_ref()?;
    let expected = fingerprint_dimensions(BRIDGE_FINGERPRINT)?;
    if embedding.len() == expected {
        return None;
    }
    Some(Envelope::Error {
        id,
        code: Some(ErrorCode::BadRequest),
        message: format!(
            "query embedding has {} dimensions; this bridge accepts {expected} ({BRIDGE_FINGERPRINT}) (§E1)",
            embedding.len()
        ),
    })
}

// ──────────────────────────────── the CGP loop ──────────────────────────────

/// Run the bridge: wrap the configured MCP server, then serve CGP over this
/// process's stdin/stdout until the host sends `shutdown` or closes the pipe.
///
/// The MCP resources are fetched once, up front, and cached — so composition
/// stays byte-stable across turns (`docs/context-reuse.md` §1) and a repeated
/// query does not re-hit the wrapped server.
pub fn run_stdio(config: &BridgeConfig) -> Result<(), String> {
    let mut mcp = McpClient::spawn(&config.program, &config.args)?;
    let resources = mcp.fetch_resources()?;
    let server_name = mcp.server_name.clone();
    let base_frames = build_frames(&server_name, &resources);
    let info = provider_info(&server_name, config);
    let caps = capabilities();

    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or broken pipe — the host is gone.
            Ok(_) => {}
        }

        let envelope = match serde_json::from_str::<Envelope>(line.trim_end()) {
            Ok(envelope) => envelope,
            Err(_) => {
                // A malformed line: stay alive and answer with a structured
                // `bad_request` (`SPEC.md` §R1).
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
                        provider: info.clone(),
                        capabilities: caps.clone(),
                        attester_keys: vec![],
                    },
                );
            }
            Envelope::Query { id, query } => {
                // §E1: reject a vector from a different embedding space rather
                // than scoring meaningless similarity.
                if let Some(error) = embedding_dimension_error(&query, id.clone()) {
                    write_envelope(&mut stdout, &error);
                    continue;
                }
                let result = answer_query(&base_frames, &query);
                // Echo the correlation id so the host can demultiplex (§H4).
                write_envelope(
                    &mut stdout,
                    &Envelope::Frames {
                        id,
                        result,
                        attestations: vec![],
                    },
                );
            }
            Envelope::Verify { request } => {
                write_envelope(
                    &mut stdout,
                    &Envelope::Verified {
                        response: verify_held(&base_frames, &request),
                    },
                );
            }
            Envelope::Shutdown => {
                // `process::exit` skips destructors, so reap the MCP child first.
                mcp.shutdown();
                std::process::exit(0);
            }
            // handshake_ack / frames / verified / error are host→provider-invalid
            // inputs; a provider ignores them.
            _ => {}
        }
    }
    Ok(())
}

/// Write one envelope as an NDJSON line, giving up quietly if the host is gone.
fn write_envelope(stdout: &mut std::io::Stdout, envelope: &Envelope) {
    if let Ok(line) = serde_json::to_string(envelope) {
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_resources() -> Vec<McpResource> {
        vec![
            McpResource {
                uri: "file:///tmp/deploy.md".into(),
                name: "Deploy runbook".into(),
                mime_type: Some("text/markdown".into()),
                text: "Roll out to canary, then staging, then production.".into(),
            },
            McpResource {
                uri: "https://example.test/mcp/policy".into(),
                name: "Rollback policy".into(),
                mime_type: Some("text/markdown".into()),
                text: "Roll back on an error rate above two percent.".into(),
            },
            McpResource {
                uri: "file:///tmp/health.rs".into(),
                name: "Health check".into(),
                mime_type: Some("text/x-rust".into()),
                text: "pub fn is_healthy() -> bool { true }".into(),
            },
        ]
    }

    fn query_with(goal: &str) -> ContextQuery {
        ContextQuery {
            goal: goal.into(),
            query_text: None,
            embedding: None,
            kinds: vec![],
            anchors: vec![],
            max_frames: 8,
            max_tokens: 4096,
            as_of: None,
            representation_preferences: vec![],
        }
    }

    #[test]
    fn every_frame_declares_its_honest_canonical_cost() {
        let frames = build_frames("srv", &sample_resources());
        assert_eq!(frames.len(), 3);
        for frame in &frames {
            assert!(frame.declares_honest_token_cost(), "{}", frame.id);
            assert!(frame.has_usable_content_digest());
            assert!(frame.representation_invariants().is_ok());
            assert!(frame.provenance_with_unusable_digests().is_empty());
        }
    }

    #[test]
    fn a_file_resource_gets_re_readable_file_provenance_a_remote_one_does_not() {
        let frames = build_frames("srv", &sample_resources());
        // The file:// resources carry a `file` provenance; the https one does not.
        let file_frame = frames.iter().find(|f| f.id.contains("deploy")).unwrap();
        assert!(file_frame.provenance.iter().any(|p| p.kind == "file"));
        assert!(
            file_frame
                .provenance
                .iter()
                .any(|p| p.kind == "mcp-resource")
        );

        let remote_frame = frames.iter().find(|f| f.id.contains("policy")).unwrap();
        assert!(!remote_frame.provenance.iter().any(|p| p.kind == "file"));
        assert!(
            remote_frame
                .provenance
                .iter()
                .any(|p| p.kind == "mcp-resource")
        );
    }

    #[test]
    fn the_content_digest_is_the_sha256_a_host_would_recompute() {
        let frames = build_frames("srv", &sample_resources());
        let frame = &frames[0];
        let expected = sha256_hex(frame.content.as_deref().unwrap().as_bytes());
        assert_eq!(frame.content_digest.as_deref(), Some(expected.as_str()));
        // The file provenance digest matches the content digest, so a host that
        // re-reads the bytes confirms both.
        let file_digest = frame
            .provenance
            .iter()
            .find(|p| p.kind == "file")
            .and_then(|p| p.digest.clone());
        assert_eq!(file_digest.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn a_kinds_filter_narrows_to_the_requested_kind() {
        let frames = build_frames("srv", &sample_resources());
        let mut query = query_with("deploy");
        query.kinds = vec![FrameKind::Doc];
        let result = answer_query(&frames, &query);
        assert!(!result.frames.is_empty());
        assert!(result.frames.iter().all(|f| f.kind == FrameKind::Doc));
    }

    #[test]
    fn an_anchored_query_ranks_the_anchored_frame_first() {
        let frames = build_frames("srv", &sample_resources());
        let anchor = frames[1].relations[0].target_uri.clone();
        let mut query = query_with("anything");
        query.anchors = vec![anchor.clone()];
        let result = answer_query(&frames, &query);
        assert!(is_anchored(&result.frames[0], &[anchor]));
    }

    #[test]
    fn the_budget_is_respected_even_when_more_frames_are_relevant() {
        let frames = build_frames("srv", &sample_resources());
        let mut query = query_with("roll out");
        query.max_frames = 1;
        let result = answer_query(&frames, &query);
        assert_eq!(result.frames.len(), 1);
        assert!(result.truncated);
        assert!(result.respects_budget(query.max_tokens));
        assert!(result.respects_frame_limit(query.max_frames));
    }

    #[test]
    fn scores_are_always_in_range_and_relevance_lifts_a_match() {
        let frames = build_frames("srv", &sample_resources());
        let result = answer_query(&frames, &query_with("rollback error rate"));
        for frame in &result.frames {
            assert!((0.0..=1.0).contains(&frame.score));
        }
        let policy = result.frames.iter().find(|f| f.id.contains("policy"));
        // "rollback"/"error"/"rate" all appear in the policy resource.
        assert!(policy.is_some_and(|f| f.score > 0.5));
    }

    #[test]
    fn verify_says_valid_for_served_digests_and_stale_for_mutated_ones() {
        let frames = build_frames("srv", &sample_resources());
        let served: Vec<_> = frames.iter().map(|f| f.identity("bridge")).collect();
        let unchanged = verify_held(&frames, &VerifyRequest::new(served.clone()));
        assert!(
            unchanged
                .verdicts
                .iter()
                .all(|v| matches!(v.verdict, Verdict::Valid))
        );

        let mutated: Vec<_> = served
            .iter()
            .map(|id| {
                contextgraph_types::FrameId::new(
                    "bridge",
                    id.frame_id.clone(),
                    id.content_digest.as_ref().map(|d| format!("{d}-mutated")),
                )
            })
            .collect();
        let changed = verify_held(&frames, &VerifyRequest::new(mutated));
        assert!(
            changed
                .verdicts
                .iter()
                .all(|v| matches!(v.verdict, Verdict::Stale { .. }))
        );
    }

    #[test]
    fn a_remote_config_declares_egress_a_local_one_does_not() {
        let local = BridgeConfig::local("x", vec![]);
        assert!(!local.data_flow().egress);
        assert!(local.data_flow().scopes_consistent());

        let mut remote = BridgeConfig::local("x", vec![]);
        remote.remote = true;
        assert!(remote.data_flow().egress);
        assert!(remote.data_flow().scopes_consistent());
        assert_eq!(remote.data_flow().off_machine_scopes().count(), 1);
    }
}
