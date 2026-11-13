//! Declarative test plans: one game load-test verification expressed as
//! reusable steps (WM-TP-001).
//!
//! A `TestPlan` is a TOML document in the same style as a Spectrum config:
//! a `[meta]` block plus an ordered `[[steps]]` array. Every step is
//! `{ id, action, params }`; `action` selects from a fixed first-batch
//! enum, `params` is a bag of named fields (`deny_unknown_fields` — a typo
//! is a parse error, not a silent default). Cross-field rules (a `hold`
//! needs `secs`, an `assert` needs `metric` + a bound) are `validate()`
//! violations in the Spectrum style: stable code, offending field, message
//! with the assumption that produced it.
//!
//! Actions (first batch):
//!
//! | action        | params | effect |
//! |---------------|--------|--------|
//! | `prepare`     | `target`, `timeout_secs`, `warmup_secs` | probe target reachability, optional warm-up |
//! | `inject_load` | `spectrum`, `concurrency`, `duration_secs`, `swell_id`, `expect_sha256` | start a load run from a spectrum file |
//! | `hold`        | `secs` | steady-state observation window |
//! | `step_load`   | `spectrum`, `levels`, `step_secs`, `capture_per_level` | staircase; one snapshot per level |
//! | `capture`     | `label`, `source` | pull a metrics snapshot |
//! | `assert`      | `metric`, `max`, `min`, `source` | judge snapshots (see below) |
//! | `cooldown`    | `secs` | cool-down window |
//! | `report`      | `stage` | emit a stage report mid-plan |
//!
//! Assert metrics: `latency_p99_ms` (cap), `error_rate` (cap, fraction
//! 0..1), `throughput` (floor, ops/s), `disconnect_recovery_secs` (cap),
//! `memory_growth_mb_per_hour` (cap; slope over the matched snapshots'
//! memory series, least squares — the leak-trend assertion). Cap metrics
//! take `max` only; `throughput` takes `min` only; direction is validated.
//!
//! Judgement semantics: an assert step matches snapshots by `source`
//! (a producing step id — `capture` or `step_load`); no `source` matches
//! every snapshot so far. A single-valued metric FAILs if any matched
//! snapshot violates the bound (`observed` reports the worst value); with
//! no matched snapshot, or no snapshot carrying the metric, the verdict is
//! `NotApplicable` — a missing data source is never a PASS. A plan-level
//! `continue_on_assert_fail` (default `false`, fail-fast) lets the
//! *executor* keep going after a FAIL; it changes sequencing, never the
//! verdict itself.
//!
//! This module is a pure model: parse, validate, judge — zero IO. File
//! reading, HTTP and timing live in the executor (wm-cli).

use serde::{Deserialize, Serialize};

use crate::spectrum::Severity;

/// One explainable validation output for a plan, in the Spectrum §6 rule 5
/// style: stable code, offending field path, message with the assumption.
/// (Spectrum's own `Violation` carries a `&'static str` field path because
/// its fields are fixed; plan field paths name steps, so they are owned.)
#[derive(Debug, Clone, PartialEq)]
pub struct PlanViolation {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `E_STEPS_EMPTY`.
    pub code: &'static str,
    /// Field path the issue refers to, e.g. `steps[id=hold-1].params.secs`.
    pub field: String,
    pub message: String,
}

/// Plan metadata block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

/// Step action names (first batch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionName {
    Prepare,
    InjectLoad,
    Hold,
    StepLoad,
    Capture,
    Assert,
    Cooldown,
    Report,
}

/// The parameter bag of one step. Keys not valid for the step's action are
/// parse-rejected when unknown outright, and flagged as
/// `E_PARAM_NOT_APPLICABLE` by `validate()` when known but misplaced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StepParams {
    // prepare
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_secs: Option<f64>,
    // inject_load / step_load
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swell_id: Option<String>,
    /// Pin the spectrum content hash (sha256 hex) for deterministic re-runs;
    /// the executor verifies and records the actual hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect_sha256: Option<String>,
    // hold / cooldown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secs: Option<f64>,
    // step_load
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_secs: Option<f64>,
    /// Snapshot at the end of every level (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_per_level: Option<bool>,
    // capture / report
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    // capture / assert: producing step id whose snapshots are judged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    // assert
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<AssertMetric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    // report
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// One plan step: `{ id, action, params }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestStep {
    pub id: String,
    pub action: ActionName,
    #[serde(default)]
    pub params: StepParams,
}

