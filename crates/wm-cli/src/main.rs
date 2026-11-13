//! `wm` — Wavemaker CLI entry point (WM-0111).

use clap::{Parser, Subcommand};
use wm_cli::commands::{self, CommandError, ExitCode, TestRunOptions};
use wm_cli::runner::RunBackend;
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
    /// Declarative test plans: validate or execute one end to end.
    Test {
        #[command(subcommand)]
        command: TestCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TestCommand {
    /// Validate a test plan TOML file.
    Validate {
        /// Path to the plan TOML file.
        #[arg(value_name = "PLAN")]
        plan: std::path::PathBuf,
    },
    /// Run a test plan and write a report. The default backend is the
    /// in-process stub simulator (reports say so loudly); use
    /// `--backend controller` to drive a real controller.
    Run {
        /// Path to the plan TOML file.
        #[arg(value_name = "PLAN")]
        plan: std::path::PathBuf,
        /// Where to place the run directory (default: wm-runs/<name>-<ts>).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Execution backend: stub (simulator, default) or controller.
        #[arg(long, default_value = "stub")]
        backend: String,
        /// Sleep compression for the stub backend (1.0 = real time).
        #[arg(long, default_value_t = 1.0)]
        time_scale: f64,
        /// HTTP endpoint serving metrics snapshots (controller backend).
        #[arg(long)]
        basin_url: Option<String>,
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
        Command::Test { command } => match command {
            TestCommand::Validate { plan } => match commands::test_validate(plan) {
                Ok(_) => {
                    println!("valid");
                    ExitCode::Ok as i32
                }
                // The real reason was already printed by test_validate;
                // CommandError's Display names spectra, not plans.
                Err(CommandError::Invalid) => ExitCode::Invalid as i32,
                Err(CommandError::Unavailable(_)) => ExitCode::Unavailable as i32,
            },
            TestCommand::Run {
                plan,
                out,
                backend,
                time_scale,
                basin_url,
            } => {
                match RunBackend::parse(backend) {
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::Invalid as i32
                    }
                    Ok(backend) => {
                        let opts = TestRunOptions {
                            plan_path: plan.clone(),
                            backend,
                            controller: cli.controller.clone(),
                            basin_url: basin_url.clone(),
                            time_scale: *time_scale,
                            out_dir: out.clone(),
                        };
                        match commands::test_run(opts) {
                            Ok(outcome) => {
                                println!(
                                    "plan {} backend={} time_scale={}",
                                    outcome.plan_name, outcome.backend, outcome.time_scale
                                );
                                for s in &outcome.steps {
                                    let dur = s.finished_unix_ms.saturating_sub(s.started_unix_ms);
                                    println!("  {} ({}) {}ms", s.step_id, s.action, dur);
                                }
                                println!(
                                    "verdict: {}",
                                    match outcome.verdict {
                                        wm_cli::runner::Verdict::Passed => "passed",
                                        wm_cli::runner::Verdict::Failed => "failed",
                                    }
                                );
                                if let Some(dir) = &outcome.run_dir {
                                    println!("report: {dir}/report.md");
                                }
                                match outcome.verdict {
                                    wm_cli::runner::Verdict::Passed => ExitCode::Ok as i32,
                                    wm_cli::runner::Verdict::Failed => ExitCode::Failed as i32,
                                }
                            }
                            // The real reason was already printed by
                            // test_run; CommandError's Display names
                            // spectra, not plans.
                            Err(CommandError::Invalid) => ExitCode::Invalid as i32,
                            Err(CommandError::Unavailable(_)) => ExitCode::Unavailable as i32,
                        }
                    }
                }
            }
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
