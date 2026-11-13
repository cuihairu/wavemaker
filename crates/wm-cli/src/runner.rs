//! Test plan runner (WM-TP-2): the step state machine that executes a
//! [`TestPlan`] and writes a run directory (plan + per-step timeline +
//! assertion results + metrics JSON + `report.md`).
//!
//! Semantics:
//!
//! - The machine is IO-free itself: everything environmental goes through
//!   [`TestIo`] (clock, probe, injection, capture). Production backends are
//!   [`StubIo`] (in-process simulator) and [`ControllerIo`] (controller
//!   REST via `commands::run` / `stop`, the same code `wm run` executes).
//! - Operational step errors (unreachable target, injection refused,
//!   capture without a source, spectrum hash mismatch) abort the run
//!   immediately and are recorded; assert FAILs honor the plan's
//!   `continue_on_assert_fail` (default false, fail-fast). Both end in
//!   verdict `failed`; the report carries per-assertion PASS/FAIL/NA.
//! - Single-load invariant: injecting while a load is active stops the
//!   previous one first; every load the runner started is stopped before
//!   the run ends (best effort, recorded).
//! - `time_scale` compresses real sleeps on the stub backend only (the
//!   simulated clock always advances by the full amount); the controller
//!   backend requires 1.0 — waiting for real load with compressed time
//!   would be a lie.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use wm_core::testplan::{
    judge_assert, ActionName, AssertJudgement, AssertVerdict, MetricsSnapshot, TestPlan, TestStep,
};

use crate::commands;
use crate::plan::{prepare_injection, PreparedSpectrum};
use crate::stub::StubIo;

/// Which execution backend a run uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunBackend {
    /// In-process simulator (no real server involved; reports say so).
    Stub,
    /// Drive a real controller REST API.
    Controller,
}

impl RunBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            RunBackend::Stub => "stub",
            RunBackend::Controller => "controller",
        }
    }

    /// Parse the CLI-facing name.
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "stub" => Ok(RunBackend::Stub),
            "controller" => Ok(RunBackend::Controller),
            other => Err(format!("unknown backend '{other}' (stub | controller)")),
        }
    }
}

/// Run configuration (the caller-facing face of the machine).
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub backend: RunBackend,
    pub controller: String,
    /// Metrics snapshot endpoint for the controller backend.
    pub basin_url: Option<String>,
    /// Sleep compression, stub backend only; must be in (0.0, 1.0].
    pub time_scale: f64,
    /// Run directory; `None` = dry run (no artifacts written).
    pub out_dir: Option<PathBuf>,
}

impl RunOptions {
    /// Validate backend/time-scale pairing: only the stub may compress
    /// time (0.0 = no real waiting at all); the controller backend waits
    /// in real time or not at all.
    pub fn validate(&self) -> Result<(), String> {
        match self.backend {
            RunBackend::Stub => {
                if !(0.0..=1.0).contains(&self.time_scale) {
                    return Err(format!(
                        "time_scale must be in [0.0, 1.0], got {}",
                        self.time_scale
                    ));
                }
                Ok(())
            }
            RunBackend::Controller => {
                if self.time_scale != 1.0 {
                    return Err(
                        "time_scale applies to the stub backend only; the controller backend \
                         always waits in real time"
                            .into(),
                    );
                }
                Ok(())
            }
        }
    }
}

/// A run ends in exactly one of these; per-assertion detail lives in
/// `asserts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    Failed,
}

/// One load the runner started (and its lifecycle state).
#[derive(Debug, Clone, Serialize)]
pub struct LoadRecord {
    pub step_id: String,
    pub run_id: String,
    pub spectrum_source: String,
    pub spectrum_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<usize>,
    /// Stopped by the runner (end of duration, plan end, or abort).
    pub stopped: bool,
}

/// One judged assert step.
#[derive(Debug, Clone, Serialize)]
pub struct AssertOutcome {
    pub step_id: String,
    pub judgement: AssertJudgement,
}

/// Per-step timeline entry.
#[derive(Debug, Clone, Serialize)]
pub struct StepTimeline {
    pub step_id: String,
    pub action: String,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

/// One structured run event (rendered as `events.jsonl`).
#[derive(Debug, Clone, Serialize)]
pub struct RunEvent {
    pub ts_unix_ms: u64,
    pub kind: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

/// Full outcome of one plan execution.
#[derive(Debug, Clone, Serialize)]
pub struct RunOutcome {
    pub plan_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    pub continue_on_assert_fail: bool,
    pub backend: String,
    pub time_scale: f64,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    pub steps: Vec<StepTimeline>,
    pub snapshots: Vec<MetricsSnapshot>,
    pub asserts: Vec<AssertOutcome>,
    pub loads: Vec<LoadRecord>,
    pub events: Vec<RunEvent>,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_dir: Option<String>,
}

/// The environment seam of the state machine: clock, reachability probe,
/// load injection, metrics capture. Production impls: [`StubIo`],
/// [`ControllerIo`].
pub trait TestIo {
    fn now_unix_ms(&self) -> u64;
    /// Wait `secs` (the impl owns any time compression).
    fn sleep_secs(&mut self, secs: f64);
    fn probe(&mut self, target: &str, timeout_secs: f64) -> Result<(), String>;
    /// Start a load from a spectrum file; returns the run id and the
    /// prepared spectrum (hash + effective copy when overridden).
    fn inject(
        &mut self,
        spectrum: &Path,
        concurrency: Option<f64>,
        swell_id: Option<&str>,
    ) -> Result<LoadHandle, String>;
    fn stop_load(&mut self, run_id: &str) -> Result<(), String>;
    /// Pull one metrics snapshot. `source_url` overrides the backend's
    /// capture endpoint (the capture step's `source` param).
    fn capture(
        &mut self,
        step_id: &str,
        label: Option<&str>,
        level: Option<usize>,
        source_url: Option<&str>,
    ) -> Result<MetricsSnapshot, String>;
}

/// Result of a successful injection.
#[derive(Debug, Clone)]
pub struct LoadHandle {
    pub run_id: String,
    pub spectrum: PreparedSpectrum,
}

/// Production backend chosen by [`RunOptions`].
pub fn production_io(opts: &RunOptions, run_dir: Option<&Path>) -> Box<dyn TestIo> {
    match opts.backend {
        RunBackend::Stub => Box::new(StubIo::new(opts.time_scale, run_dir.map(Path::to_path_buf))),
        RunBackend::Controller => Box::new(ControllerIo {
            controller: opts.controller.clone(),
            basin_url: opts.basin_url.clone(),
            run_dir: run_dir.map(Path::to_path_buf),
        }),
    }
}

/// Controller REST backend: drives the same `commands::run` / `stop` code
/// `wm run` / `wm stop` execute; captures pull an HTTP snapshot endpoint.
pub struct ControllerIo {
    pub controller: String,
    pub basin_url: Option<String>,
    pub run_dir: Option<PathBuf>,
}

impl TestIo for ControllerIo {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn sleep_secs(&mut self, secs: f64) {
        // Real waiting: the controller backend does not compress time.
        if secs > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(secs));
        }
    }