/// A declarative test plan (TOML, Spectrum document style).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestPlan {
    pub meta: PlanMeta,
    /// Keep executing after a FAIL verdict (default false: fail-fast).
    #[serde(default)]
    pub continue_on_assert_fail: bool,
    pub steps: Vec<TestStep>,
}

/// Metrics judged by assert steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertMetric {
    /// p99 latency cap, milliseconds.
    LatencyP99Ms,
    /// Error-rate cap, fraction 0..1 (0.01 = 1%).
    ErrorRate,
    /// Throughput floor, ops/s.
    Throughput,
    /// Time from disconnect to full recovery, cap, seconds.
    DisconnectRecoverySecs,
    /// Memory-leak trend: MB-per-hour slope over the matched snapshots.
    MemoryGrowthMbPerHour,
}

impl ActionName {
    /// The snake_case name as it appears in plan documents.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionName::Prepare => "prepare",
            ActionName::InjectLoad => "inject_load",
            ActionName::Hold => "hold",
            ActionName::StepLoad => "step_load",
            ActionName::Capture => "capture",
            ActionName::Assert => "assert",
            ActionName::Cooldown => "cooldown",
            ActionName::Report => "report",
        }
    }
}

impl AssertMetric {
    /// Cap metrics are upper-bounded (`max`); throughput is floor-bounded
    /// (`min`).
    pub fn takes_max(self) -> bool {
        !matches!(self, AssertMetric::Throughput)
    }

    pub fn takes_min(self) -> bool {
        matches!(self, AssertMetric::Throughput)
    }
}

/// One metrics snapshot (capture output). Every metric is `Option` — a
/// source that cannot provide a metric records it absent, and asserts on
/// it become NotApplicable instead of fabricating a value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsSnapshot {
    /// Id of the step that produced this snapshot (`capture` / `step_load`).
    pub step_id: String,
    /// Optional caller label (capture `label` param).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Level index for step_load snapshots (0-based).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<usize>,
    pub captured_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p99_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput_ops: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_concurrency: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnect_recovery_secs: Option<f64>,
}

impl MetricsSnapshot {
    /// The value an assert metric reads from this snapshot; `None` when the
    /// source did not provide it.
    pub fn metric_value(&self, metric: AssertMetric) -> Option<f64> {
        match metric {
            AssertMetric::LatencyP99Ms => self.latency_p99_ms,
            AssertMetric::ErrorRate => self.error_rate,
            AssertMetric::Throughput => self.throughput_ops,
            AssertMetric::DisconnectRecoverySecs => self.disconnect_recovery_secs,
            // The trend metric is judged across snapshots, not per snapshot.
            AssertMetric::MemoryGrowthMbPerHour => None,
        }
    }
}

/// Verdict of one assert step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertVerdict {
    Pass,
    Fail,
    /// No data to judge (no matched snapshot, or the metric is absent).
    /// A missing data source is never a PASS.
    NotApplicable,
}

/// Full result of judging one assert step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertJudgement {
    pub verdict: AssertVerdict,
    pub metric: AssertMetric,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Worst observed value (the violating extreme on FAIL; the binding
    /// extreme on PASS). `None` on NotApplicable and for the trend metric
    /// when the slope cannot be computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<f64>,
    pub reason: String,
}

/// Parse a plan document (TOML string → model; no file IO).
pub fn parse_plan(toml_str: &str) -> Result<TestPlan, TestPlanError> {
    let plan: TestPlan =
        toml::from_str(toml_str).map_err(|e| TestPlanError::Parse(e.to_string()))?;
    Ok(plan)
}

/// Plan parse/validation errors.
#[derive(Debug, thiserror::Error)]
pub enum TestPlanError {
    #[error("cannot parse plan: {0}")]
    Parse(String),
    #[error("invalid plan: {0} issue(s)")]
    Invalid(usize),
}

