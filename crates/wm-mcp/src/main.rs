//! Wavemaker MCP server (WM-MCP-001): the official rmcp SDK over stdio,
//! exposing the controller and the spectrum engine to AI agents.
//!
//! Tool surface (seven, per WM-MCP-001 plus the WM-TP-2 plan tools):
//!
//! - `list_runs`: forward `GET /api/v1/runs/{run_id}` for one run, or list
//!   every run from the status payload (the controller has no dedicated
//!   list route — `GET /api/v1/status` embeds the runs; existing contract in
//!   `crates/wm-controller/src/http.rs`).
//! - `query_status`: forward `GET /api/v1/status` verbatim.
//! - `validate_spectrum`: run `wm_cli::commands::validate` — the exact
//!   engine and rendering behind `wm validate`; no second implementation.
//! - `load_timeline`: the planned-load curve, same-source as the desktop
//!   command (wm-runtime `ArrivalPlan` + `rate_at`/`cumulative`).
//! - `run_scenario`: start/stop through `wm_cli::commands::run` / `stop` —
//!   a library call, because that module *is* the CLI's subcommand face
//!   (`wm run` / `wm stop` execute exactly this code), it returns typed
//!   errors instead of stdout to parse, and it does not require the `wm`
//!   binary to be installed.
//! - `run_test_plan`: execute a declarative test plan TOML end to end with
//!   the `wm test run` engine (`wm_cli::plan` + `wm_cli::runner`); writes
//!   the run directory (plan, timeline, asserts, metrics, report.md).
//! - `get_test_report`: read a finished plan run back (run.json outcome +
//!   report.md), defaulting to the latest run under `wm-runs`.
//!
//! Honest boundaries:
//!
//! - `run_test_plan` on the stub backend is a simulation: metrics are
//!   synthetic and every stub report says so loudly. The controller backend
//!   drives real runs, but metrics capture has no engine-side source yet
//!   (WM-0412 / WM-0413 unimplemented), so it errors without `basin_url`
//!   instead of fabricating numbers — there is deliberately no
//!   metrics/report tool for live runs.
//! - The controller address defaults to `http://127.0.0.1:7600` and can be
//!   overridden per call or via `WM_CONTROLLER_URL` (same env as the CLI).
//! - Tool failures are tool-level errors (`is_error: true`) carrying the
//!   named reason — never a silent fake success.
//!
//! stdio discipline: stdout carries JSON-RPC frames only; diagnostics go to
//! stderr (violations printed by the plan engine already land there).

mod forward;
mod timeline;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_router, ErrorData, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use wm_cli::commands::{self, format_violations};
use wm_cli::runner::{production_io, run_plan, RunBackend, RunOptions};
use wm_core::spectrum::{Severity, SpectrumConfig};

/// Bridge the native layer (`Result<Value, String>`) onto the MCP tool
/// result: `Err` text is forwarded verbatim as a tool-level error, never
/// rewritten into a success shape.
fn to_tool_result(v: Result<Value, String>) -> Result<CallToolResult, ErrorData> {
    match v {
        Ok(v) => {
            // serde_json cannot fail on an acyclic Value; a failure would be
            // a broken internal invariant, so it takes the honest internal
            // error path instead of returning an empty success.
            let text = serde_json::to_string(&v).map_err(|e| {
                ErrorData::internal_error(format!("result serialization failed: {e}"), None)
            })?;
            Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
        }
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e)])),
    }
}

// —— native helper layer (testable without the MCP protocol) ——

/// Forward the status payload or a single run. `None` run id lists every
/// run from the status payload (contract: runs are embedded there).
fn list_runs_native(run_id: Option<String>, controller: Option<String>) -> Result<Value, String> {
    let base = forward::controller_base(controller.as_deref());
    match run_id {
        Some(id) => {
            let body = match forward::get(&base, &format!("/api/v1/runs/{id}")) {
                Ok(body) => body,
                // Same wording as the CLI's 404 mapping for stop.
                Err(e) if e.starts_with("HTTP 404") => return Err(format!("unknown run: {id}")),
                Err(e) => return Err(e),
            };
            serde_json::from_str(&body).map_err(|e| format!("bad response from controller: {e}"))
        }
        None => {
            let body = forward::get(&base, "/api/v1/status")?;
            let status: Value = serde_json::from_str(&body)
                .map_err(|e| format!("bad response from controller: {e}"))?;
            let runs = status
                .get("runs")
                .ok_or("contract drift: status payload has no runs array")?;
            if !runs.is_array() {
                return Err("contract drift: status payload runs field is not an array".into());
            }
            Ok(runs.clone())
        }
    }
}