    fn probe(&mut self, target: &str, timeout_secs: f64) -> Result<(), String> {
        let resp = ureq::get(target)
            .timeout(Duration::from_secs_f64(timeout_secs.max(0.1)))
            .call();
        match resp {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, _)) => Err(format!("target answered HTTP {code}")),
            Err(ureq::Error::Transport(t)) => Err(format!("target unreachable: {t}")),
        }
    }

    fn inject(
        &mut self,
        spectrum: &Path,
        concurrency: Option<f64>,
        swell_id: Option<&str>,
    ) -> Result<LoadHandle, String> {
        let prepared = prepare_injection(spectrum, concurrency, self.run_dir.as_deref())?;
        let info = commands::run(Path::new(&prepared.path), &self.controller, swell_id)
            .map_err(|e| e.to_string())?;
        Ok(LoadHandle {
            run_id: info.run_id,
            spectrum: prepared,
        })
    }

    fn stop_load(&mut self, run_id: &str) -> Result<(), String> {
        commands::stop(run_id, &self.controller)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn capture(
        &mut self,
        step_id: &str,
        label: Option<&str>,
        level: Option<usize>,
        source_url: Option<&str>,
    ) -> Result<MetricsSnapshot, String> {
        let url = source_url.or(self.basin_url.as_deref()).ok_or(
            "no capture source: basin endpoint not configured — controller runs carry \
                    no metrics yet (engine-side aggregation unimplemented, WM-0412/WM-0413); \
                    pass --basin-url or use --backend stub",
        )?;
        let resp = ureq::get(url)
            .timeout(Duration::from_secs_f64(10.0))
            .call()
            .map_err(|e| format!("capture source unreachable: {e}"))?;
        let body = resp
            .into_string()
            .map_err(|e| format!("capture source returned an unreadable body: {e}"))?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| format!("capture source returned bad JSON: {e}"))?;
        let f = |key: &str| v.get(key).and_then(Value::as_f64);
        Ok(MetricsSnapshot {
            step_id: step_id.to_string(),
            label: label.map(str::to_string),
            level,
            captured_at_unix_ms: v
                .get("captured_at_unix_ms")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| self.now_unix_ms()),
            latency_p99_ms: f("latency_p99_ms"),
            error_rate: f("error_rate"),
            throughput_ops: f("throughput_ops"),
            active_concurrency: f("active_concurrency"),
            memory_mb: f("memory_mb"),
            disconnect_recovery_secs: f("disconnect_recovery_secs"),
        })
    }
}

/// What one executed step told the loop.
enum StepResult {
    /// Timeline entry with detail plus the seconds this step actually
    /// waited (feeds the inject_load duration budget).
    Done(Value, f64),
    /// An assert was judged (the outcome records it); the loop decides
    /// fail-fast vs continue from the recorded judgement.
    AssertDone(Value),
    /// Operational error: timeline entry carries the error; the loop
    /// aborts.
    Err(String),
}