impl TestPlan {
    /// Validate cross-field rules. Output is always explainable (Spectrum
    /// §6 rule 5 style): every issue carries a stable code, the offending
    /// step/field and a message.
    pub fn validate(&self) -> Vec<PlanViolation> {
        let mut v = Vec::new();
        if self.meta.name.trim().is_empty() {
            v.push(PlanViolation {
                code: "E_META_NAME_MISSING",
                field: "meta.name".into(),
                message: "plan name is required (assumption: reports are keyed by plan name)"
                    .into(),
                severity: Severity::Error,
            });
        }
        if self.steps.is_empty() {
            v.push(PlanViolation {
                code: "E_STEPS_EMPTY",
                field: "steps".into(),
                message: "a plan needs at least one step".into(),
                severity: Severity::Error,
            });
        }
        let mut seen: Vec<&str> = Vec::new();
        for step in &self.steps {
            if step.id.trim().is_empty() {
                v.push(PlanViolation {
                    code: "E_STEP_ID_EMPTY",
                    field: format!("steps[id={}].id", step.id),
                    message: "step id is required".into(),
                    severity: Severity::Error,
                });
                continue;
            }
            if seen.contains(&step.id.as_str()) {
                v.push(PlanViolation {
                    code: "E_STEP_ID_DUPLICATE",
                    field: format!("steps[id={}].id", step.id),
                    message: "step ids must be unique (assumption: assert source refers to \
                              step ids)"
                        .into(),
                    severity: Severity::Error,
                });
            } else {
                seen.push(&step.id);
            }
            self.validate_step(step, &mut v);
        }
        // Warn when asserts exist but nothing produces snapshots for them.
        let produces = self.steps.iter().any(|s| {
            matches!(s.action, ActionName::Capture)
                || (s.action == ActionName::StepLoad && s.params.capture_per_level.unwrap_or(true))
        });
        let asserts = self.steps.iter().any(|s| s.action == ActionName::Assert);
        if asserts && !produces {
            v.push(PlanViolation {
                code: "W_PLAN_NO_DATA",
                field: "steps".into(),
                message: "plan asserts metrics but no capture/step_load step produces \
                          snapshots; every assert would judge NotApplicable"
                    .into(),
                severity: Severity::Warning,
            });
        }
        v
    }

    fn validate_step(&self, step: &TestStep, v: &mut Vec<PlanViolation>) {
        let p = &step.params;
        let field = |name: &str| format!("steps[id={}].params.{}", step.id, name);
        // Known-but-misplaced params are authoring bugs.
        for (key, present) in p.used_keys() {
            if present && !action_accepts(step.action, key) {
                v.push(PlanViolation {
                    code: "E_PARAM_NOT_APPLICABLE",
                    field: field(key),
                    message: format!(
                        "param `{key}` does not apply to action `{}`",
                        step.action.as_str()
                    ),
                    severity: Severity::Error,
                });
            }
        }
        match step.action {
            ActionName::Prepare => {
                if p.timeout_secs.is_some_and(|t| t <= 0.0) {
                    v.push(PlanViolation {
                        code: "E_TIMEOUT_NON_POSITIVE",
                        field: field("timeout_secs"),
                        message: "timeout_secs must be > 0".into(),
                        severity: Severity::Error,
                    });
                }
                if p.warmup_secs.is_some_and(|t| t < 0.0) {
                    v.push(PlanViolation {
                        code: "E_WARMUP_NEGATIVE",
                        field: field("warmup_secs"),
                        message: "warmup_secs must be >= 0".into(),
                        severity: Severity::Error,
                    });
                }
            }
            ActionName::InjectLoad => {
                if p.spectrum
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    v.push(PlanViolation {
                        code: "E_SPECTRUM_REQUIRED",
                        field: field("spectrum"),
                        message: "inject_load references a spectrum file".into(),
                        severity: Severity::Error,
                    });
                }
                if p.duration_secs.is_some_and(|d| d <= 0.0) {
                    v.push(PlanViolation {
                        code: "E_DURATION_NON_POSITIVE",
                        field: field("duration_secs"),
                        message: "duration_secs must be > 0".into(),
                        severity: Severity::Error,
                    });
                }
            }
            ActionName::Hold | ActionName::Cooldown => {
                if !p.secs.is_some_and(|s| s > 0.0) {
                    v.push(PlanViolation {
                        code: "E_SECS_REQUIRED",
                        field: field("secs"),
                        message: format!("{} needs a positive secs value", step.action.as_str()),
                        severity: Severity::Error,
                    });
                }
            }
            ActionName::StepLoad => {
                if p.spectrum
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    v.push(PlanViolation {
                        code: "E_SPECTRUM_REQUIRED",
                        field: field("spectrum"),
                        message: "step_load references a spectrum file".into(),
                        severity: Severity::Error,
                    });
                }
                match &p.levels {
                    None => v.push(PlanViolation {
                        code: "E_LEVELS_REQUIRED",
                        field: field("levels"),
                        message: "step_load needs at least one concurrency level".into(),
                        severity: Severity::Error,
                    }),
                    Some(l) if l.is_empty() => v.push(PlanViolation {
                        code: "E_LEVELS_REQUIRED",
                        field: field("levels"),
                        message: "step_load needs at least one concurrency level".into(),
                        severity: Severity::Error,
                    }),
                    Some(l) if !l.windows(2).all(|w| w[0] < w[1]) => {
                        v.push(PlanViolation {
                            code: "E_LEVELS_UNORDERED",
                            field: field("levels"),
                            message: "levels must be strictly ascending (staircase semantics)"
                                .into(),
                            severity: Severity::Error,
                        });
                    }
                    Some(_) => {}
                }
                if !p.step_secs.is_some_and(|s| s > 0.0) {
                    v.push(PlanViolation {
                        code: "E_STEP_SECS_REQUIRED",
                        field: field("step_secs"),
                        message: "step_load needs a positive step_secs per level".into(),
                        severity: Severity::Error,
                    });
                }
            }
            ActionName::Capture => {}
            ActionName::Assert => {
                let Some(metric) = p.metric else {
                    v.push(PlanViolation {
                        code: "E_METRIC_REQUIRED",
                        field: field("metric"),
                        message: "assert names the metric to judge".into(),
                        severity: Severity::Error,
                    });
                    return;
                };
                if p.max.is_none() && p.min.is_none() {
                    v.push(PlanViolation {
                        code: "E_ASSERT_NO_BOUND",
                        field: field("max"),
                        message: format!(
                            "assert needs a bound ({})",
                            if metric.takes_max() { "max" } else { "min" }
                        ),
                        severity: Severity::Error,
                    });
                }
                if p.max.is_some() && !metric.takes_max() {
                    v.push(PlanViolation {
                        code: "E_ASSERT_BOUND_DIRECTION",
                        field: field("max"),
                        message: "throughput is a floor metric: bound it with min".into(),
                        severity: Severity::Error,
                    });
                }
                if p.min.is_some() && !metric.takes_min() {
                    v.push(PlanViolation {
                        code: "E_ASSERT_BOUND_DIRECTION",
                        field: field("min"),
                        message: "cap metrics are upper-bounded: use max".into(),
                        severity: Severity::Error,
                    });
                }
                if let (Some(max), Some(min)) = (p.max, p.min) {
                    if max < min {
                        v.push(PlanViolation {
                            code: "E_ASSERT_BOUNDS_CROSSED",
                            field: field("max"),
                            message: "max < min leaves no passing band".into(),
                            severity: Severity::Error,
                        });
                    }
                }
            }
            ActionName::Report => {}
        }
    }
}