/// Forward the full status payload verbatim.
fn query_status_native(controller: Option<String>) -> Result<Value, String> {
    let base = forward::controller_base(controller.as_deref());
    let body = forward::get(&base, "/api/v1/status")?;
    serde_json::from_str(&body).map_err(|e| format!("bad response from controller: {e}"))
}

/// Validate a spectrum file with the `wm validate` engine: parse, run
/// `SpectrumConfig::validate()`, and render violations with the CLI's own
/// formatter. Error-severity issues are a tool-level error (the CLI exits
/// non-zero); warnings still validate.
fn validate_spectrum_native(path: &str) -> Result<Value, String> {
    let (cfg, violations) = commands::validate(Path::new(path)).map_err(|e| format!("{e:#}"))?;
    let report = format_violations(&violations);
    let has_errors = violations.iter().any(|v| v.severity == Severity::Error);
    if has_errors {
        return Err(format!("{report}\ninvalid: {} issue(s)", violations.len()));
    }
    let cfg: SpectrumConfig = cfg.unwrap_or_default();
    Ok(json!({
        "path": path,
        "valid": true,
        "warnings": violations.len(),
        "violations": report,
        "config": cfg,
    }))
}

/// Start or stop a scenario through the CLI's subcommand face. `start`
/// validates the spectrum first (refusing an invalid config before
/// contacting the controller, exactly like `wm run`); the violation report
/// is on the server's stderr and `validate_spectrum` can render it on
/// demand.
fn run_scenario_native(
    action: &str,
    path: Option<String>,
    swell_id: Option<String>,
    run_id: Option<String>,
    controller: Option<String>,
) -> Result<Value, String> {
    let base = forward::controller_base(controller.as_deref());
    match action {
        "start" => {
            let path = path
                .as_deref()
                .ok_or("parameter error: action start requires path (spectrum YAML file)")?;
            let info = commands::run(Path::new(path), &base, swell_id.as_deref())
                .map_err(|e| e.to_string())?;
            serde_json::to_value(info).map_err(|e| format!("result serialization failed: {e}"))
        }
        "stop" => {
            let run_id = run_id
                .as_deref()
                .ok_or("parameter error: action stop requires run_id")?;
            let info = commands::stop(run_id, &base).map_err(|e| e.to_string())?;
            serde_json::to_value(info).map_err(|e| format!("result serialization failed: {e}"))
        }
        other => Err(format!(
            "parameter error: unknown action {other} (start | stop)"
        )),
    }
}

/// Execute a declarative test plan end to end with the `wm test run`
/// engine: load → validate → backend → state machine → run directory +
/// report.md. The plan/violation rendering matches the CLI (stderr) and
/// error reasons come back in-band. Verdict `failed` is a SUCCESS result
/// carrying `verdict: "failed"` — the plan ran; verification did not.
fn run_test_plan_native(
    path: &str,
    backend: Option<String>,
    time_scale: Option<f64>,
    out: Option<String>,
    controller: Option<String>,
    basin_url: Option<String>,
) -> Result<Value, String> {
    let backend = RunBackend::parse(backend.as_deref().unwrap_or("stub"))
        .map_err(|e| format!("parameter error: {e}"))?;
    let time_scale = time_scale.unwrap_or(1.0);
    let base = forward::controller_base(controller.as_deref());
    let (mut plan, raw, sha) = wm_cli::plan::load_plan(Path::new(path))?;
    // Relative spectrum paths anchor at the plan file's directory.
    if let Some(dir) = Path::new(path).parent() {
        wm_cli::plan::resolve_spectra(&mut plan, dir);
    }
    let violations = plan.validate();
    if !violations.is_empty() {
        let report = wm_cli::plan::format_plan_violations(&violations);
        eprintln!("{report}");
        if violations.iter().any(|v| v.severity == Severity::Error) {
            return Err(format!(
                "{report}\ninvalid plan: {} issue(s)",
                violations.len()
            ));
        }
    }
    let out_dir = out
        .map(PathBuf::from)
        .unwrap_or_else(|| commands::default_run_dir(&plan.meta.name));
    let opts = RunOptions {
        backend,
        controller: base,
        basin_url,
        time_scale,
        out_dir: Some(out_dir),
    };
    let mut io = production_io(&opts, opts.out_dir.as_deref());
    let outcome = run_plan(&plan, &opts, io.as_mut(), &raw, &sha)?;
    let mut v =
        serde_json::to_value(&outcome).map_err(|e| format!("result serialization failed: {e}"))?;
    if let Some(dir) = &outcome.run_dir {
        v["report_path"] = Value::String(format!("{dir}/report.md"));
    }
    Ok(v)
}