/// Execute `plan` step by step. Returns the full outcome; `Err` is
/// reserved for environment failures (cannot create the run directory) —
/// step-level problems live in the outcome.
pub fn run_plan(
    plan: &TestPlan,
    opts: &RunOptions,
    io: &mut dyn TestIo,
    plan_raw: &str,
    plan_sha: &str,
) -> Result<RunOutcome, String> {
    opts.validate()
        .map_err(|e| format!("invalid run options: {e}"))?;
    let out_dir = match &opts.out_dir {
        Some(d) => {
            std::fs::create_dir_all(d)
                .map_err(|e| format!("cannot create run directory {}: {e}", d.display()))?;
            Some(d.clone())
        }
        None => None,
    };
    let mut outcome = RunOutcome {
        plan_name: plan.meta.name.clone(),
        game: plan.meta.game.clone(),
        engine: plan.meta.engine.clone(),
        continue_on_assert_fail: plan.continue_on_assert_fail,
        backend: opts.backend.as_str().to_string(),
        time_scale: opts.time_scale,
        started_unix_ms: io.now_unix_ms(),
        finished_unix_ms: 0,
        steps: Vec::new(),
        snapshots: Vec::new(),
        asserts: Vec::new(),
        loads: Vec::new(),
        events: Vec::new(),
        verdict: Verdict::Passed,
        aborted_step: None,
        abort_reason: None,
        plan_sha256: Some(plan_sha.to_string()),
        run_dir: out_dir.as_ref().map(|d| d.display().to_string()),
    };
    outcome.events.push(RunEvent {
        ts_unix_ms: outcome.started_unix_ms,
        kind: "run_started",
        step: None,
        data: json!({"plan_sha256": plan_sha, "backend": opts.backend.as_str()}),
    });
    // Run id of the currently active injected load, and the remaining
    // seconds of its inject_load duration budget (None = no budget).
    let mut active: Option<String> = None;
    let mut budget: Option<f64> = None;
    let mut has_assert_fail = false;
    let mut aborted = false;
    let mut report_ordinal = 0usize;

    for step in &plan.steps {
        let started = io.now_unix_ms();
        outcome.events.push(RunEvent {
            ts_unix_ms: started,
            kind: "step_started",
            step: Some(step.id.clone()),
            data: json!({"action": step.action.as_str()}),
        });
        let result = exec_step(
            step,
            io,
            &mut active,
            &mut budget,
            &mut outcome,
            out_dir.as_deref(),
            &mut report_ordinal,
        );
        let detail = match &result {
            StepResult::Done(d, _) | StepResult::AssertDone(d) => d.clone(),
            StepResult::Err(e) => json!({"error": e}),
        };
        outcome.steps.push(StepTimeline {
            step_id: step.id.clone(),
            action: step.action.as_str().to_string(),
            started_unix_ms: started,
            finished_unix_ms: io.now_unix_ms(),
            detail,
        });
        match result {
            StepResult::Done(_, slept) => {
                // The inject_load duration is a load-time budget consumed
                // by subsequent steps' waits: once it runs out, the load is
                // released ("inject → hold → capture" keeps the load up for
                // the whole observation window).
                if active.is_some() {
                    if let Some(remaining) = &mut budget {
                        *remaining -= slept;
                        if *remaining <= 0.0 {
                            release_active(
                                io,
                                &mut active,
                                &mut outcome.loads,
                                &mut outcome.events,
                                "duration budget exhausted",
                            );
                            budget = None;
                        }
                    }
                }
                outcome.events.push(RunEvent {
                    ts_unix_ms: io.now_unix_ms(),
                    kind: "step_finished",
                    step: Some(step.id.clone()),
                    data: json!({}),
                });
            }
            StepResult::AssertDone(_) => {
                let judgement = &outcome
                    .asserts
                    .last()
                    .expect("assert step recorded its outcome")
                    .judgement;
                let failed = judgement.verdict == AssertVerdict::Fail;
                outcome.events.push(RunEvent {
                    ts_unix_ms: io.now_unix_ms(),
                    kind: "step_finished",
                    step: Some(step.id.clone()),
                    data: json!({"verdict": judgement.verdict}),
                });
                if failed {
                    has_assert_fail = true;
                    if !plan.continue_on_assert_fail {
                        aborted = true;
                        outcome.aborted_step = Some(step.id.clone());
                        outcome.abort_reason = Some(format!(
                            "assert {} failed (fail-fast): {}",
                            judgement.metric.name(),
                            judgement.reason
                        ));
                        outcome.events.push(RunEvent {
                            ts_unix_ms: io.now_unix_ms(),
                            kind: "run_aborted",
                            step: Some(step.id.clone()),
                            data: json!({"reason": outcome.abort_reason}),
                        });
                        break;
                    }
                }
            }
            StepResult::Err(e) => {
                aborted = true;
                outcome.aborted_step = Some(step.id.clone());
                outcome.abort_reason = Some(e.clone());
                outcome.events.push(RunEvent {
                    ts_unix_ms: io.now_unix_ms(),
                    kind: "run_aborted",
                    step: Some(step.id.clone()),
                    data: json!({"reason": e}),
                });
                break;
            }
        }
    }

    // End of run: release any still-active load (best effort, recorded).
    release_active(
        io,
        &mut active,
        &mut outcome.loads,
        &mut outcome.events,
        "plan end",
    );

    outcome.finished_unix_ms = io.now_unix_ms();
    outcome.verdict = if aborted || has_assert_fail {
        Verdict::Failed
    } else {
        Verdict::Passed
    };
    outcome.events.push(RunEvent {
        ts_unix_ms: outcome.finished_unix_ms,
        kind: "run_finished",
        step: None,
        data: json!({"verdict": outcome.verdict}),
    });
    if let Some(dir) = &out_dir {
        write_artifacts(&outcome, plan_raw, dir)
            .map_err(|e| format!("cannot write run artifacts: {e}"))?;
    }
    Ok(outcome)
}

