//! `hoi4-imagegen` command-line tool.

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use imagegen_cli::{execute, Cli};
use imagegen_core::config::ProcessEnv;
use imagegen_core::hashing::SystemClock;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::UreqTransport;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let engine = match Engine::open(&cli.project_root, Arc::new(ProcessEnv), Arc::new(UreqTransport::new()), Arc::new(SystemClock), RunMode::Cli) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hoi4-imagegen: cannot open the project at {}: {e}", cli.project_root.display());
            return ExitCode::from(2);
        }
    };
    let code = execute(&engine, &cli, &mut std::io::stdout());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
