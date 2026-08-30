//! `contextgraph-mcp-server` — the Context Graph Protocol → MCP server
//! (issue #19, direction 2).
//!
//! An MCP **server** exposing one tool, `query_context(goal, budget, kinds)`,
//! backed by a CGP [`Host`](contextgraph_host::Host). A call builds a
//! [`ContextQuery`](contextgraph_types::ContextQuery), fans it out with
//! [`Host::query_all`](contextgraph_host::Host::query_all), and returns the
//! result as MCP **structured content**: frames with their provenance and
//! citation labels intact, plus a budget audit. An agent that only speaks MCP
//! (Claude Code, etc.) gets CGP retrieval — and the frame-vs-blob difference
//! becomes directly visible in the tool output.
//!
//! The host here is wired to a small in-process example provider so the server
//! is self-contained; a real deployment would register its own providers (a code
//! graph, a docs index, an MCP bridge from direction 1) and change nothing else.
//!
//! MCP's stdio framing is one JSON-RPC 2.0 message per line.

use async_trait::async_trait;
use serde_json::{Value, json};

use contextgraph_host::{ContextProvider, FanOut, Host, HostError, ProviderResult};
use contextgraph_types::capability::QueryCapability;
use contextgraph_types::{
    Capabilities, ContextFrame, ContextQuery, ContextQueryResult, DataFlow, FrameKind, Provenance,
    ProviderInfo, Relation, budget_tokens, format_protocol_timestamp, frame::rel,
};

/// The name of the single tool this server exposes.
pub const TOOL_NAME: &str = "query_context";

/// The MCP protocol revision this server reports at `initialize`.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

// ─────────────────────────── the example provider ───────────────────────────

/// A tiny in-process CGP provider serving a couple of canned, cited frames — so
/// the server demonstrates the translation end to end without any external
/// dependency. A real host swaps this for its own providers.
pub struct ExampleProvider {
    info: ProviderInfo,
    capabilities: Capabilities,
    frames: Vec<ContextFrame>,
}

impl Default for ExampleProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ExampleProvider {
    pub fn new() -> Self {
        Self {
            info: ProviderInfo {
                name: "contextgraph-example-docs".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                data_flow: DataFlow {
                    reads: true,
                    writes: false,
                    egress: false,
                    egress_scopes: vec![],
                },
            },
            capabilities: Capabilities {
                query: QueryCapability {
                    kinds: vec!["doc".into(), "snippet".into()],
                },
                graph: true,
                ..Capabilities::default()
            },
            frames: canned_frames(),
        }
    }
}

/// The frames the example provider serves. Each carries provenance and a human
/// citation label, so the translation has something real to surface.
fn canned_frames() -> Vec<ContextFrame> {
    vec![
        cited_frame(
            "frm_retry",
            FrameKind::Doc,
            "Retry policy",
            "Retries use exponential backoff with full jitter, capped at five \
             attempts. A 429 is always retried; a 4xx other than 429 never is.",
            "docs/retry.md L1-12",
        ),
        cited_frame(
            "frm_timeout",
            FrameKind::Snippet,
            "Client timeout default",
            "const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);",
            "src/client.rs L44",
        ),
    ]
}

fn cited_frame(
    id: &str,
    kind: FrameKind,
    title: &str,
    content: &str,
    citation: &str,
) -> ContextFrame {
    let mut frame = ContextFrame::full(id, kind, title, content, 0.8, budget_tokens(content));
    frame.uri = Some(format!("context://example-docs/{id}"));
    frame.citation_label = Some(citation.into());
    frame.provenance = vec![Provenance {
        kind: "derivation".into(),
        uri: None,
        range: None,
        digest: None,
        method: Some("curated".into()),
        by: Some("contextgraph-example-docs".into()),
    }];
    frame.relations = vec![Relation {
        rel: rel::DOC_DOCUMENTS.into(),
        target_uri: format!("symbol://example-docs/{id}#overview"),
        display_name: Some(format!("{title} overview")),
    }];
    frame
}

#[async_trait]
impl ContextProvider for ExampleProvider {
    fn id(&self) -> &str {
        "example-docs"
    }
    fn info(&self) -> &ProviderInfo {
        &self.info
    }
    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    async fn query(&self, query: &ContextQuery) -> Result<ContextQueryResult, HostError> {
        let mut frames: Vec<ContextFrame> = self.frames.clone();
        if !query.kinds.is_empty() {
            frames.retain(|f| query.kinds.contains(&f.kind));
        }
        frames.truncate(query.max_frames as usize);
        Ok(ContextQueryResult {
            frames,
            truncated: false,
            dropped_estimate: None,
            ..Default::default()
        })
    }
}

// ─────────────────────────────── the server ────────────────────────────────