fn exec_step(
    step: &TestStep,
    io: &mut dyn TestIo,
    active: &mut Option<String>,
    budget: &mut Option<f64>,
    outcome: &mut RunOutcome,
    out_dir: Option<&Path>,
    report_ordinal: &mut usize,
) -> StepResult {
    let p = &step.params;
    match step.action {
        ActionName::Prepare => {
            if let Some(warmup) = p.warmup_secs {
                if warmup > 0.0 {
                    io.sleep_secs(warmup);
                }
            }
            match &p.target {
                Some(target) => match io.probe(target, p.timeout_secs.unwrap_or(5.0)) {
                    Ok(()) => {
                        StepResult::Done(json!({"target": target}), p.warmup_secs.unwrap_or(0.0))
                    }
                    Err(e) => StepResult::Err(e),
                },
                // Warm-up only: nothing to probe.
                None => StepResult::Done(
                    json!({"warmup_secs": p.warmup_secs.unwrap_or(0.0)}),
                    p.warmup_secs.unwrap_or(0.0),
                ),
            }
        }
        ActionName::InjectLoad => {
            let spectrum = PathBuf::from(p.spectrum.clone().unwrap_or_default());
            let handle = match io.inject(&spectrum, p.concurrency, p.swell_id.as_deref()) {
                Ok(h) => h,
                Err(e) => return StepResult::Err(e),
            };
            // Hash pinning: expect_sha256 pins the SOURCE file. On
            // mismatch the just-created load is released immediately —
            // an aborted plan never leaks an injected load.
            if let Some(expect) = &p.expect_sha256 {
                if *expect != handle.spectrum.sha256 {
                    let _ = io.stop_load(&handle.run_id);
                    outcome.events.push(RunEvent {
                        ts_unix_ms: io.now_unix_ms(),
                        kind: "load_stopped",
                        step: Some(step.id.clone()),
                        data: json!({"run_id": handle.run_id, "reason": "spectrum hash mismatch"}),
                    });
                    return StepResult::Err(format!(
                        "spectrum hash mismatch: expect_sha256 {expect} but {} has sha256 {}",
                        handle.spectrum.source_path, handle.spectrum.sha256
                    ));
                }
            }
            release_active(
                io,
                active,
                &mut outcome.loads,
                &mut outcome.events,
                "replaced by next injection",
            );
            let mut record = load_record(step, &handle, None);
            record.expect_sha256 = p.expect_sha256.clone();
            outcome.loads.push(record);
            *active = Some(handle.run_id.clone());
            push_event(
                io,
                outcome,
                step,
                "load_injected",
                json!({
                    "run_id": handle.run_id,
                    "spectrum": handle.spectrum.source_path,
                    "sha256": handle.spectrum.sha256,
                }),
            );
            let detail = json!({
                "run_id": handle.run_id,
                "spectrum": handle.spectrum.source_path,
                "sha256": handle.spectrum.sha256,
                "duration_secs": p.duration_secs,
            });
            // The duration is a budget on time-under-load, consumed by
            // subsequent waits (the loop releases the load when it runs
            // out) — not an in-step sleep that would end the load before
            // the plan observes it.
            *budget = p.duration_secs;
            StepResult::Done(detail, 0.0)
        }
        ActionName::Hold => {
            let secs = p.secs.unwrap_or_default();
            io.sleep_secs(secs);
            StepResult::Done(json!({"secs": secs}), secs)
        }
        ActionName::StepLoad => {
            let spectrum = PathBuf::from(p.spectrum.clone().unwrap_or_default());
            let levels = p.levels.clone().unwrap_or_default();
            let step_secs = p.step_secs.unwrap_or_default();
            let capture_per_level = p.capture_per_level.unwrap_or(true);
            let mut captured = 0usize;
            for (i, level) in levels.iter().enumerate() {
                release_active(
                    io,
                    active,
                    &mut outcome.loads,
                    &mut outcome.events,
                    "next staircase level",
                );
                let handle = match io.inject(&spectrum, Some(*level), None) {
                    Ok(h) => h,
                    Err(e) => return StepResult::Err(e),
                };
                outcome.loads.push(load_record(step, &handle, Some(i)));
                *active = Some(handle.run_id.clone());
                push_event(
                    io,
                    outcome,
                    step,
                    "load_injected",
                    json!({"run_id": handle.run_id, "level": i}),
                );
                io.sleep_secs(step_secs);
                if capture_per_level {
                    let snap = match io.capture(
                        &step.id,
                        p.label.as_deref(),
                        Some(i),
                        p.source.as_deref(),
                    ) {
                        Ok(s) => s,
                        Err(e) => return StepResult::Err(e),
                    };
                    push_event(
                        io,
                        outcome,
                        step,
                        "snapshot_captured",
                        json!({"level": i, "captured_at_unix_ms": snap.captured_at_unix_ms}),
                    );
                    outcome.snapshots.push(snap);
                    captured += 1;
                }
            }
            StepResult::Done(
                json!({
                    "levels": levels,
                    "step_secs": step_secs,
                    "captures": captured,
                }),
                levels.len() as f64 * step_secs,
            )
        }
        ActionName::Capture => {
            let snap = match io.capture(&step.id, p.label.as_deref(), None, p.source.as_deref()) {
                Ok(s) => s,
                Err(e) => return StepResult::Err(e),
            };
            let detail = json!({
                "label": p.label,
                "captured_at_unix_ms": snap.captured_at_unix_ms,
            });
            push_event(
                io,
                outcome,
                step,
                "snapshot_captured",
                json!({"captured_at_unix_ms": snap.captured_at_unix_ms}),
            );
            outcome.snapshots.push(snap);
            StepResult::Done(detail, 0.0)
        }
        ActionName::Assert => {
            // validate() ran before the run; a missing metric here cannot
            // happen post-validation — if it somehow does, fail loudly.
            let metric = match p.metric {
                Some(m) => m,
                None => return StepResult::Err(format!("assert step '{}' has no metric", step.id)),
            };
            let judgement = judge_assert(
                metric,
                p.max,
                p.min,
                p.source.as_deref(),
                &outcome.snapshots,
            );
            let detail = json!({
                "metric": judgement.metric,
                "verdict": judgement.verdict,
                "observed": judgement.observed,
                "reason": judgement.reason,
            });
            push_event(io, outcome, step, "assert_judged", detail.clone());
            outcome.asserts.push(AssertOutcome {
                step_id: step.id.clone(),
                judgement,
            });
            StepResult::AssertDone(detail)
        }
        ActionName::Cooldown => {
            let secs = p.secs.unwrap_or_default();
            io.sleep_secs(secs);
            StepResult::Done(json!({"secs": secs}), secs)
        }
        ActionName::Report => {
            *report_ordinal += 1;
            let stage = p
                .stage
                .clone()
                .unwrap_or_else(|| format!("interim-{report_ordinal}"));
            let mut file = None;
            if let Some(dir) = out_dir {
                // Interim report: the outcome built so far.
                let path = dir.join(format!("report-{stage}.md"));
                let md = render_report_md(outcome);
                if let Err(e) = std::fs::write(&path, md) {
                    return StepResult::Err(format!("cannot write {}: {e}", path.display()));
                }
                file = Some(path.display().to_string());
            }
            push_event(
                io,
                outcome,
                step,
                "report_written",
                json!({"stage": stage, "file": file}),
            );
            StepResult::Done(json!({"stage": stage, "file": file}), 0.0)
        }
    }
}

/// Record one structured event stamped by the backend's clock.
fn push_event(
    io: &mut dyn TestIo,
    outcome: &mut RunOutcome,
    step: &TestStep,
    kind: &'static str,
    data: Value,
) {
    outcome.events.push(RunEvent {
        ts_unix_ms: io.now_unix_ms(),
        kind,
        step: Some(step.id.clone()),
        data,
    });
}

/// Stop the active load (best effort) and record it.
fn release_active(
    io: &mut dyn TestIo,
    active: &mut Option<String>,
    loads: &mut [LoadRecord],
    events: &mut Vec<RunEvent>,
    reason: &str,
) {
    let Some(run_id) = active.take() else {
        return;
    };
    match io.stop_load(&run_id) {
        Ok(()) => {
            mark_stopped(loads, &run_id);
            events.push(RunEvent {
                ts_unix_ms: io.now_unix_ms(),
                kind: "load_stopped",
                step: None,
                data: json!({"run_id": run_id, "reason": reason}),
            });
        }
        Err(e) => events.push(RunEvent {
            ts_unix_ms: io.now_unix_ms(),
            kind: "load_stop_failed",
            step: None,
            data: json!({"run_id": run_id, "error": e}),
        }),
    }
}

fn load_record(step: &TestStep, handle: &LoadHandle, level: Option<usize>) -> LoadRecord {
    LoadRecord {
        step_id: step.id.clone(),
        run_id: handle.run_id.clone(),
        spectrum_source: handle.spectrum.source_path.clone(),
        spectrum_sha256: handle.spectrum.sha256.clone(),
        expect_sha256: None,
        effective_path: handle.spectrum.effective_path.clone(),
        level,
        stopped: false,
    }
}

