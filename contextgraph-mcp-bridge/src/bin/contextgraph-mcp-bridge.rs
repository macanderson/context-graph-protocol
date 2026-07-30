//! `contextgraph-mcp-bridge` — wrap an MCP resource server as a CGP provider
//! (issue #19, direction 1).
//!
//! The host spawns this program as a stdio CGP provider; this program in turn
//! spawns the wrapped MCP server named after `--`, so the two wires never cross:
//! CGP flows over *this* process's stdin/stdout, MCP over the child's.
//!
//! ```text
//! # wrap a local (non-egress) MCP server
//! contextgraph-mcp-bridge -- ./target/debug/contextgraph-mcp-fixture
//!
//! # wrap a remote MCP server: declares egress: true, gated behind consent
//! contextgraph-mcp-bridge --remote -- some-remote-mcp-server --flag
//!
//! # probe it end-to-end with the CGP inspector
//! contextgraph-inspect stdio -- ./target/debug/contextgraph-mcp-bridge \
//!     -- ./target/debug/contextgraph-mcp-fixture
//! ```

use clap::Parser;
use contextgraph_mcp_bridge::{BridgeConfig, run_stdio};
use contextgraph_types::EgressScope;

#[derive(Parser)]
#[command(
    name = "contextgraph-mcp-bridge",
    about = "Wrap an MCP resource server as a budgeted, cited, consent-gated Context Graph Protocol provider."
)]
struct Args {
    /// Declare the wrapped MCP server as off-machine: the bridge advertises
    /// `egress: true` with an off-machine scope, so a host gates it behind
    /// consent (`SPEC.md` §4). Omit for a local/filesystem MCP server.
    #[arg(long)]
    remote: bool,

    /// The off-machine egress scope to declare when `--remote` is set
    /// (e.g. `third-party-index`, `third-party-model`, or a namespaced
    /// `vendor:scope`). Ignored without `--remote`.
    #[arg(long, default_value = "third-party-index")]
    egress_scope: String,

    /// The MCP server command to wrap, after `--`: `<program> [args...]`.
    #[arg(last = true, required = true)]
    mcp_command: Vec<String>,
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let mut parts = args.mcp_command.into_iter();
    let program = match parts.next() {
        Some(program) => program,
        None => {
            eprintln!("contextgraph-mcp-bridge: no MCP server command given after `--`");
            return std::process::ExitCode::FAILURE;
        }
    };
    let config = BridgeConfig {
        program,
        args: parts.collect(),
        remote: args.remote,
        egress_scope: EgressScope::from_wire(args.egress_scope),
    };

    match run_stdio(&config) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("contextgraph-mcp-bridge: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