/// Read a finished plan run back: run.json (the structured outcome the
/// machine wrote) plus report.md (the rendered view). Without `run_dir`,
/// the latest run directory under `root` (default `wm-runs`) is returned.
fn get_test_report_native(run_dir: Option<String>, root: Option<String>) -> Result<Value, String> {
    let dir = match run_dir {
        Some(d) => PathBuf::from(d),
        None => latest_run_dir(root.as_deref().unwrap_or("wm-runs"))?,
    };
    let run_path = dir.join("run.json");
    let run_json = std::fs::read_to_string(&run_path).map_err(|e| {
        format!(
            "cannot read {}: {e} (is this a `wm test run` directory?)",
            run_path.display()
        )
    })?;
    let run: Value = serde_json::from_str(&run_json)
        .map_err(|e| format!("bad run.json in {}: {e}", dir.display()))?;
    let report_path = dir.join("report.md");
    let report = std::fs::read_to_string(&report_path)
        .map_err(|e| format!("cannot read {}: {e}", report_path.display()))?;
    Ok(json!({
        "run_dir": dir.display().to_string(),
        "run": run,
        "report_markdown": report,
    }))
}

/// Latest subdirectory of `root` containing a run.json. Run directory
/// names embed a timestamp but the slug varies, so modification time
/// decides.
fn latest_run_dir(root: &str) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(root).map_err(|e| {
        format!(
            "cannot list {root}: {e} (no runs yet? `wm test run` writes to wm-runs/<name>-<ts>)"
        )
    })?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() || !p.join("run.json").is_file() {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, p));
        }
    }
    best.map(|(_, p)| p)
        .ok_or_else(|| format!("no test plan run directories under {root}"))
}