/// A CGP → MCP server: a [`Host`] plus the MCP request handlers over it.
pub struct McpServer {
    host: Host,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    /// A server whose host is wired to the in-process example provider.
    pub fn new() -> Self {
        Self::with_host(default_host())
    }

    /// A server over a caller-supplied host, so a real deployment can register
    /// its own providers.
    pub fn with_host(host: Host) -> Self {
        Self { host }
    }

    /// Handle one JSON-RPC request, returning the reply value — or `None` for a
    /// notification (no `id`), which expects no reply.
    pub async fn handle(&self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let reply = match method {
            "initialize" => ok(id, self.initialize_result()),
            "tools/list" => ok(id, json!({ "tools": [tool_descriptor()] })),
            "tools/call" => self.handle_tools_call(id, message).await,
            "ping" => ok(id, json!({})),
            _ => error(id, -32601, &format!("method not found: {method}")),
        };
        Some(reply)
    }

    fn initialize_result(&self) -> Value {
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "contextgraph-mcp-server", "version": env!("CARGO_PKG_VERSION") },
        })
    }

    async fn handle_tools_call(&self, id: Value, message: &Value) -> Value {
        let params = message.get("params");
        let name = params
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if name != TOOL_NAME {
            // MCP convention: an unknown tool is a tool error, not a protocol
            // error — the agent chose a bad tool, the transport is fine.
            return ok(id, tool_error(&format!("unknown tool: {name}")));
        }
        let arguments = params
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let query = match parse_query(&arguments) {
            Ok(query) => query,
            Err(message) => return ok(id, tool_error(&message)),
        };

        let fanout = self.host.query_all(&query).await;
        ok(id, self.tool_success(&query, &fanout))
    }

    fn tool_success(&self, query: &ContextQuery, fanout: &FanOut) -> Value {
        let structured = translate_fanout(query, fanout);
        let summary = human_summary(&structured);
        json!({
            "content": [{ "type": "text", "text": summary }],
            "structuredContent": structured,
            "isError": false,
        })
    }
}

/// A [`Host`] with the in-process example provider registered.
pub fn default_host() -> Host {
    let mut host = Host::new();
    host.register(Box::new(ExampleProvider::new()));
    host
}

/// The MCP tool descriptor for `tools/list`.
pub fn tool_descriptor() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Retrieve budgeted, cited context frames for a goal from a Context Graph \
                        Protocol host. Unlike a raw resource read, each frame carries provenance, \
                        an honest token cost, and a citation label.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "goal": { "type": "string", "description": "The task or turn goal driving retrieval." },
                "budget": { "type": "integer", "description": "Token budget for the returned frames (default 4096).", "minimum": 1 },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["snippet", "symbol", "fact", "doc", "memory", "episode", "graph"] },
                    "description": "Optional frame-kind filter."
                }
            },
            "required": ["goal"]
        }
    })
}

/// Build a [`ContextQuery`] from the tool arguments.
fn parse_query(arguments: &Value) -> Result<ContextQuery, String> {
    let goal = arguments
        .get("goal")
        .and_then(Value::as_str)
        .filter(|g| !g.trim().is_empty())
        .ok_or_else(|| "`goal` is required and must be a non-empty string".to_string())?
        .to_string();
    let max_tokens = arguments
        .get("budget")
        .and_then(Value::as_u64)
        .map(|b| b.min(u32::MAX as u64) as u32)
        .unwrap_or(4096);
    let kinds = match arguments.get("kinds") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(frame_kind_from_wire)
            .collect(),
        Some(_) => return Err("`kinds` must be an array of frame-kind strings".to_string()),
    };
    Ok(ContextQuery {
        goal: goal.clone(),
        query_text: Some(goal),
        embedding: None,
        kinds,
        anchors: vec![],
        max_frames: 8,
        max_tokens,
        as_of: None,
        representation_preferences: vec![],
    })
}

fn frame_kind_from_wire(kind: &str) -> Option<FrameKind> {
    match kind {
        "snippet" => Some(FrameKind::Snippet),
        "symbol" => Some(FrameKind::Symbol),
        "fact" => Some(FrameKind::Fact),
        "doc" => Some(FrameKind::Doc),
        "memory" => Some(FrameKind::Memory),
        "episode" => Some(FrameKind::Episode),
        "graph" => Some(FrameKind::Graph),
        _ => None,
    }
}