impl StepParams {
    /// Which known keys are set (for misplaced-param validation).
    fn used_keys(&self) -> [(&'static str, bool); 18] {
        [
            ("target", self.target.is_some()),
            ("timeout_secs", self.timeout_secs.is_some()),
            ("warmup_secs", self.warmup_secs.is_some()),
            ("spectrum", self.spectrum.is_some()),
            ("concurrency", self.concurrency.is_some()),
            ("duration_secs", self.duration_secs.is_some()),
            ("swell_id", self.swell_id.is_some()),
            ("expect_sha256", self.expect_sha256.is_some()),
            ("secs", self.secs.is_some()),
            ("levels", self.levels.is_some()),
            ("step_secs", self.step_secs.is_some()),
            ("capture_per_level", self.capture_per_level.is_some()),
            ("label", self.label.is_some()),
            ("source", self.source.is_some()),
            ("metric", self.metric.is_some()),
            ("max", self.max.is_some()),
            ("min", self.min.is_some()),
            ("stage", self.stage.is_some()),
        ]
    }
}

/// Whether `key` is meaningful for `action`.
fn action_accepts(action: ActionName, key: &str) -> bool {
    let prepare = ["target", "timeout_secs", "warmup_secs"];
    let inject = [
        "spectrum",
        "concurrency",
        "duration_secs",
        "swell_id",
        "expect_sha256",
    ];
    let window = ["secs"];
    let stepload = ["spectrum", "levels", "step_secs", "capture_per_level"];
    let capture = ["label", "source"];
    let assert = ["metric", "max", "min", "source"];
    let report = ["stage"];
    let accepted: &[&[&str]] = match action {
        ActionName::Prepare => &[&prepare],
        ActionName::InjectLoad => &[&inject],
        ActionName::Hold | ActionName::Cooldown => &[&window],
        ActionName::StepLoad => &[&stepload],
        ActionName::Capture => &[&capture],
        ActionName::Assert => &[&assert],
        ActionName::Report => &[&report],
    };
    accepted.iter().any(|set| set.contains(&key))
}

/// Snapshots an assert step judges: those produced by `source` when given,
/// otherwise every snapshot so far.
pub fn matched_snapshots<'a>(
    snapshots: &'a [MetricsSnapshot],
    source: Option<&str>,
) -> Vec<&'a MetricsSnapshot> {
    match source {
        Some(s) => snapshots
            .iter()
            .filter(|x| x.step_id == s)
            .collect::<Vec<_>>(),
        None => snapshots.iter().collect::<Vec<_>>(),
    }
}