fn mark_stopped(loads: &mut [LoadRecord], run_id: &str) {
    for l in loads.iter_mut() {
        if l.run_id == run_id {
            l.stopped = true;
        }
    }
}

/// Write the run directory artifacts: the plan snapshot, the event log,
/// metrics, assertion results, the run summary and the human report.
fn write_artifacts(outcome: &RunOutcome, plan_raw: &str, dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join("plan.toml"), plan_raw)?;
    let events = outcome
        .events
        .iter()
        .map(|e| serde_json::to_string(e).expect("event serialization cannot fail"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.join("events.jsonl"), events)?;
    std::fs::write(
        dir.join("metrics.json"),
        serde_json::to_string_pretty(&outcome.snapshots)?,
    )?;
    std::fs::write(
        dir.join("asserts.json"),
        serde_json::to_string_pretty(&outcome.asserts)?,
    )?;
    std::fs::write(dir.join("run.json"), serde_json::to_string_pretty(outcome)?)?;
    std::fs::write(dir.join("report.md"), render_report_md(outcome))?;
    Ok(())
}

/// Render the human-readable markdown report.
pub fn render_report_md(outcome: &RunOutcome) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Test plan report: {}\n\n", outcome.plan_name));
    if let Some(game) = &outcome.game {
        out.push_str(&format!("- game: {game}\n"));
    }
    if let Some(engine) = &outcome.engine {
        out.push_str(&format!("- engine: {engine}\n"));
    }
    out.push_str(&format!("- backend: `{}`\n", outcome.backend));
    if outcome.backend == "stub" {
        out.push_str(
            "- **simulated**: every metric below is synthesized by the in-process stub \
             (deterministic formulas over wm-runtime's arrival curve); nothing observed a \
             real server.\n",
        );
    }
    if outcome.backend == "stub" && outcome.time_scale != 1.0 {
        out.push_str(&format!(
            "- time_scale: {} (real sleeps compressed; simulated durations are full)\n",
            outcome.time_scale
        ));
    }
    out.push_str(&format!(
        "- verdict: **{}**\n",
        match outcome.verdict {
            Verdict::Passed => "passed",
            Verdict::Failed => "failed",
        }
    ));
    if let Some(step) = &outcome.aborted_step {
        out.push_str(&format!(
            "- aborted at step `{}`: {}\n",
            step,
            outcome.abort_reason.as_deref().unwrap_or("unspecified")
        ));
    }
    out.push('\n');

    if !outcome.loads.is_empty() {
        out.push_str("## Loads\n\n| step | run id | spectrum | sha256 | pin | stopped |\n|---|---|---|---|---|---|\n");
        for l in &outcome.loads {
            let pin = match &l.expect_sha256 {
                Some(e) if *e == l.spectrum_sha256 => "match".into(),
                Some(e) => format!("MISMATCH (expected {e})"),
                None => "—".into(),
            };
            out.push_str(&format!(
                "| {} | {} | {} | `{}` | {} | {} |\n",
                l.step_id,
                l.run_id,
                l.spectrum_source,
                &l.spectrum_sha256[..12.min(l.spectrum_sha256.len())],
                pin,
                if l.stopped { "yes" } else { "no" },
            ));
        }
        out.push('\n');
    }

    out.push_str("## Steps\n\n| step | action | duration (ms) | detail |\n|---|---|---|---|\n");
    for s in &outcome.steps {
        let dur = s.finished_unix_ms.saturating_sub(s.started_unix_ms);
        let detail = summary_of(&s.detail);
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            s.step_id, s.action, dur, detail
        ));
    }
    out.push('\n');

    if !outcome.asserts.is_empty() {
        out.push_str("## Assertions\n\n| step | metric | bound | observed | verdict | reason |\n|---|---|---|---|---|---|\n");
        for a in &outcome.asserts {
            let j = &a.judgement;
            let bound = match (j.max, j.min) {
                (Some(m), _) => format!("max {m}"),
                (None, Some(n)) => format!("min {n}"),
                (None, None) => "—".into(),
            };
            let observed = j
                .observed
                .map(|o| format!("{o}"))
                .unwrap_or_else(|| "—".into());
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                a.step_id,
                j.metric.name(),
                bound,
                observed,
                match j.verdict {
                    AssertVerdict::Pass => "PASS",
                    AssertVerdict::Fail => "FAIL",
                    AssertVerdict::NotApplicable => "NA",
                },
                j.reason
            ));
        }
        out.push('\n');
    }

    if !outcome.snapshots.is_empty() {
        out.push_str("## Snapshots\n\n| step | level | p99 (ms) | error rate | throughput | active | memory (MB) |\n|---|---|---|---|---|---|---|\n");
        for s in &outcome.snapshots {
            let cell = |v: Option<f64>| v.map(|x| format!("{x}")).unwrap_or_else(|| "—".into());
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                s.step_id,
                s.level.map(|l| l.to_string()).unwrap_or_else(|| "—".into()),
                cell(s.latency_p99_ms),
                cell(s.error_rate),
                cell(s.throughput_ops),
                cell(s.active_concurrency),
                cell(s.memory_mb),
            ));
        }
        out.push('\n');
    }

    out.push_str("## Honesty notes\n\n");
    match outcome.backend.as_str() {
        "stub" => out.push_str(
            "- Capture source: the in-process stub simulator. Metrics are synthetic by \
             construction (documented formulas); they exercise the plan/assert pipeline, \
             not a server.\n",
        ),
        _ => out.push_str(
            "- Controller runs record orchestration only. Engine-side metrics aggregation \
             is unimplemented (WM-0412 / WM-0413); without a `--basin-url` snapshot source \
             every capture errors and metric asserts judge NotApplicable — never silently \
             pass.\n",
        ),
    }
    out.push_str(
        "- KBEngine live load awaits the Bridge (docs/kbengine.md is at design-doc stage); \
          this run used the native orchestration face only.\n",
    );
    if let Some(dir) = &outcome.run_dir {
        out.push_str(&format!(
            "\nArtifacts: `{}` (plan.toml, events.jsonl, metrics.json, asserts.json, \
             run.json, report.md)\n",
            dir
        ));
    }
    out
}