/// Translate a fan-out into the MCP tool's structured content: frames (with
/// provenance + citations), a per-provider outcome list, and a budget audit.
pub fn translate_fanout(query: &ContextQuery, fanout: &FanOut) -> Value {
    let frames: Vec<Value> = fanout
        .accepted_with_provider()
        .map(|(provider_id, frame)| frame_to_json(provider_id, frame))
        .collect();
    let citations: Vec<String> = fanout
        .accepted_frames()
        .map(|frame| {
            frame
                .citation_label
                .clone()
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| frame.title.clone())
        })
        .collect();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let report = fanout.usage_report(query, format_protocol_timestamp(now));

    let providers: Vec<Value> = fanout
        .outcomes
        .iter()
        .map(|outcome| {
            json!({
                "provider": outcome.provider_id,
                "outcome": outcome_label(&outcome.result),
            })
        })
        .collect();

    json!({
        "goal": query.goal,
        "frames": frames,
        "citations": citations,
        "providers": providers,
        "budget_audit": {
            "budget_requested": report.budget_requested,
            "budget_consumed": report.budget_consumed,
            "within_budget": report.within_budget(),
            "frames_served": frames.len(),
            "as_of": report.as_of,
        }
    })
}

fn frame_to_json(provider_id: &str, frame: &ContextFrame) -> Value {
    let provenance: Vec<Value> = frame.provenance.iter().map(provenance_to_json).collect();
    json!({
        "provider": provider_id,
        "id": frame.id,
        "kind": frame.kind,
        "title": frame.title,
        "citation": frame.citation_label.clone().unwrap_or_else(|| frame.title.clone()),
        "uri": frame.uri,
        "token_cost": frame.token_cost,
        "score": frame.score,
        "content": frame.content,
        "provenance": provenance,
    })
}

fn provenance_to_json(provenance: &Provenance) -> Value {
    json!({
        "type": provenance.kind,
        "uri": provenance.uri,
        "range": provenance.range,
        "digest": provenance.digest,
        "by": provenance.by,
    })
}

fn outcome_label(result: &ProviderResult) -> &'static str {
    match result {
        ProviderResult::Frames(_) => "frames",
        ProviderResult::BudgetLie { .. } => "dropped_budget_lie",
        ProviderResult::FrameFlood { .. } => "dropped_frame_flood",
        ProviderResult::ConsentRequired(_) => "consent_required",
        ProviderResult::ConsentScopeRequired { .. } => "consent_scope_required",
        ProviderResult::Failed(_) => "failed",
    }
}

/// A one-line human summary for the tool's text content block.
fn human_summary(structured: &Value) -> String {
    let frames = structured
        .get("frames")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let consumed = structured
        .get("budget_audit")
        .and_then(|b| b.get("budget_consumed"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let requested = structured
        .get("budget_audit")
        .and_then(|b| b.get("budget_requested"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    format!("Retrieved {frames} context frame(s), {consumed}/{requested} budget tokens.")
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_tool_returns_frames_with_provenance_and_citations() {
        let server = McpServer::new();
        let call = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": TOOL_NAME, "arguments": { "goal": "how do retries and timeouts work" } }
        });
        let reply = server.handle(&call).await.expect("a request gets a reply");
        let result = &reply["result"];
        assert_eq!(result["isError"], json!(false));

        let structured = &result["structuredContent"];
        let frames = structured["frames"].as_array().expect("frames array");
        assert_eq!(frames.len(), 2);

        // Every frame carries provenance and a citation — the CGP difference.
        for frame in frames {
            assert!(frame["citation"].as_str().is_some_and(|c| !c.is_empty()));
            assert!(!frame["provenance"].as_array().unwrap().is_empty());
        }
        let citations = structured["citations"].as_array().unwrap();
        assert_eq!(citations.len(), 2);

        // The budget audit is present and self-consistent.
        let audit = &structured["budget_audit"];
        assert_eq!(audit["within_budget"], json!(true));
        assert!(audit["budget_consumed"].as_u64().unwrap() > 0);
        assert_eq!(audit["frames_served"], json!(2));
    }

    #[tokio::test]
    async fn a_kinds_filter_narrows_the_returned_frames() {
        let server = McpServer::new();
        let call = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": TOOL_NAME, "arguments": { "goal": "timeouts", "kinds": ["snippet"] } }
        });
        let reply = server.handle(&call).await.unwrap();
        let frames = reply["result"]["structuredContent"]["frames"]
            .as_array()
            .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["kind"], json!("snippet"));
    }

    #[tokio::test]
    async fn a_missing_goal_is_a_tool_error_not_a_protocol_error() {
        let server = McpServer::new();
        let call = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": TOOL_NAME, "arguments": {} }
        });
        let reply = server.handle(&call).await.unwrap();
        assert_eq!(reply["result"]["isError"], json!(true));
    }

    #[tokio::test]
    async fn tools_list_advertises_query_context() {
        let server = McpServer::new();
        let call = json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" });
        let reply = server.handle(&call).await.unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], json!(TOOL_NAME));
    }

    #[tokio::test]
    async fn a_notification_gets_no_reply() {
        let server = McpServer::new();
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(server.handle(&note).await.is_none());
    }
}
