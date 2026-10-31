//! `hoi4-imagegen-mcp`: the MCP stdio server.
//!
//! Standard output carries only protocol messages. Anything meant for a person goes to standard error.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use imagegen_core::config::ProcessEnv;
use imagegen_core::hashing::SystemClock;
use imagegen_mcp::server::ImageServer;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::UreqTransport;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

/// MCP server for Hearts of Iron IV graphical assets.
#[derive(Parser, Debug)]
#[command(name = "hoi4-imagegen-mcp", version, about)]
struct Args {
    /// The mod project folder. Everything the server reads or writes stays inside it.
    #[arg(long, env = "HOI4_IMAGEGEN_PROJECT_ROOT", default_value = ".")]
    project_root: PathBuf,
    /// How the server is run. Only a local stdio server is supported.
    #[arg(long, default_value = "local-stdio", value_parser = ["local-stdio"])]
    mode: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let engine = match Engine::open(&args.project_root, Arc::new(ProcessEnv), Arc::new(UreqTransport::new()), Arc::new(SystemClock), RunMode::LocalStdio) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hoi4-imagegen-mcp: cannot open the project at {}: {e}", args.project_root.display());
            return ExitCode::from(2);
        }
    };
    let server = ImageServer::new(Arc::new(engine));
    let service = match server.serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hoi4-imagegen-mcp: the MCP handshake failed: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = service.waiting().await {
        eprintln!("hoi4-imagegen-mcp: the session ended with an error: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
