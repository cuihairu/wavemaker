//! Subcommand implementations. `validate` checks a spectrum config file
//! (WM-0103); `run` / `status` / `stop` drive the controller's HTTP API
//! (WM-0111).

use std::fmt;
use std::path::Path;
use wm_core::spectrum::{Severity, SpectrumConfig, Violation};

/// Default controller HTTP endpoint (matches `ControllerConfig` defaults).
pub const DEFAULT_CONTROLLER: &str = "http://127.0.0.1:7600";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Ok = 0,
    /// Validation found error-severity issues.
    Invalid = 1,
    /// Controller unreachable, or it answered with an error status.
    Unavailable = 3,
}

/// Failure modes shared by the controller-facing subcommands.
#[derive(Debug)]
pub enum CommandError {
    /// The spectrum config failed validation; violations were already
    /// printed to stderr.
    Invalid,
    /// The controller could not be reached or returned a non-success
    /// status; carries a human-readable reason.
    Unavailable(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Invalid => write!(f, "spectrum config is invalid"),
            CommandError::Unavailable(msg) => write!(f, "controller unavailable: {msg}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Load and validate a spectrum config file; returns the parsed config (when
/// parseable) plus all explainable violations.
pub fn validate(path: &Path) -> anyhow::Result<(Option<SpectrumConfig>, Vec<Violation>)> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let cfg: SpectrumConfig = serde_yaml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("cannot parse {}: {e}", path.display()))?;
    let violations = cfg.validate();
    Ok((Some(cfg), violations))
}

// --- controller HTTP client ---------------------------------------------

/// Minimal typed view of a worker in `GET /api/v1/status`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub generation: u64,
    pub capabilities: Vec<String>,
    pub seconds_since_heartbeat: u64,
}

/// Typed view of a run record (`RunEntry` on the controller).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RunInfo {
    pub run_id: String,
    pub swell_id: String,
    pub spectrum_path: String,
    pub state: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct StatusInfo {
    pub status: String,
    pub protocol_version: u32,
    pub worker_count: usize,
    pub expired_workers: Vec<String>,
    pub workers: Vec<WorkerInfo>,
    pub runs: Vec<RunInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CreateRunBody<'a> {
    swell_id: Option<&'a str>,
    spectrum_path: &'a str,
}

/// Thin blocking HTTP client for the controller REST API.
#[derive(Debug, Clone)]
pub struct ControllerClient {
    base: String,
}

impl ControllerClient {
    pub fn new(base: impl Into<String>) -> Self {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self { base }
    }

    pub fn status(&self) -> Result<StatusInfo, CommandError> {
        let url = format!("{}/api/v1/status", self.base);
        let resp = ureq::get(&url).call().map_err(map_transport)?;
        decode(resp, &url)
    }

    /// Create a run record; the controller assigns the run id (and the
    /// swell id when the caller does not pin one).
    pub fn create_run(
        &self,
        spectrum_path: &str,
        swell_id: Option<&str>,
    ) -> Result<RunInfo, CommandError> {
        let url = format!("{}/api/v1/runs", self.base);
        let body = CreateRunBody {
            swell_id,
            spectrum_path,
        };
        let resp = ureq::post(&url)
            .send_json(&body)
            .map_err(|e| map_request(&url, e))?;
        decode(resp, &url)
    }

    /// Stop a run. Idempotent on the controller side.
    pub fn stop_run(&self, run_id: &str) -> Result<RunInfo, CommandError> {
        let url = format!("{}/api/v1/runs/{run_id}/stop", self.base);
        let resp = ureq::post(&url).call().map_err(|e| map_request(&url, e))?;
        decode(resp, &url)
    }
}

fn decode<T: serde::de::DeserializeOwned>(
    resp: ureq::Response,
    url: &str,
) -> Result<T, CommandError> {
    resp.into_json::<T>()
        .map_err(|e| CommandError::Unavailable(format!("bad response from {url}: {e}")))
}

fn map_transport(e: ureq::Error) -> CommandError {
    match e {
        ureq::Error::Status(code, resp) => {
            CommandError::Unavailable(format!("HTTP {code} {}", resp.status_text()))
        }
        ureq::Error::Transport(t) => CommandError::Unavailable(t.to_string()),
    }
}

/// Like `map_transport`, but a 404 on a run id is reported as such so
/// `wm stop nope` says "unknown run" instead of a generic HTTP error.
fn map_request(url: &str, e: ureq::Error) -> CommandError {
    match e {
        ureq::Error::Status(404, _) => CommandError::Unavailable(format!("unknown run at {url}")),
        other => map_transport(other),
    }
}

// --- subcommands --------------------------------------------------------