/// One-line summary of a step detail object for the steps table.
fn summary_of(detail: &Value) -> String {
    match detail {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| match v {
                Value::Null => format!("{k}=—"),
                Value::String(s) => format!("{k}={s}"),
                other => format!("{k}={other}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use wm_core::testplan::parse_plan;

    /// Deterministic snapshot factory for scripted captures.
    fn snap(step: &str, at: u64, p99: f64, err_rate: f64) -> MetricsSnapshot {
        MetricsSnapshot {
            step_id: step.into(),
            label: None,
            level: None,
            captured_at_unix_ms: at,
            latency_p99_ms: Some(p99),
            error_rate: Some(err_rate),
            throughput_ops: Some(42.0),
            active_concurrency: Some(10.0),
            memory_mb: Some(120.0),
            disconnect_recovery_secs: None,
        }
    }

    /// Fully scripted environment: no clocks, no sockets — the state
    /// machine decides everything from these records.
    struct ScriptedIo {
        now: u64,
        sleeps: Vec<f64>,
        probes: Vec<String>,
        injects: Vec<(String, Option<f64>)>,
        stops: Vec<String>,
        next_sha: VecDeque<String>,
        captures: VecDeque<MetricsSnapshot>,
        probe_error: Option<String>,
    }

    impl ScriptedIo {
        fn new() -> Self {
            Self {
                now: 1_000_000,
                sleeps: Vec::new(),
                probes: Vec::new(),
                injects: Vec::new(),
                stops: Vec::new(),
                next_sha: VecDeque::new(),
                captures: VecDeque::new(),
                probe_error: None,
            }
        }

        fn sha(mut self, sha: &str) -> Self {
            self.next_sha.push_back(sha.to_string());
            self
        }

        fn capture(mut self, s: MetricsSnapshot) -> Self {
            self.captures.push_back(s);
            self
        }
    }

    impl TestIo for ScriptedIo {
        fn now_unix_ms(&self) -> u64 {
            self.now
        }

        fn sleep_secs(&mut self, secs: f64) {
            self.sleeps.push(secs);
            self.now += (secs * 1000.0) as u64;
        }

        fn probe(&mut self, target: &str, _timeout_secs: f64) -> Result<(), String> {
            self.probes.push(target.to_string());
            match &self.probe_error {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }

        fn inject(
            &mut self,
            spectrum: &Path,
            concurrency: Option<f64>,
            _swell_id: Option<&str>,
        ) -> Result<LoadHandle, String> {
            self.injects
                .push((spectrum.display().to_string(), concurrency));
            let sha = self.next_sha.pop_front().unwrap_or_else(|| "a".repeat(64));
            Ok(LoadHandle {
                run_id: format!("run-{}", self.injects.len()),
                spectrum: PreparedSpectrum {
                    path: spectrum.display().to_string(),
                    source_path: spectrum.display().to_string(),
                    sha256: sha,
                    effective_path: None,
                },
            })
        }

        fn stop_load(&mut self, run_id: &str) -> Result<(), String> {
            self.stops.push(run_id.to_string());
            Ok(())
        }

        fn capture(
            &mut self,
            _step_id: &str,
            _label: Option<&str>,
            level: Option<usize>,
            _source_url: Option<&str>,
        ) -> Result<MetricsSnapshot, String> {
            // The scripted snapshot keeps its factory-set step id — tests
            // pin which step a snapshot "belongs to" — and takes the level
            // context the machine passes (the backend stamps it).
            let mut s = self
                .captures
                .pop_front()
                .ok_or_else(|| "no scripted snapshot".to_string())?;
            s.level = level;
            Ok(s)
        }
    }

    fn opts(backend: RunBackend, out: Option<PathBuf>) -> RunOptions {
        RunOptions {
            backend,
            controller: commands::DEFAULT_CONTROLLER.into(),
            basin_url: None,
            time_scale: 1.0,
            out_dir: out,
        }
    }

    const HAPPY: &str = r#"
[meta]
name = "happy"

[[steps]]
id = "probe"
action = "prepare"
params = { target = "http://x/healthz", timeout_secs = 2 }

[[steps]]
id = "settle"
action = "hold"
params = { secs = 60 }

[[steps]]
id = "snap"
action = "capture"

[[steps]]
id = "check"
action = "assert"
params = { metric = "latency_p99_ms", max = 250, source = "snap" }

[[steps]]
id = "mid"
action = "report"
params = { stage = "mid" }

[[steps]]
id = "rest"
action = "cooldown"
params = { secs = 30 }
"#;

    #[test]
    fn machine_runs_the_happy_path_in_order() {
        let plan = parse_plan(HAPPY).unwrap();
        let mut io = ScriptedIo::new().capture(snap("snap", 1, 100.0, 0.0));
        let outcome =
            run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "raw", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Passed);
        assert_eq!(io.probes, vec!["http://x/healthz".to_string()]);
        assert_eq!(io.sleeps, vec![60.0, 30.0], "hold then cooldown");
        assert_eq!(outcome.steps.len(), 6);
        assert_eq!(outcome.snapshots.len(), 1);
        assert_eq!(outcome.asserts.len(), 1);
        assert_eq!(outcome.asserts[0].judgement.verdict, AssertVerdict::Pass);
        assert_eq!(outcome.plan_sha256, Some("cafe".into()));
        // The interim report action recorded its stage.
        assert!(outcome
            .events
            .iter()
            .any(|e| e.kind == "report_written" && e.data["stage"] == "mid"));
    }

    const CAPTURE_FAIL: &str = r#"
[meta]
name = "fail-fast"

[[steps]]
id = "snap"
action = "capture"

[[steps]]
id = "check"
action = "assert"
params = { metric = "error_rate", max = 0.0, source = "snap" }

[[steps]]
id = "rest"
action = "cooldown"
params = { secs = 30 }
"#;

    #[test]
    fn fail_fast_aborts_before_remaining_steps() {
        let plan = parse_plan(CAPTURE_FAIL).unwrap();
        let mut io = ScriptedIo::new().capture(snap("snap", 1, 100.0, 0.5));
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Failed);
        assert_eq!(outcome.aborted_step, Some("check".into()));
        assert!(outcome.abort_reason.unwrap().contains("fail-fast"));
        // The cooldown never ran.
        assert_eq!(io.sleeps, Vec::<f64>::new());
        assert_eq!(outcome.steps.len(), 2);
    }

    #[test]
    fn continue_on_assert_fail_finishes_the_plan() {
        let toml_str = CAPTURE_FAIL.replace(
            "[meta]\nname = \"fail-fast\"",
            "continue_on_assert_fail = true\n\n[meta]\nname = \"continue\"",
        );
        let plan = parse_plan(&toml_str).unwrap();
        let mut io = ScriptedIo::new().capture(snap("snap", 1, 100.0, 0.5));
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Failed);
        assert_eq!(outcome.aborted_step, None);
        // The cooldown ran despite the FAIL.
        assert_eq!(io.sleeps, vec![30.0]);
        assert_eq!(outcome.steps.len(), 3);
    }

    #[test]
    fn not_applicable_assert_does_not_fail_the_run() {
        let plan = parse_plan(CAPTURE_FAIL).unwrap();
        // Snapshot from a different step: source "snap" matches nothing.
        let mut io = ScriptedIo::new().capture(snap("other", 1, 100.0, 0.5));
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Passed, "NA is not a FAIL");
        assert_eq!(
            outcome.asserts[0].judgement.verdict,
            AssertVerdict::NotApplicable
        );
    }

    const STAIRCASE: &str = r#"
[meta]
name = "baseline"

[[steps]]
id = "stairs"
action = "step_load"
params = { spectrum = "spec.yaml", levels = [10, 50, 100], step_secs = 30, capture_per_level = true }

[[steps]]
id = "check"
action = "assert"
params = { metric = "latency_p99_ms", max = 250, source = "stairs" }

[[steps]]
id = "rest"
action = "cooldown"
params = { secs = 5 }
"#;

    #[test]
    fn step_load_captures_per_level_and_releases_loads() {
        let plan = parse_plan(STAIRCASE).unwrap();
        let mut io = ScriptedIo::new();
        for (at, p99) in [(1, 46.0), (2, 70.0), (3, 100.0)] {
            io = io.capture(snap("stairs", at, p99, 0.0));
        }
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Passed);
        // One inject per level with the pinned concurrency.
        assert_eq!(
            io.injects.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
            vec![Some(10.0), Some(50.0), Some(100.0)]
        );
        // Stop before level 2 and 3, plus the plan-end release.
        assert_eq!(io.stops.len(), 3);
        assert_eq!(
            io.sleeps,
            vec![30.0, 30.0, 30.0, 5.0],
            "three levels then cooldown"
        );
        // Per-level snapshots, level-indexed, all judged by the assert.
        let levels: Vec<_> = outcome.snapshots.iter().filter_map(|s| s.level).collect();
        assert_eq!(levels, vec![0, 1, 2]);
        assert_eq!(
            outcome.asserts[0].judgement.observed,
            Some(100.0),
            "worst of the three levels"
        );
        assert!(outcome.loads.iter().all(|l| l.stopped));
    }

    const INJECT: &str = r#"
