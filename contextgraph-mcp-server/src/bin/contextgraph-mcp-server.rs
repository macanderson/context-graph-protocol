//! `contextgraph-mcp-server` — expose a Context Graph Protocol host's fan-out as
//! an MCP `query_context` tool (issue #19, direction 2).
//!
//! An MCP host (Claude Code, etc.) spawns this program and calls its one tool;
//! each call runs `Host::query_all` and returns frames, provenance, citations,
//! and a budget audit as MCP structured content.
//!
//! ```text
//! contextgraph-mcp-server        # then speak MCP (JSON-RPC 2.0) over stdio
//! ```

use std::io::{BufRead, Write};

use serde_json::Value;

use contextgraph_mcp_server::McpServer;

fn main() {
    // A current-thread runtime: the MCP loop reads stdin synchronously and drives
    // the async `Host::query_all` per tool call via `block_on`.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("contextgraph-mcp-server: could not start runtime: {error}");
            std::process::exit(1);
        }
    };

    let server = McpServer::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF or broken pipe — the MCP host is gone.
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            // A malformed line carries no id to answer; ignore it.
            Err(_) => continue,
        };

        if let Some(reply) = runtime.block_on(server.handle(&message))
            && let Ok(encoded) = serde_json::to_string(&reply)
        {
            let _ = writeln!(stdout, "{encoded}");
            let _ = stdout.flush();
        }
    }
}