/// Create a run for `file` after validating it. Violations are printed
/// before returning `CommandError::Invalid`.
pub fn run(file: &Path, controller: &str, swell_id: Option<&str>) -> Result<RunInfo, CommandError> {
    let (_cfg, violations) =
        validate(file).map_err(|e| CommandError::Unavailable(format!("{e:#}")))?;
    if !violations.is_empty() {
        eprintln!("{}", format_violations(&violations));
    }
    if violations.iter().any(|v| v.severity == Severity::Error) {
        return Err(CommandError::Invalid);
    }

    ControllerClient::new(controller).create_run(&file.display().to_string(), swell_id)
}

/// Query controller status.
pub fn status(controller: &str) -> Result<StatusInfo, CommandError> {
    ControllerClient::new(controller).status()
}

/// Stop a run by id.
pub fn stop(run_id: &str, controller: &str) -> Result<RunInfo, CommandError> {
    ControllerClient::new(controller).stop_run(run_id)
}

/// Render a status payload for terminal output.
pub fn format_status(info: &StatusInfo) -> String {
    let mut out = format!(
        "controller: {} (protocol v{}, {} worker(s))\n",
        info.status, info.protocol_version, info.worker_count
    );
    for w in &info.workers {
        out.push_str(&format!(
            "  worker {} gen={} caps=[{}] heartbeat={}s ago\n",
            w.worker_id,
            w.generation,
            w.capabilities.join(","),
            w.seconds_since_heartbeat
        ));
    }
    if !info.expired_workers.is_empty() {
        out.push_str(&format!("  expired: {}\n", info.expired_workers.join(",")));
    }
    if info.runs.is_empty() {
        out.push_str("  runs: none\n");
    }
    for r in &info.runs {
        out.push_str(&format!(
            "  run {} swell={} state={} spectrum={}\n",
            r.run_id, r.swell_id, r.state, r.spectrum_path
        ));
    }
    out
}

/// Render violations for terminal output.
pub fn format_violations(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|v| {
            let sev = match v.severity {
                Severity::Error => "ERROR",
                Severity::Warning => "WARN ",
            };
            format!("{sev} {} {}: {}", v.code, v.field, v.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    const GOOD: &str = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nsaturation_policy: queue\n";

    #[test]
    fn valid_config_passes() {
        let f = write_tmp(GOOD);
        let (cfg, violations) = validate(f.path()).unwrap();
        let cfg = cfg.unwrap();
        assert_eq!(
            violations
                .iter()
                .filter(|v| v.severity == Severity::Error)
                .count(),
            0
        );
        assert!(cfg.is_valid());
    }

    #[test]
    fn invalid_config_reports_errors() {
        let f = write_tmp("lifetime:\n  distribution: fixed\n  mean_secs: 900\n");
        let (_cfg, violations) = validate(f.path()).unwrap();
        assert!(violations.iter().any(|v| v.code == "E_ARRIVAL_MISSING"));
        assert!(format_violations(&violations).contains("ERROR"));
    }

    #[test]
    fn unparseable_file_is_an_error() {
        let f = write_tmp("arrival: [broken");
        assert!(validate(f.path()).is_err());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(validate(Path::new("/nonexistent/spectrum.yaml")).is_err());
    }

    #[test]
    fn run_rejects_invalid_spectrum_before_contacting_controller() {
        let f = write_tmp("lifetime:\n  distribution: fixed\n  mean_secs: 900\n");
        // Point at a controller that does not exist: validation must fail
        // first, so the error is `Invalid`, not `Unavailable`.
        let err = run(f.path(), "http://127.0.0.1:1", None).unwrap_err();
        assert!(matches!(err, CommandError::Invalid), "got {err:?}");
    }

    #[test]
    fn unreachable_controller_is_unavailable() {
        let err = status("http://127.0.0.1:1").unwrap_err();
        assert!(matches!(err, CommandError::Unavailable(_)), "got {err:?}");
        let err = stop("run-1", "http://127.0.0.1:1").unwrap_err();
        assert!(matches!(err, CommandError::Unavailable(_)), "got {err:?}");
    }

    #[test]
    fn client_trims_trailing_slashes() {
        let c = ControllerClient::new("http://127.0.0.1:7600//");
        assert_eq!(c.base, "http://127.0.0.1:7600");
    }

    #[test]
    fn format_status_renders_workers_and_runs() {
        let info = StatusInfo {
            status: "running".into(),
            protocol_version: 1,
            worker_count: 1,
            expired_workers: vec!["w-old".into()],
            workers: vec![WorkerInfo {
                worker_id: "worker-a".into(),
                generation: 2,
                capabilities: vec!["native".into()],
                seconds_since_heartbeat: 3,
            }],
            runs: vec![RunInfo {
                run_id: "run-1".into(),
                swell_id: "swell-0001".into(),
                spectrum_path: "s.yaml".into(),
                state: "created".into(),
                created_at: 0,
            }],
        };
        let text = format_status(&info);
        assert!(text.contains("worker worker-a gen=2"));
        assert!(text.contains("expired: w-old"));
        assert!(text.contains("run run-1 swell=swell-0001 state=created"));
    }
}
