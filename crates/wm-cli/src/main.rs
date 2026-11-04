//! `wm` — Wavemaker CLI entry point (WM-0111).

use clap::{Parser, Subcommand};
use wm_cli::commands::{self, ExitCode};
use wm_core::spectrum::Severity;

#[derive(Debug, Parser)]
#[command(name = "wm", version, about = "Wavemaker load-testing control plane")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a spectrum config file against the frozen semantics.
    Validate {
        /// Path to the spectrum YAML file.
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
    },
    /// Start a swell run (Phase 1 skeleton).
    Run,
    /// Query controller status (Phase 1 skeleton).
    Status,
    /// Stop a swell run (Phase 1 skeleton).
    Stop,
}

fn main() {
    wm_observability::init("wm-cli");
    let cli = Cli::parse();

    let code = match cli.command {
        Command::Validate { file } => match commands::validate(&file) {
            Err(e) => {
                eprintln!("error: {e:#}");
                ExitCode::Invalid as i32
            }
            Ok((_, violations)) => {
                if !violations.is_empty() {
                    println!("{}", commands::format_violations(&violations));
                }
                let has_errors = violations.iter().any(|v| v.severity == Severity::Error);
                if has_errors {
                    eprintln!("invalid: {} error(s)", violations.len());
                    ExitCode::Invalid as i32
                } else {
                    println!("valid");
                    ExitCode::Ok as i32
                }
            }
        },
        Command::Run => commands::run() as i32,
        Command::Status => commands::status() as i32,
        Command::Stop => commands::stop() as i32,
    };

    std::process::exit(code);
}
