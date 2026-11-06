//! `wm` — Wavemaker CLI entry point (WM-0111).

use clap::{Parser, Subcommand};
use wm_cli::commands::{self, CommandError, ExitCode};
use wm_core::spectrum::Severity;

#[derive(Debug, Parser)]
#[command(name = "wm", version, about = "Wavemaker load-testing control plane")]
struct Cli {
    /// Controller HTTP endpoint (env: WM_CONTROLLER_URL).
    #[arg(
        long,
        global = true,
        default_value = commands::DEFAULT_CONTROLLER,
        env = "WM_CONTROLLER_URL"
    )]
    controller: String,

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
    /// Create a run from a validated spectrum config on the controller.
    Run {
        /// Path to the spectrum YAML file.
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
        /// Pin the swell id instead of letting the controller assign one.
        #[arg(long)]
        swell_id: Option<String>,
    },
    /// Show controller status: workers, leases, runs.
    Status,
    /// Stop a run by id (idempotent).
    Stop {
        /// Run id, e.g. `run-1`.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
    },
}

fn main() {
    wm_observability::init("wm-cli");
    let cli = Cli::parse();

    let code = match &cli.command {
        Command::Validate { file } => match commands::validate(file) {
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
                    eprintln!("invalid: {} issue(s)", violations.len());
                    ExitCode::Invalid as i32
                } else {
                    println!("valid");
                    ExitCode::Ok as i32
                }
            }
        },
        Command::Run { file, swell_id } => {
            match commands::run(file, &cli.controller, swell_id.as_deref()) {
                Ok(run) => {
                    println!(
                        "created {} swell={} spectrum={}",
                        run.run_id, run.swell_id, run.spectrum_path
                    );
                    ExitCode::Ok as i32
                }
                Err(e) => report(e),
            }
        }
        Command::Status => match commands::status(&cli.controller) {
            Ok(info) => {
                print!("{}", commands::format_status(&info));
                ExitCode::Ok as i32
            }
            Err(e) => report(e),
        },
        Command::Stop { run_id } => match commands::stop(run_id, &cli.controller) {
            Ok(run) => {
                println!("stopped {} state={}", run.run_id, run.state);
                ExitCode::Ok as i32
            }
            Err(e) => report(e),
        },
    };

    std::process::exit(code);
}

fn report(err: CommandError) -> i32 {
    eprintln!("error: {err}");
    match err {
        CommandError::Invalid => ExitCode::Invalid as i32,
        CommandError::Unavailable(_) => ExitCode::Unavailable as i32,
    }
}