[meta]
name = "inject"

[[steps]]
id = "load"
action = "inject_load"
params = { spectrum = "spec.yaml", duration_secs = 120 }

[[steps]]
id = "settle"
action = "hold"
params = { secs = 1 }
"#;

    #[test]
    fn inject_duration_is_a_budget_the_load_keeps_during_observation() {
        let plan = parse_plan(INJECT).unwrap();
        let mut io = ScriptedIo::new();
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Passed);
        // The hold ran while the load stayed up; the release happened at
        // plan end (budget 120 barely consumed).
        assert_eq!(io.sleeps, vec![1.0]);
        assert_eq!(io.stops, vec!["run-1".to_string()]);
        let load = &outcome.loads[0];
        assert!(load.stopped);
        assert_eq!(load.run_id, "run-1");
        assert_eq!(load.spectrum_sha256, "a".repeat(64));
    }

    #[test]
    fn inject_duration_budget_releases_the_load_mid_plan() {
        // Budget 120: hold 100 keeps the load, hold 100 exhausts it — the
        // release fires before the second hold finishes, cooldown still
        // runs.
        const BUDGET: &str = r#"
[meta]
name = "inject"

[[steps]]
id = "load"
action = "inject_load"
params = { spectrum = "spec.yaml", duration_secs = 120 }

[[steps]]
id = "settle"
action = "hold"
params = { secs = 100 }

[[steps]]
id = "settle2"
action = "hold"
params = { secs = 100 }
"#;
        let plan = parse_plan(BUDGET).unwrap();
        let mut io = ScriptedIo::new();
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Passed);
        assert_eq!(io.sleeps, vec![100.0, 100.0]);
        assert_eq!(
            io.stops,
            vec!["run-1".to_string()],
            "one release, no double stop"
        );
        assert!(outcome.loads[0].stopped);
        assert!(outcome
            .events
            .iter()
            .any(|e| e.kind == "load_stopped" && e.data["reason"] == "duration budget exhausted"));
    }

    #[test]
    fn expect_sha256_mismatch_aborts_with_both_hashes() {
        let toml_str = INJECT.replace(
            "spectrum = \"spec.yaml\", duration_secs = 120",
            "spectrum = \"spec.yaml\", duration_secs = 120, expect_sha256 = \"b\"",
        );
        let plan = parse_plan(&toml_str).unwrap();
        let mut io = ScriptedIo::new().sha(&"a".repeat(64));
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Failed);
        assert_eq!(outcome.aborted_step, Some("load".into()));
        let reason = outcome.abort_reason.unwrap();
        assert!(reason.contains("hash mismatch"), "{reason}");
        assert!(reason.contains("b"), "{reason}");
        // The injected load was released on abort.
        assert_eq!(io.stops, vec!["run-1".to_string()]);
    }

    #[test]
    fn prepare_probe_failure_aborts() {
        let plan = parse_plan(HAPPY).unwrap();
        let mut io = ScriptedIo::new();
        io.probe_error = Some("target unreachable: connection refused".into());
        let outcome = run_plan(&plan, &opts(RunBackend::Stub, None), &mut io, "", "cafe").unwrap();
        assert_eq!(outcome.verdict, Verdict::Failed);
        assert_eq!(outcome.aborted_step, Some("probe".into()));
        assert_eq!(outcome.steps.len(), 1);
        // Nothing after the failed probe ran.
        assert_eq!(io.sleeps, Vec::<f64>::new());
    }

    #[test]
    fn artifacts_and_report_written_for_stub_runs() {
        let plan = parse_plan(HAPPY).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("run");
        let mut io = ScriptedIo::new().capture(snap("snap", 1, 100.0, 0.0));
        let outcome = run_plan(
            &plan,
            &opts(RunBackend::Stub, Some(out.clone())),
            &mut io,
            "[meta] raw plan",
            "cafe",
        )
        .unwrap();
        for file in [
            "plan.toml",
            "events.jsonl",
            "metrics.json",
            "asserts.json",
            "run.json",
            "report.md",
            "report-mid.md",
        ] {
            assert!(out.join(file).exists(), "{file} missing");
        }
        assert_eq!(
            std::fs::read_to_string(out.join("plan.toml")).unwrap(),
            "[meta] raw plan"
        );
        let events = std::fs::read_to_string(out.join("events.jsonl")).unwrap();
        assert!(events.contains("\"run_started\""));
        assert!(events.contains("\"assert_judged\""));
        let report = std::fs::read_to_string(out.join("report.md")).unwrap();
        assert!(report.contains("# Test plan report: happy"), "{report}");
        assert!(report.contains("**passed**"), "{report}");
        assert!(
            report.contains("simulated"),
            "stub runs must say so: {report}"
        );
        assert!(report.contains("KBEngine"), "{report}");
        let run_json = std::fs::read_to_string(out.join("run.json")).unwrap();
        assert!(run_json.contains("\"backend\": \"stub\""), "{run_json}");
        assert_eq!(outcome.run_dir, Some(out.display().to_string()));
    }

    #[test]
    fn time_scale_pairing_is_validated_before_running() {
        let plan = parse_plan(HAPPY).unwrap();
        let mut io = ScriptedIo::new();
        let bad = RunOptions {
            backend: RunBackend::Stub,
            controller: String::new(),
            basin_url: None,
            time_scale: 1.5,
            out_dir: None,
        };
        let err = run_plan(&plan, &bad, &mut io, "", "").unwrap_err();
        assert!(err.contains("invalid run options"), "{err}");
        let bad = RunOptions {
            backend: RunBackend::Controller,
            controller: String::new(),
            basin_url: None,
            time_scale: 0.5,
            out_dir: None,
        };
        let err = run_plan(&plan, &bad, &mut io, "", "").unwrap_err();
        assert!(err.contains("stub backend only"), "{err}");
        assert_eq!(io.sleeps, Vec::<f64>::new(), "nothing ran");
        // Scale 0.0 is legal for the stub: fully compressed, still labeled.
        let zero = RunOptions {
            backend: RunBackend::Stub,
            controller: String::new(),
            basin_url: None,
            time_scale: 0.0,
            out_dir: None,
        };
        let mut io = ScriptedIo::new().capture(snap("snap", 1, 100.0, 0.0));
        let outcome = run_plan(&plan, &zero, &mut io, "", "").unwrap();
        assert_eq!(outcome.verdict, Verdict::Passed);
        assert_eq!(outcome.time_scale, 0.0);
    }

    // --- controller backend against a real controller ---

    /// Bind the controller HTTP API on an ephemeral port (cli_stack.rs
    /// pattern).
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
                let app = wm_controller::http::router(std::sync::Arc::new(
                    wm_controller::state::ControllerState::new(5000),
                ));
                axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                    .await
                    .unwrap();
            });
        });
        format!("http://{addr}")
    }

    const GOOD_SPECTRUM: &str = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nsaturation_policy: queue\n";

    const CONTROLLER_PLAN: &str = r#"
