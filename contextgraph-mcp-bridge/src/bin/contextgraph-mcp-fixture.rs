//! `contextgraph-mcp-fixture` — a tiny, hermetic MCP resource server
//! (issue #19, direction 1).
//!
//! It speaks just enough of the Model Context Protocol over stdio —
//! `initialize`, `resources/list`, `resources/read` (plus `ping`) — to stand in
//! for a real MCP server, so [`contextgraph-mcp-bridge`] and its CI job are
//! **self-contained**: no network, no `npx`, no external MCP install to point
//! the bridge at.
//!
//! Its resources are backed by real files under `fixtures/mcp/`, addressed by
//! absolute `file://` URIs (resolved from this crate's compile-time manifest
//! directory). Because the resource text it serves *is* those files' exact
//! bytes, the bridge's `file` provenance digests re-read and re-hash correctly
//! on the host side — which is what carries the bridge's conformance run all the
//! way to green (`SPEC.md` §6.2).
//!
//! MCP's stdio framing is one JSON-RPC 2.0 message per line.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

/// The MCP protocol revision this fixture reports at `initialize`.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// The directory holding the fixture's backing files, resolved at compile time
/// so the `file://` URIs are valid absolute paths wherever the binary runs.
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/mcp");

/// The resources this server serves: `(display name, filename, MIME type)`.
const RESOURCES: &[(&str, &str, &str)] = &[
    ("Deploy runbook", "deploy-runbook.md", "text/markdown"),
    ("Rollback policy", "rollback-policy.md", "text/markdown"),
    ("Health check probe", "health-check.rs", "text/x-rust"),
];

fn resource_uri(filename: &str) -> String {
    format!("file://{FIXTURE_DIR}/{filename}")
}

fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or broken pipe — the bridge is gone.
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            // A malformed line has no id to answer, so there is nothing to reply
            // to — MCP servers just ignore un-parseable input.
            Err(_) => continue,
        };

        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");

        // A message with no `id` is a notification: no reply is expected.
        let Some(id) = id else { continue };

        let reply = match method {
            "initialize" => ok(
                id,
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": { "resources": {} },
                    "serverInfo": { "name": "contextgraph-mcp-fixture", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
            "resources/list" => ok(id, json!({ "resources": resource_list() })),
            "resources/read" => match read_resource(&message) {
                Ok(result) => ok(id, result),
                Err(message) => error(id, -32602, &message),
            },
            "ping" => ok(id, json!({})),
            _ => error(id, -32601, &format!("method not found: {method}")),
        };
        write_line(&mut stdout, &reply);
    }
}

fn resource_list() -> Vec<Value> {
    RESOURCES
        .iter()
        .map(|(name, filename, mime)| {
            json!({
                "uri": resource_uri(filename),
                "name": name,
                "mimeType": mime,
            })
        })
        .collect()
}

fn read_resource(message: &Value) -> Result<Value, String> {
    let uri = message
        .get("params")
        .and_then(|p| p.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| "resources/read requires a `uri` param".to_string())?;

    let entry = RESOURCES
        .iter()
        .find(|(_, filename, _)| resource_uri(filename) == uri)
        .ok_or_else(|| format!("no such resource: {uri}"))?;
    let (_, filename, mime) = entry;

    let path = format!("{FIXTURE_DIR}/{filename}");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read `{path}`: {error}"))?;

    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": mime,
            "text": text,
        }],
    }))
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn write_line(stdout: &mut std::io::Stdout, message: &Value) {
    if let Ok(line) = serde_json::to_string(message) {
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}
