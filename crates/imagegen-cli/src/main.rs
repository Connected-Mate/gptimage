//! `hoi4-imagegen` command-line tool.

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use imagegen_cli::{account, account_action, execute, Cli};
use imagegen_core::config::ProcessEnv;
use imagegen_core::hashing::SystemClock;
use imagegen_mcp::{Engine, RunMode};
use imagegen_providers::UreqTransport;

fn main() -> ExitCode {
    let cli = Cli::parse();
    // signing in and out concerns the user, not a mod project, so it runs without opening one
    if account_action(&cli.command).is_some() {
        return match account(&cli.command, &ProcessEnv, &SystemClock) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hoi4-imagegen: {e}");
                ExitCode::from(1)
            }
        };
    }
    let engine = match Engine::open(
        &cli.project_root,
        Arc::new(imagegen_mcp::keyenv::SetupKeyEnv::new(ProcessEnv)),
        Arc::new(UreqTransport::new()),
        Arc::new(SystemClock),
        RunMode::Cli,
    ) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hoi4-imagegen: cannot open the project at {}: {e}", cli.project_root.display());
            return ExitCode::from(2);
        }
    };
    let code = execute(&engine, &cli, &mut std::io::stdout());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