[meta]
name = "controller-loop"

[[steps]]
id = "probe"
action = "prepare"
params = { target = "__HEALTHZ__", timeout_secs = 2 }

[[steps]]
id = "load"
action = "inject_load"
params = { spectrum = "__SPEC__", duration_secs = 1 }

[[steps]]
id = "rest"
action = "cooldown"
params = { secs = 1 }
"#;

    #[test]
    fn controller_backend_runs_green_end_to_end() {
        let base = spawn_controller();
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("spec.yaml");
        std::fs::write(&spec, GOOD_SPECTRUM).unwrap();
        let plan_file = dir.path().join("plan.toml");
        std::fs::write(
            &plan_file,
            CONTROLLER_PLAN
                .replace("__HEALTHZ__", &format!("{base}/healthz"))
                .replace("__SPEC__", &spec.display().to_string()),
        )
        .unwrap();
        let out = dir.path().join("run");
        let opts = RunOptions {
            backend: RunBackend::Controller,
            controller: base,
            basin_url: None,
            time_scale: 1.0,
            out_dir: Some(out.clone()),
        };
        let mut io = production_io(&opts, Some(&out));
        let (plan, raw, sha) = crate::plan::load_plan(&plan_file).unwrap();
        let outcome = run_plan(&plan, &opts, io.as_mut(), &raw, &sha).unwrap();
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "{:?}",
            outcome.abort_reason
        );
        // The controller created and stopped the run.
        assert_eq!(outcome.loads[0].run_id, "run-1");
        assert!(outcome.loads[0].stopped);
        assert!(out.join("report.md").exists());
    }

    #[test]
    fn controller_capture_without_basin_aborts_with_named_reason() {
        let base = spawn_controller();
        let dir = tempfile::tempdir().unwrap();
        let toml_str = r#"
[meta]
name = "no-basin"

[[steps]]
id = "snap"
action = "capture"
"#;
        let plan_file = dir.path().join("plan.toml");
        std::fs::write(&plan_file, toml_str).unwrap();
        let opts = RunOptions {
            backend: RunBackend::Controller,
            controller: base,
            basin_url: None,
            time_scale: 1.0,
            out_dir: None,
        };
        let mut io = production_io(&opts, None);
        let (plan, raw, sha) = crate::plan::load_plan(&plan_file).unwrap();
        let outcome = run_plan(&plan, &opts, io.as_mut(), &raw, &sha).unwrap();
        assert_eq!(outcome.verdict, Verdict::Failed);
        let reason = outcome.abort_reason.unwrap();
        assert!(reason.contains("no capture source"), "{reason}");
        assert!(reason.contains("WM-0412"), "{reason}");
    }
}
