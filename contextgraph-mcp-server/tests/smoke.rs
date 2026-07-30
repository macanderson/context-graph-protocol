//! Smoke test: drive the `contextgraph-mcp-server` binary over stdio as a real
//! MCP host would (issue #19, direction 2). Spawns the process, completes the
//! MCP `initialize` handshake, calls the `query_context` tool, and asserts the
//! structured content carries frames, provenance, citations, and a budget audit.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

const SERVER: &str = env!("CARGO_BIN_EXE_contextgraph-mcp-server");

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn spawn() -> Self {
        let mut child = Command::new(SERVER)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn contextgraph-mcp-server");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.write(&message);
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read reply");
            assert_ne!(read, 0, "server closed stdout before replying to {method}");
            let value: Value = serde_json::from_str(line.trim()).expect("valid JSON-RPC reply");
            if value.get("id") == Some(&json!(id)) {
                return value;
            }
        }
    }

    fn notify(&mut self, method: &str) {
        self.write(&json!({ "jsonrpc": "2.0", "method": method }));
    }

    fn write(&mut self, message: &Value) {
        let line = serde_json::to_string(message).unwrap();
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn query_context_over_stdio_returns_frames_with_citations_and_a_budget_audit() {
    let mut server = Server::spawn();

    // MCP handshake.
    let init = server.request(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "smoke", "version": "0" } }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"],
        json!("contextgraph-mcp-server")
    );
    server.notify("notifications/initialized");

    // The tool is advertised.
    let listed = server.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == json!("query_context")));

    // Call it.
    let called = server.request(
        "tools/call",
        json!({ "name": "query_context", "arguments": { "goal": "how do retries and timeouts work", "budget": 2000 } }),
    );
    let result = &called["result"];
    assert_eq!(result["isError"], json!(false));

    let structured = &result["structuredContent"];
    let frames = structured["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "the tool returned no frames");
    for frame in frames {
        assert!(frame["citation"].as_str().is_some_and(|c| !c.is_empty()));
        assert!(!frame["provenance"].as_array().unwrap().is_empty());
    }

    let audit = &structured["budget_audit"];
    assert_eq!(audit["budget_requested"], json!(2000));
    assert_eq!(audit["within_budget"], json!(true));
    assert!(audit["budget_consumed"].as_u64().unwrap() > 0);

    // A human-readable content block accompanies the structured content.
    assert!(
        result["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("frame"))
    );
}