// —— MCP tool layer (parameter structs + rmcp macros; field docs are the
//    inputSchema descriptions) ——

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListRunsParams {
    /// Optional run id: forward `GET /api/v1/runs/{run_id}` for that single
    /// run. Omit to list every run (from the status payload; the controller
    /// has no dedicated list endpoint).
    #[serde(default)]
    #[schemars(default)]
    pub run_id: Option<String>,
    /// Controller base URL; overrides WM_CONTROLLER_URL and the default
    /// http://127.0.0.1:7600.
    #[serde(default)]
    #[schemars(default)]
    pub controller: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryStatusParams {
    /// Controller base URL; overrides WM_CONTROLLER_URL and the default
    /// http://127.0.0.1:7600.
    #[serde(default)]
    #[schemars(default)]
    pub controller: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ValidateSpectrumParams {
    /// Path to the spectrum YAML file (read-only; the same input `wm validate`
    /// takes).
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LoadTimelineParams {
    /// Spectrum YAML document text (the same input the desktop
    /// load_timeline command takes). Needs an `arrival.ramp` section.
    pub content: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunScenarioParams {
    /// start (create a run from a spectrum file) or stop (stop a run by id).
    pub action: String,
    /// start: path to the spectrum YAML file (validated before the run is
    /// created; an invalid config is refused with a tool-level error — call
    /// validate_spectrum for the report).
    #[serde(default)]
    #[schemars(default)]
    pub path: Option<String>,
    /// start: optional caller-pinned swell id.
    #[serde(default)]
    #[schemars(default)]
    pub swell_id: Option<String>,
    /// stop: run id, e.g. `run-1`.
    #[serde(default)]
    #[schemars(default)]
    pub run_id: Option<String>,
    /// Controller base URL; overrides WM_CONTROLLER_URL and the default
    /// http://127.0.0.1:7600.
    #[serde(default)]
    #[schemars(default)]
    pub controller: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunTestPlanParams {
    /// Path to the test plan TOML file (the same input `wm test run`
    /// takes).
    pub path: String,
    /// Execution backend: stub (in-process simulator, the default — its
    /// metrics are synthetic and every stub report says so) or controller
    /// (drives a real controller; capture then needs basin_url, otherwise
    /// it errors instead of fabricating metrics).
    #[serde(default)]
    #[schemars(default)]
    pub backend: Option<String>,
    /// Sleep compression for the stub backend, 0.0–1.0 (1.0 = real time).
    /// The controller backend only accepts 1.0.
    #[serde(default)]
    #[schemars(default)]
    pub time_scale: Option<f64>,
    /// Where to place the run directory (default: wm-runs/<name>-<ts>).
    #[serde(default)]
    #[schemars(default)]
    pub out: Option<String>,
    /// Controller base URL (controller backend only); overrides
    /// WM_CONTROLLER_URL and the default http://127.0.0.1:7600.
    #[serde(default)]
    #[schemars(default)]
    pub controller: Option<String>,
    /// HTTP endpoint serving metrics snapshots (controller backend only).
    #[serde(default)]
    #[schemars(default)]
    pub basin_url: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTestReportParams {
    /// Path to the run directory written by run_test_plan (or `wm test
    /// run`). Omit to read the latest run under `root`.
    #[serde(default)]
    #[schemars(default)]
    pub run_dir: Option<String>,
    /// Root directory scanned when run_dir is omitted (default wm-runs).
    #[serde(default)]
    #[schemars(default)]
    pub root: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct WavemakerMcp;

#[tool_router(server_handler)]
impl WavemakerMcp {
    #[tool(
        description = "List wavemaker runs, or fetch one run by id. Without run_id: every run is returned, taken from GET /api/v1/status (the controller has no dedicated list endpoint). With run_id: forwards GET /api/v1/runs/{run_id}. The controller JSON is forwarded, not re-modeled."
    )]
    fn list_runs(
        &self,
        Parameters(ListRunsParams { run_id, controller }): Parameters<ListRunsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(list_runs_native(run_id, controller))
    }

    #[tool(
        description = "Query wavemaker controller status: forwards GET /api/v1/status verbatim (status, protocol_version, workers with heartbeats, expired workers, and the run list)."
    )]
    fn query_status(
        &self,
        Parameters(QueryStatusParams { controller }): Parameters<QueryStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(query_status_native(controller))
    }

    #[tool(
        description = "Validate a wavemaker spectrum YAML file with the same engine as `wm validate` (wm-core SpectrumConfig::validate). Returns the parsed config plus any warnings; error-severity issues come back as a tool-level error carrying the CLI-formatted violation report."
    )]
    fn validate_spectrum(
        &self,
        Parameters(ValidateSpectrumParams { path }): Parameters<ValidateSpectrumParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(validate_spectrum_native(&path))
    }

    #[tool(
        description = "Planned-load timeline for a spectrum YAML document: arrival rate λ(t) and expected cumulative arrivals over the ramp-plus-hold window (ramp + one ramp length; one hour for constant rate). Same engine as the runtime executes (wm-runtime ArrivalPlan) and the same math as the desktop timeline chart. 121 sample points."
    )]
    fn load_timeline(
        &self,
        Parameters(LoadTimelineParams { content }): Parameters<LoadTimelineParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(
            timeline::load_timeline(&content)
                .map(|tl| serde_json::to_value(tl).unwrap_or(Value::Null)),
        )
    }

    #[tool(
        description = "Start or stop a wavemaker load scenario via the controller (the same code path as `wm run` / `wm stop`). action=start: requires path (spectrum YAML file, validated first; invalid configs are refused), optional swell_id; returns the created run {run_id, swell_id, state}. action=stop: requires run_id; idempotent. This creates controller run records; it does not deploy workers."
    )]
    fn run_scenario(
        &self,
        Parameters(RunScenarioParams {
            action,
            path,
            swell_id,
            run_id,
            controller,
        }): Parameters<RunScenarioParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(run_scenario_native(
            &action, path, swell_id, run_id, controller,
        ))
    }

    #[tool(
        description = "Execute a declarative wavemaker test plan TOML end to end (the same engine as `wm test run`): drives the plan's steps (prepare/inject_load/hold/step_load/capture/assert/cooldown/report) against the chosen backend, judges each assertion PASS/FAIL/NA, and writes a run directory (plan, event timeline, metrics, assertion verdicts, report.md). backend=stub (default) runs an in-process simulator — metrics are synthetic and the report labels them so; backend=controller drives a real controller. A `failed` verdict is a successful call: the plan ran but verification did not pass (exit 4 for the CLI)."
    )]
    fn run_test_plan(
        &self,
        Parameters(RunTestPlanParams {
            path,
            backend,
            time_scale,
            out,
            controller,
            basin_url,
        }): Parameters<RunTestPlanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(run_test_plan_native(
            &path, backend, time_scale, out, controller, basin_url,
        ))
    }

    #[tool(
        description = "Fetch a finished wavemaker test plan run: the structured outcome (run.json — verdict, steps, assertion verdicts, snapshots, loads) and the rendered Markdown report (report.md). Without run_dir, the latest run directory under wm-runs is returned. These directories are what run_test_plan (or `wm test run`) writes."
    )]
    fn get_test_report(
        &self,
        Parameters(GetTestReportParams { run_dir, root }): Parameters<GetTestReportParams>,
    ) -> Result<CallToolResult, ErrorData> {
        to_tool_result(get_test_report_native(run_dir, root))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdio transport: stdout carries MCP frames only; this server prints
    // no diagnostics of its own.
    let service = WavemakerMcp.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;
    use wm_controller::http::router as controller_router;
    use wm_controller::state::ControllerState;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    const GOOD: &str = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nsaturation_policy: queue\n";

    /// Bind the controller HTTP API on an ephemeral port; returns its base
    /// URL. The client (ureq) is blocking and the server is async, so the
    /// server gets its own thread + runtime (cli_stack.rs pattern).
    fn spawn_controller() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let app = controller_router(Arc::new(ControllerState::new(5000)));
                axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                    .await
                    .unwrap();
            });
        });

        format!("http://{addr}")
    }

    // —— validate_spectrum ——

    #[test]
    fn validate_reports_valid_config_with_engine_output() {
        let f = write_tmp(GOOD);
        let v = validate_spectrum_native(f.path().to_str().unwrap()).unwrap();
        // Warnings still validate (the CLI prints "valid"): this config
        // carries exactly W_POLICY_WITHOUT_CAP, an uncapped saturation
        // policy notice.
        assert_eq!(v["valid"], true);
        assert_eq!(v["warnings"], 1);
        assert!(
            v["violations"]
                .as_str()
                .unwrap()
                .contains("W_POLICY_WITHOUT_CAP"),
            "{}",
            v
        );
        assert!(v["config"].is_object(), "{}", v);
        // The parsed config is the engine's own view (round-trips the YAML).
        assert!(v["config"]["arrival"].is_object());
    }

    #[test]
    fn validate_reports_violations_verbatim_on_invalid() {
        let f = write_tmp("lifetime:\n  distribution: fixed\n  mean_secs: 900\n");
        let err = validate_spectrum_native(f.path().to_str().unwrap()).unwrap_err();
        // Same rendering the CLI prints (format_violations + exit message).
        assert!(err.contains("E_ARRIVAL_MISSING"), "{err}");
        assert!(err.contains("ERROR"), "{err}");
        assert!(err.contains("invalid: 1 issue(s)"), "{err}");
    }

    #[test]
    fn validate_missing_file_is_a_named_error() {
        let err = validate_spectrum_native("/nonexistent/spectrum.yaml").unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
    }

    #[test]
    fn validate_unparseable_file_is_a_named_error() {
        let f = write_tmp("arrival: [broken");
        let err = validate_spectrum_native(f.path().to_str().unwrap()).unwrap_err();
        assert!(err.contains("cannot parse"), "{err}");
    }

    // —— run_scenario ——

    #[test]
    fn run_scenario_validates_action_name() {
        let err = run_scenario_native("restart", None, None, None, None).unwrap_err();
        assert!(err.contains("unknown action restart"), "{err}");
        let err = run_scenario_native("start", None, None, None, None).unwrap_err();
        assert!(err.contains("requires path"), "{err}");
        let err = run_scenario_native("stop", None, None, None, None).unwrap_err();
        assert!(err.contains("requires run_id"), "{err}");
    }

    #[test]
    fn run_scenario_refuses_invalid_spectrum_before_contacting_controller() {
        let f = write_tmp("lifetime:\n  distribution: fixed\n  mean_secs: 900\n");
        let err = run_scenario_native(
            "start",
            Some(f.path().to_string_lossy().into_owned()),
            None,
            None,
            Some("http://127.0.0.1:1".into()),
        )
        .unwrap_err();
        // The CLI validates first: the error is the config, not the dead port.
        assert_eq!(err, "spectrum config is invalid", "{err}");
    }

    #[test]
    fn run_scenario_start_stop_roundtrip_on_real_controller() {
        let base = spawn_controller();
        let f = write_tmp(GOOD);

        let v = run_scenario_native(
            "start",
            Some(f.path().to_string_lossy().into_owned()),
            None,
            None,
            Some(base.clone()),
        )
        .unwrap();
        assert_eq!(v["run_id"], "run-1");
        assert_eq!(v["swell_id"], "swell-0001");
        assert_eq!(v["state"], "created");

        // list_runs sees the created run (from the status payload).
        let runs = list_runs_native(None, Some(base.clone())).unwrap();
        assert_eq!(runs.as_array().unwrap().len(), 1);
        assert_eq!(runs[0]["run_id"], "run-1");

        // Single-run forward returns the run object itself.
        let run = list_runs_native(Some("run-1".into()), Some(base.clone())).unwrap();
        assert_eq!(run["run_id"], "run-1");

        let v = run_scenario_native("stop", None, None, Some("run-1".into()), Some(base.clone()))
            .unwrap();
        assert_eq!(v["state"], "stopped");

        // Stop is idempotent, matching the controller contract.
        let v = run_scenario_native("stop", None, None, Some("run-1".into()), Some(base.clone()))
            .unwrap();
        assert_eq!(v["state"], "stopped");
    }

    #[test]
    fn run_scenario_unknown_run_and_unknown_run_forward() {
        let base = spawn_controller();
        let err = run_scenario_native(
            "stop",
            None,
            None,
            Some("run-nope".into()),
            Some(base.clone()),
        )
        .unwrap_err();
        assert!(err.contains("unknown run"), "{err}");

        let err = list_runs_native(Some("run-404".into()), Some(base)).unwrap_err();
        assert!(err.contains("unknown run"), "{err}");
    }

    #[test]
    fn controller_errors_are_named_not_silent() {
        let dead = "http://127.0.0.1:1";
        let err = query_status_native(Some(dead.into())).unwrap_err();
        assert!(err.contains("controller unavailable"), "{err}");
        let err = list_runs_native(None, Some(dead.into())).unwrap_err();
        assert!(err.contains("controller unavailable"), "{err}");
    }

    #[test]
    fn query_status_forwards_the_controller_contract() {
        let base = spawn_controller();
        let v = query_status_native(Some(base)).unwrap();
        assert_eq!(v["status"], "running");
        assert!(v["protocol_version"].is_u64());
        assert_eq!(v["worker_count"], 0);
        assert!(v["workers"].is_array());
        assert!(v["runs"].is_array());
    }

    // —— run_test_plan / get_test_report ——

    /// A stub-runnable plan in a tempdir: the spectrum is referenced by
    /// absolute path so the plan file's location does not depend on CWD.
    fn write_stub_plan() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("spec.yaml");
        std::fs::write(&spec, GOOD).unwrap();
        let plan = dir.path().join("plan.toml");
        // Forward slashes: a Windows path's backslashes are TOML escapes.
        let spectrum = spec.display().to_string().replace('\\', "/");
        std::fs::write(
            &plan,
            format!(
                "[meta]\nname = \"mcp-demo\"\n\n[[steps]]\nid = \"load\"\naction = \"inject_load\"\n[steps.params]\nspectrum = \"{spectrum}\"\nconcurrency = 1\nduration_secs = 30\n\n[[steps]]\nid = \"snap\"\naction = \"capture\"\n\n[[steps]]\nid = \"ok\"\naction = \"assert\"\n[steps.params]\nmetric = \"error_rate\"\nmax = 0.0\nsource = \"snap\"\n"
            ),
        )
        .unwrap();
        (dir, plan, spec)
    }

    #[test]
    fn run_test_plan_stub_runs_green_and_reports_the_path() {
        let (_dir, plan, _spec) = write_stub_plan();
        let out_root = tempfile::tempdir().unwrap();
        let out = out_root.path().join("run");
        let v = run_test_plan_native(
            plan.to_str().unwrap(),
            Some("stub".into()),
            Some(0.0),
            Some(out.to_string_lossy().into_owned()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(v["verdict"], "passed", "{}", v);
        assert_eq!(v["backend"], "stub");
        assert_eq!(v["plan_name"], "mcp-demo");
        assert_eq!(v["asserts"][0]["judgement"]["verdict"], "pass", "{}", v);
        assert_eq!(v["report_path"], format!("{}/report.md", out.display()));
        assert!(out.join("report.md").exists());
        assert!(out.join("run.json").exists());
    }

    #[test]
    fn run_test_plan_failed_verdict_is_an_ok_result_carrying_the_reason() {
        let (_dir, plan, _spec) = write_stub_plan();
        // A p99 bound the stub cannot meet: verdict failed, result still Ok.
        let raw = std::fs::read_to_string(&plan).unwrap();
        let tight = raw
            .replace("metric = \"error_rate\"", "metric = \"latency_p99_ms\"")
            .replace("max = 0.0", "max = 1.0");
        std::fs::write(&plan, tight).unwrap();
        let out = tempfile::tempdir().unwrap();
        let v = run_test_plan_native(
            plan.to_str().unwrap(),
            Some("stub".into()),
            Some(0.0),
            Some(out.path().join("run").to_string_lossy().into_owned()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(v["verdict"], "failed", "{}", v);
        assert_eq!(v["asserts"][0]["judgement"]["verdict"], "fail", "{}", v);
    }

    #[test]
    fn run_test_plan_invalid_plan_is_a_named_error() {
        let dir = tempfile::tempdir().unwrap();
        let plan = dir.path().join("bad.toml");
        std::fs::write(&plan, "steps = []\n\n[meta]\nname = \"x\"\n").unwrap();
        let err =
            run_test_plan_native(plan.to_str().unwrap(), None, None, None, None, None).unwrap_err();
        assert!(err.contains("E_STEPS_EMPTY"), "{err}");
    }

    #[test]
    fn run_test_plan_rejects_unknown_backend_and_bad_scale_pairing() {
        let (_dir, plan, _spec) = write_stub_plan();
        let err = run_test_plan_native(
            plan.to_str().unwrap(),
            Some("chaos".into()),
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(err.contains("unknown backend"), "{err}");
        let err = run_test_plan_native(
            plan.to_str().unwrap(),
            Some("controller".into()),
            Some(0.5),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(err.contains("time_scale"), "{err}");
    }

    #[test]
    fn get_test_report_reads_run_dir_roundtrip() {
        let (_dir, plan, _spec) = write_stub_plan();
        let out_root = tempfile::tempdir().unwrap();
        let out = out_root.path().join("run");
        run_test_plan_native(
            plan.to_str().unwrap(),
            Some("stub".into()),
            Some(0.0),
            Some(out.to_string_lossy().into_owned()),
            None,
            None,
        )
        .unwrap();
        let v = get_test_report_native(Some(out.to_string_lossy().into_owned()), None).unwrap();
        assert_eq!(v["run"]["verdict"], "passed", "{}", v);
        assert_eq!(v["run"]["plan_name"], "mcp-demo");
        let report = v["report_markdown"].as_str().unwrap();
        assert!(report.contains("# Test plan report: mcp-demo"), "{report}");
    }

    #[test]
    fn get_test_report_discovers_the_latest_run_under_root() {
        let (_dir, plan, _spec) = write_stub_plan();
        let root = tempfile::tempdir().unwrap();
        // Two runs; the later one must win the discovery.
        for name in ["run-a", "run-b"] {
            run_test_plan_native(
                plan.to_str().unwrap(),
                Some("stub".into()),
                Some(0.0),
                Some(root.path().join(name).to_string_lossy().into_owned()),
                None,
                None,
            )
            .unwrap();
        }
        let v = get_test_report_native(None, Some(root.path().display().to_string())).unwrap();
        assert_eq!(
            v["run_dir"],
            root.path().join("run-b").display().to_string()
        );
    }

    #[test]
    fn get_test_report_errors_are_named() {
        let err = get_test_report_native(Some("/nonexistent/run".into()), None).unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
        assert!(err.contains("wm test run"), "{err}");
        let empty = tempfile::tempdir().unwrap();
        let err =
            get_test_report_native(None, Some(empty.path().display().to_string())).unwrap_err();
        assert!(err.contains("no test plan run directories"), "{err}");
    }
}