/// Least-squares slope of memory (MB) over time (hours) — the leak-trend
/// metric. Needs at least two points; `None` otherwise.
pub fn memory_growth_mb_per_hour(snapshots: &[&MetricsSnapshot]) -> Option<f64> {
    let mut pts: Vec<(f64, f64)> = snapshots
        .iter()
        .filter_map(|s| s.memory_mb.map(|mb| (s.captured_at_unix_ms as f64, mb)))
        .collect();
    if pts.len() < 2 {
        return None;
    }
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let t0 = pts[0].0;
    let xs: Vec<f64> = pts.iter().map(|(t, _)| (t - t0) / 3_600_000.0).collect();
    let ys: Vec<f64> = pts.iter().map(|(_, mb)| *mb).collect();
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom.abs() < f64::EPSILON {
        return None;
    }
    Some((n * sxy - sx * sy) / denom)
}

/// Judge one assert step against the snapshots it matches. Pure: data in,
/// verdict out. Bound direction is validated at parse time; here a
/// missing bound judges NotApplicable defensively rather than passing.
pub fn judge_assert(
    metric: AssertMetric,
    max: Option<f64>,
    min: Option<f64>,
    source: Option<&str>,
    snapshots: &[MetricsSnapshot],
) -> AssertJudgement {
    let mut judgement = AssertJudgement {
        verdict: AssertVerdict::NotApplicable,
        metric,
        max,
        min,
        observed: None,
        reason: String::new(),
    };
    if max.is_none() && min.is_none() {
        judgement.reason = "no bound configured".into();
        return judgement;
    }
    let matched = matched_snapshots(snapshots, source);
    if matched.is_empty() {
        judgement.reason = match source {
            Some(s) => format!("no snapshot from source '{s}'"),
            None => "no snapshot captured yet".into(),
        };
        return judgement;
    }
    if metric == AssertMetric::MemoryGrowthMbPerHour {
        let slope = memory_growth_mb_per_hour(&matched);
        let Some(slope) = slope else {
            judgement.reason = format!(
                "memory trend needs >= 2 snapshots carrying memory_mb, got {}",
                matched.iter().filter(|s| s.memory_mb.is_some()).count()
            );
            return judgement;
        };
        judgement.observed = Some(slope);
        match max {
            Some(cap) if slope > cap => {
                judgement.verdict = AssertVerdict::Fail;
                judgement.reason = format!("memory growth {slope:.2} MB/h exceeds max {cap} MB/h");
            }
            _ => {
                judgement.verdict = AssertVerdict::Pass;
                judgement.reason = format!("memory growth {slope:.2} MB/h within bounds");
            }
        }
        return judgement;
    }
    let values: Vec<f64> = matched
        .iter()
        .filter_map(|s| s.metric_value(metric))
        .collect();
    if values.is_empty() {
        judgement.reason = format!("metric absent in {} matched snapshot(s)", matched.len());
        return judgement;
    }
    // Worst case decides: any violating snapshot is a FAIL (a cap holds for
    // every sample, a floor holds for every sample).
    let worst_max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let worst_min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let mut reasons: Vec<String> = Vec::new();
    let mut failed = false;
    let mut binding: Option<f64> = None;
    if let Some(cap) = max {
        if worst_max > cap {
            failed = true;
            binding = Some(worst_max);
            reasons.push(format!("observed {worst_max} exceeds max {cap}"));
        } else {
            binding = Some(binding.map_or(worst_max, |b: f64| b.max(worst_max)));
        }
    }
    if let Some(floor) = min {
        if worst_min < floor {
            failed = true;
            binding = Some(worst_min);
            reasons.push(format!("observed {worst_min} below min {floor}"));
        } else if !failed {
            binding = Some(binding.map_or(worst_min, |b: f64| b.min(worst_min)));
        }
    }
    judgement.observed = binding;
    if failed {
        judgement.verdict = AssertVerdict::Fail;
        judgement.reason = reasons.join("; ");
    } else {
        judgement.verdict = AssertVerdict::Pass;
        judgement.reason = format!("all {} sample(s) within bounds", values.len());
    }
    judgement
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMOKE: &str = r#"
[meta]
name = "smoke"
game = "demo"
engine = "stub"

[[steps]]
id = "probe"
action = "prepare"
[steps.params]
target = "http://127.0.0.1:7600/healthz"
timeout_secs = 5

[[steps]]
id = "load"
action = "inject_load"
[steps.params]
spectrum = "examples/spectra/smoke.yaml"
concurrency = 1
duration_secs = 300

[[steps]]
id = "settle"
action = "hold"
[steps.params]
secs = 60

[[steps]]
id = "snap"
action = "capture"
[steps.params]
label = "steady"

[[steps]]
id = "no-errors"
action = "assert"
[steps.params]
metric = "error_rate"
max = 0.0
source = "snap"

[[steps]]
id = "rest"
action = "cooldown"
[steps.params]
secs = 30

[[steps]]
id = "stage-report"
action = "report"
[steps.params]
stage = "smoke"
"#;

    #[test]
    fn parse_smoke_shape_and_defaults() {
        let plan = parse_plan(SMOKE).unwrap();
        assert_eq!(plan.meta.name, "smoke");
        assert!(!plan.continue_on_assert_fail, "fail-fast is the default");
        assert_eq!(plan.steps.len(), 7);
        assert_eq!(plan.steps[0].action, ActionName::Prepare);
        assert_eq!(
            plan.steps[0].params.target.as_deref(),
            Some("http://127.0.0.1:7600/healthz")
        );
        assert_eq!(plan.steps[2].params.secs, Some(60.0));
        assert_eq!(plan.steps[6].action, ActionName::Report);
        assert_eq!(plan.validate(), vec![]);
    }

    #[test]
    fn parse_every_action_of_the_first_batch() {
        let toml_str = r#"
[meta]
name = "all-actions"

[[steps]]
id = "a"
action = "prepare"
params = { warmup_secs = 10 }

[[steps]]
id = "b"
action = "inject_load"
params = { spectrum = "s.yaml", concurrency = 10, duration_secs = 60, swell_id = "swell-x", expect_sha256 = "deadbeef" }

[[steps]]
id = "c"
action = "hold"
params = { secs = 5 }

[[steps]]
id = "d"
action = "step_load"
params = { spectrum = "s.yaml", levels = [10, 50, 100], step_secs = 120, capture_per_level = true }

[[steps]]
id = "e"
action = "capture"

[[steps]]
id = "f"
action = "assert"
params = { metric = "latency_p99_ms", max = 250, source = "d" }

[[steps]]
id = "g"
action = "cooldown"
params = { secs = 1 }

[[steps]]
id = "h"
action = "report"
params = { stage = "levels" }
"#;
        let plan = parse_plan(toml_str).unwrap();
        assert_eq!(plan.steps.len(), 8);
        assert_eq!(plan.steps[3].params.levels, Some(vec![10.0, 50.0, 100.0]));
        assert_eq!(
            plan.steps[5].params.metric,
            Some(AssertMetric::LatencyP99Ms)
        );
        assert_eq!(plan.validate(), vec![]);
    }

    #[test]
    fn unknown_action_is_a_named_parse_error() {
        let err = parse_plan("[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"explode\"\n")
            .unwrap_err();
        assert!(err.to_string().contains("unknown variant"), "{err}");
    }

    #[test]
    fn unknown_param_key_is_a_named_parse_error() {
        let err = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"hold\"\n[steps.params]\nsecc = 5\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn misplaced_known_param_is_a_violation() {
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"hold\"\n[steps.params]\nsecs = 5\nspectrum = \"s.yaml\"\n",
        )
        .unwrap();
        let v = plan.validate();
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "E_PARAM_NOT_APPLICABLE");
        assert!(v[0].field.contains("spectrum"), "{}", v[0].field);
    }

    #[test]
    fn duplicate_and_empty_step_ids_are_violations() {
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"report\"\n\n[[steps]]\nid=\"a\"\naction=\"report\"\n\n[[steps]]\nid=\" \"\naction=\"report\"\n",
        )
        .unwrap();
        let codes: Vec<&str> = plan.validate().iter().map(|x| x.code).collect();
        assert!(codes.contains(&"E_STEP_ID_DUPLICATE"), "{codes:?}");
        assert!(codes.contains(&"E_STEP_ID_EMPTY"), "{codes:?}");
    }

    #[test]
    fn empty_steps_and_missing_name_are_violations() {
        let plan = parse_plan("steps = []\n\n[meta]\nname = \"\"\n").unwrap();
        let codes: Vec<&str> = plan.validate().iter().map(|x| x.code).collect();
        assert!(codes.contains(&"E_META_NAME_MISSING"), "{codes:?}");
        assert!(codes.contains(&"E_STEPS_EMPTY"), "{codes:?}");
    }

    #[test]
    fn hold_without_secs_is_a_violation() {
        let plan =
            parse_plan("[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"hold\"\n").unwrap();
        assert_eq!(plan.validate()[0].code, "E_SECS_REQUIRED");
    }

    #[test]
    fn step_load_requires_ascending_levels_and_step_secs() {
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"step_load\"\nparams = { spectrum = \"s.yaml\", levels = [50, 10], step_secs = 60 }\n",
        )
        .unwrap();
        assert_eq!(plan.validate()[0].code, "E_LEVELS_UNORDERED");

        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"step_load\"\nparams = { spectrum = \"s.yaml\", levels = [10], step_secs = 0 }\n",
        )
        .unwrap();
        assert_eq!(plan.validate()[0].code, "E_STEP_SECS_REQUIRED");
    }

    #[test]
    fn assert_direction_and_bound_rules() {
        // throughput with max: wrong direction.
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"assert\"\nparams = { metric = \"throughput\", max = 100 }\n",
        )
        .unwrap();
        assert_eq!(plan.validate()[0].code, "E_ASSERT_BOUND_DIRECTION");

        // p99 with min: wrong direction.
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"assert\"\nparams = { metric = \"latency_p99_ms\", min = 10 }\n",
        )
        .unwrap();
        assert_eq!(plan.validate()[0].code, "E_ASSERT_BOUND_DIRECTION");

        // no bound at all.
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"assert\"\nparams = { metric = \"error_rate\" }\n",
        )
        .unwrap();
        assert_eq!(plan.validate()[0].code, "E_ASSERT_NO_BOUND");

        // crossed bounds (reported alongside the direction issue).
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"assert\"\nparams = { metric = \"throughput\", min = 200, max = 100 }\n",
        )
        .unwrap();
        let codes: Vec<&str> = plan.validate().iter().map(|x| x.code).collect();
        assert!(codes.contains(&"E_ASSERT_BOUNDS_CROSSED"), "{codes:?}");
    }

    #[test]
    fn asserts_without_data_producer_warn() {
        let plan = parse_plan(
            "[meta]\nname=\"x\"\n\n[[steps]]\nid=\"a\"\naction=\"assert\"\nparams = { metric = \"error_rate\", max = 0.0 }\n",
        )
        .unwrap();
        let v = plan.validate();
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].code, "W_PLAN_NO_DATA");
        assert_eq!(v[0].severity, Severity::Warning);
    }

    #[test]
    fn fail_fast_flag_roundtrips() {
        let plan = parse_plan(
            "continue_on_assert_fail = true\n\n[meta]\nname = \"x\"\n\n[[steps]]\nid = \"a\"\naction = \"report\"\n",
        )
        .unwrap();
        assert!(plan.continue_on_assert_fail);
        assert_eq!(plan.validate(), vec![]);

        // A plan-level key misplaced under [meta] is a named parse error,
        // not a silently-ignored default.
        let err = parse_plan(
            "[meta]\nname = \"x\"\ncontinue_on_assert_fail = true\n\n[[steps]]\nid = \"a\"\naction = \"report\"\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    fn snap(
        step: &str,
        at: u64,
        p99: Option<f64>,
        errs: Option<f64>,
        tput: Option<f64>,
        mem: Option<f64>,
    ) -> MetricsSnapshot {
        MetricsSnapshot {
            step_id: step.into(),
            label: None,
            level: None,
            captured_at_unix_ms: at,
            latency_p99_ms: p99,
            error_rate: errs,
            throughput_ops: tput,
            active_concurrency: None,
            memory_mb: mem,
            disconnect_recovery_secs: None,
        }
    }

    #[test]
    fn judge_p99_cap_pass_fail_na() {
        let snaps = vec![
            snap("c", 1, Some(120.0), None, None, None),
            snap("c", 2, Some(240.0), None, None, None),
        ];
        let j = judge_assert(
            AssertMetric::LatencyP99Ms,
            Some(250.0),
            None,
            Some("c"),
            &snaps,
        );
        assert_eq!(j.verdict, AssertVerdict::Pass);
        assert_eq!(j.observed, Some(240.0), "binding extreme is the worst");

        let j = judge_assert(
            AssertMetric::LatencyP99Ms,
            Some(200.0),
            None,
            Some("c"),
            &snaps,
        );
        assert_eq!(j.verdict, AssertVerdict::Fail);
        assert_eq!(j.observed, Some(240.0));
        assert!(j.reason.contains("exceeds max 200"), "{}", j.reason);

        // Metric absent → NA, never PASS.
        let j = judge_assert(
            AssertMetric::DisconnectRecoverySecs,
            Some(10.0),
            None,
            Some("c"),
            &snaps,
        );
        assert_eq!(j.verdict, AssertVerdict::NotApplicable);
        assert!(j.reason.contains("absent"), "{}", j.reason);

        // Unknown source → NA naming the source.
        let j = judge_assert(
            AssertMetric::LatencyP99Ms,
            Some(250.0),
            None,
            Some("nope"),
            &snaps,
        );
        assert_eq!(j.verdict, AssertVerdict::NotApplicable);
        assert!(
            j.reason.contains("no snapshot from source 'nope'"),
            "{}",
            j.reason
        );
    }

    #[test]
    fn judge_error_rate_fraction_and_throughput_floor() {
        let snaps = vec![snap("c", 1, None, Some(0.004), Some(42.0), None)];
        let j = judge_assert(AssertMetric::ErrorRate, Some(0.01), None, None, &snaps);
        assert_eq!(j.verdict, AssertVerdict::Pass);
        let j = judge_assert(AssertMetric::ErrorRate, Some(0.001), None, None, &snaps);
        assert_eq!(j.verdict, AssertVerdict::Fail);
        let j = judge_assert(AssertMetric::Throughput, None, Some(10.0), None, &snaps);
        assert_eq!(j.verdict, AssertVerdict::Pass);
        let j = judge_assert(AssertMetric::Throughput, None, Some(50.0), None, &snaps);
        assert_eq!(j.verdict, AssertVerdict::Fail);
        assert!(j.reason.contains("below min 50"), "{}", j.reason);
    }

    #[test]
    fn judge_memory_growth_slope() {
        // +100 MB every 10 min => ~600 MB/h.
        let snaps = vec![
            snap("s", 0, None, None, None, Some(100.0)),
            snap("s", 600_000, None, None, None, Some(200.0)),
            snap("s", 1_200_000, None, None, None, Some(300.0)),
        ];
        let j = judge_assert(
            AssertMetric::MemoryGrowthMbPerHour,
            Some(700.0),
            None,
            Some("s"),
            &snaps,
        );
        assert_eq!(j.verdict, AssertVerdict::Pass, "{}", j.reason);
        let observed = j.observed.unwrap();
        assert!((observed - 600.0).abs() < 1.0, "{observed}");

        let j = judge_assert(
            AssertMetric::MemoryGrowthMbPerHour,
            Some(500.0),
            None,
            Some("s"),
            &snaps,
        );
        assert_eq!(j.verdict, AssertVerdict::Fail);

        // One point is not a trend.
        let j = judge_assert(
            AssertMetric::MemoryGrowthMbPerHour,
            Some(500.0),
            None,
            Some("s"),
            &snaps[..1],
        );
        assert_eq!(j.verdict, AssertVerdict::NotApplicable);
    }

    #[test]
    fn judge_without_bound_is_na_defensively() {
        let snaps = vec![snap("c", 1, Some(1.0), None, None, None)];
        let j = judge_assert(AssertMetric::LatencyP99Ms, None, None, None, &snaps);
        assert_eq!(j.verdict, AssertVerdict::NotApplicable);
        assert_eq!(j.reason, "no bound configured");
    }

    #[test]
    fn matched_snapshots_filter_by_source() {
        let snaps = vec![
            snap("a", 1, Some(1.0), None, None, None),
            snap("b", 2, Some(2.0), None, None, None),
        ];
        assert_eq!(matched_snapshots(&snaps, Some("b")).len(), 1);
        assert_eq!(matched_snapshots(&snaps, None).len(), 2);
    }

    #[test]
    fn plan_serializes_back_to_toml_without_nulls() {
        let plan = parse_plan(SMOKE).unwrap();
        let text = toml::to_string(&plan).unwrap();
        assert!(!text.contains("None"), "toml cannot carry nulls: {text}");
        let reparsed = parse_plan(&text).unwrap();
        assert_eq!(plan, reparsed);
    }
}
