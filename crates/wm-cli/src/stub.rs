//! Stub backend: an in-process load/metrics simulator so a TestPlan runs
//! end to end without a real game server (WM-TP-2 demo face).
//!
//! Honesty rules:
//!
//! - The load curve is NOT invented here: `StubSim` reuses wm-runtime's
//!   `ArrivalPlan` — the same arrival math the native runtime executes —
//!   fed by the real spectrum file the plan references.
//! - Metrics are synthetic by construction (fixed deterministic formulas of
//!   the simulated load state, no RNG). Reports label the backend `stub`
//!   and carry `time_scale`; every number in a stub report is a simulation
//!   artifact, not an observation of a real server.
//! - `disconnect_recovery_secs` is always absent: the stub has no
//!   disconnects, so the recovery assertion judges NotApplicable — a
//!   missing data source is never a PASS.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wm_core::spectrum::SpectrumConfig;
use wm_core::testplan::MetricsSnapshot;
use wm_runtime::arrival::ArrivalPlan;

use crate::plan::prepare_injection;
use crate::runner::{LoadHandle, TestIo};

const NANOS_PER_SEC: f64 = 1_000_000_000.0;

/// Deterministic load/metrics simulator over one injected spectrum.
#[derive(Debug, Clone)]
pub struct StubSim {
    plan: ArrivalPlan,
    lifetime_mean_secs: f64,
    /// Pinned steady-state concurrency (plan `concurrency` override).
    concurrency_target: Option<f64>,
    elapsed_ns: u64,
}

impl StubSim {
    /// Build from a parsed spectrum. Requires an `arrival.ramp` section
    /// (that is the curve the stub simulates).
    pub fn from_spectrum(
        cfg: &SpectrumConfig,
        concurrency_target: Option<f64>,
    ) -> Result<Self, String> {
        let ramp = cfg
            .arrival
            .as_ref()
            .map(|a| &a.ramp)
            .ok_or("spectrum has no arrival.ramp; the stub simulator needs a load curve")?;
        let plan = ArrivalPlan::new(
            ramp.from_per_second,
            ramp.to_per_second,
            (ramp.duration_secs * NANOS_PER_SEC) as u64,
        )
        .map_err(|e| format!("arrival plan rejected: {e}"))?;
        Ok(Self {
            plan,
            lifetime_mean_secs: cfg.lifetime.as_ref().map(|l| l.mean_secs).unwrap_or(300.0),
            concurrency_target,
            elapsed_ns: 0,
        })
    }

    /// Parse a spectrum file and build the simulator from it.
    pub fn from_file(path: &Path, concurrency_target: Option<f64>) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let cfg: SpectrumConfig = serde_yaml::from_str(&raw)
            .map_err(|e| format!("cannot parse spectrum {}: {e}", path.display()))?;
        Self::from_spectrum(&cfg, concurrency_target)
    }

    pub fn advance_secs(&mut self, secs: f64) {
        self.elapsed_ns += (secs.max(0.0) * NANOS_PER_SEC) as u64;
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.elapsed_ns as f64 / NANOS_PER_SEC
    }

    /// Cumulative admitted bots at the simulated time.
    fn admitted(&self) -> f64 {
        self.plan.cumulative(self.elapsed_ns)
    }

    /// Cumulative completed bots: arrivals shifted by the lifetime mean
    /// (deterministic expected-value approximation, not a sampled lifetime).
    fn completed(&self) -> f64 {
        let shifted = self
            .elapsed_ns
            .saturating_sub((self.lifetime_mean_secs * NANOS_PER_SEC) as u64);
        self.plan.cumulative(shifted)
    }

    fn active(&self) -> f64 {
        let mut active = (self.admitted() - self.completed()).max(0.0);
        if let Some(target) = self.concurrency_target {
            active = active.min(target);
        }
        active
    }

    /// One metrics snapshot at the current simulated time. The formulas are
    /// the stub's fixed synthetic model (documented in the module header):
    /// p99 = 40 + 0.6·active ms; errors zero; throughput = current arrival
    /// rate; memory = 120 + 0.35·active + slow 1.2 MB/h drift; no
    /// disconnects.
    pub fn snapshot(
        &self,
        step_id: &str,
        label: Option<&str>,
        level: Option<usize>,
        captured_at_unix_ms: u64,
    ) -> MetricsSnapshot {
        let active = self.active();
        MetricsSnapshot {
            step_id: step_id.to_string(),
            label: label.map(str::to_string),
            level,
            captured_at_unix_ms,
            latency_p99_ms: Some(40.0 + 0.6 * active),
            error_rate: Some(0.0),
            throughput_ops: Some(self.plan.rate_at(self.elapsed_ns)),
            active_concurrency: Some(active),
            memory_mb: Some(120.0 + 0.35 * active + 0.02 * (self.elapsed_secs() / 60.0)),
            disconnect_recovery_secs: None,
        }
    }
}

/// In-process stub backend. Real sleeps are compressed by `time_scale`
/// (simulated time always advances by the full amount); metrics come from
/// the active [`StubSim`].
pub struct StubIo {
    pub time_scale: f64,
    sim: Option<StubSim>,
    run_counter: usize,
    base_unix_ms: u64,
    sim_offset_ms: u64,
    /// Accumulated real sleep (milliseconds) — observable in tests.
    pub real_sleep_ms: u64,
    /// Accumulated simulated sleep (seconds) — observable in tests.
    pub simulated_sleep_secs: f64,
    run_dir: Option<std::path::PathBuf>,
}

impl StubIo {
    pub fn new(time_scale: f64, run_dir: Option<std::path::PathBuf>) -> Self {
        Self {
            time_scale,
            sim: None,
            run_counter: 0,
            base_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            sim_offset_ms: 0,
            real_sleep_ms: 0,
            simulated_sleep_secs: 0.0,
            run_dir,
        }
    }

    pub fn active_run(&self) -> Option<&StubSim> {
        self.sim.as_ref()
    }
}

impl TestIo for StubIo {
    fn now_unix_ms(&self) -> u64 {
        self.base_unix_ms + self.sim_offset_ms
    }

    fn sleep_secs(&mut self, secs: f64) {
        // Simulated time advances fully; real wall time is compressed.
        self.sim_offset_ms += (secs.max(0.0) * 1000.0) as u64;
        self.simulated_sleep_secs += secs.max(0.0);
        let real = secs.max(0.0) * self.time_scale;
        self.real_sleep_ms += (real * 1000.0) as u64;
        if real > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(real));
        }
        if let Some(sim) = &mut self.sim {
            sim.advance_secs(secs);
        }
    }

    fn probe(&mut self, _target: &str, _timeout_secs: f64) -> Result<(), String> {
        // The stub target is up by construction.
        Ok(())
    }

    fn inject(
        &mut self,
        spectrum: &Path,
        concurrency: Option<f64>,
        _swell_id: Option<&str>,
    ) -> Result<LoadHandle, String> {
        // The stub reads the REAL spectrum file (hash included) — the same
        // injection preparation the controller backend performs.
        let prepared = prepare_injection(spectrum, concurrency, self.run_dir.as_deref())?;
        let sim = StubSim::from_file(Path::new(&prepared.path), concurrency)?;
        self.run_counter += 1;
        self.sim = Some(sim);
        Ok(LoadHandle {
            run_id: format!("stub-run-{}", self.run_counter),
            spectrum: prepared,
        })
    }

    fn stop_load(&mut self, _run_id: &str) -> Result<(), String> {
        self.sim = None;
        Ok(())
    }

    fn capture(
        &mut self,
        step_id: &str,
        label: Option<&str>,
        level: Option<usize>,
        _source_url: Option<&str>,
    ) -> Result<MetricsSnapshot, String> {
        let sim = self
            .sim
            .as_ref()
            .ok_or("no active stub load to capture (inject_load/step_load first)")?;
        Ok(sim.snapshot(
            step_id,
            label,
            level,
            self.base_unix_ms + self.sim_offset_ms,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAMP: &str = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nsaturation_policy: queue\n";

    fn sim() -> StubSim {
        let cfg: SpectrumConfig = serde_yaml::from_str(RAMP).unwrap();
        StubSim::from_spectrum(&cfg, None).unwrap()
    }

    #[test]
    fn stub_requires_an_arrival_curve() {
        let cfg: SpectrumConfig =
            serde_yaml::from_str("lifetime:\n  distribution: fixed\n  mean_secs: 900\n").unwrap();
        let err = StubSim::from_spectrum(&cfg, None).unwrap_err();
        assert!(err.contains("no arrival.ramp"), "{err}");
    }

    #[test]
    fn snapshots_are_deterministic() {
        let mut a = sim();
        let mut b = sim();
        a.advance_secs(600.0);
        b.advance_secs(600.0);
        assert_eq!(
            a.snapshot("s", None, None, 1),
            b.snapshot("s", None, None, 1)
        );
    }

    #[test]
    fn active_load_grows_over_the_ramp() {
        let mut s = sim();
        s.advance_secs(0.0);
        let early = s.snapshot("s", None, None, 0).active_concurrency.unwrap();
        s.advance_secs(600.0);
        let later = s.snapshot("s", None, None, 0).active_concurrency.unwrap();
        assert!(later > early, "{early} -> {later}");
        // λ(t) plateaus at 20/s with mean lifetime 900s: steady-state active
        // ≈ λ·W = 20·900 = 18000 by cumulative math.
        let mut steady = sim();
        steady.advance_secs(60_000.0);
        let active = steady.active();
        assert!(active > 10_000.0, "{active}");
    }

    #[test]
    fn concurrency_override_pins_active() {
        let cfg: SpectrumConfig = serde_yaml::from_str(RAMP).unwrap();
        let mut s = StubSim::from_spectrum(&cfg, Some(50.0)).unwrap();
        s.advance_secs(60_000.0);
        let active = s.active();
        assert!((active - 50.0).abs() < f64::EPSILON, "{active}");
    }

    #[test]
    fn baseline_assert_bounds_hold_at_100_active() {
        // Demo ② pins p99 < 250 ms at concurrency 100: the stub formula
        // gives 40 + 0.6·100 = 100 ms.
        let cfg: SpectrumConfig = serde_yaml::from_str(RAMP).unwrap();
        let mut s = StubSim::from_spectrum(&cfg, Some(100.0)).unwrap();
        s.advance_secs(3600.0);
        let snap = s.snapshot("s", None, None, 0);
        assert_eq!(snap.latency_p99_ms, Some(100.0));
        assert_eq!(snap.error_rate, Some(0.0));
        assert!(snap.disconnect_recovery_secs.is_none());
    }

    #[test]
    fn memory_drift_stays_under_a_leak_threshold() {
        // Soak demo asserts < 15 MB/h growth; the stub drifts ~1.2 MB/h.
        let cfg: SpectrumConfig = serde_yaml::from_str(RAMP).unwrap();
        let mut s = StubSim::from_spectrum(&cfg, Some(5.0)).unwrap();
        let t0 = s.snapshot("s", None, None, 0).memory_mb.unwrap();
        s.advance_secs(1800.0);
        let t1 = s.snapshot("s", None, None, 1_800_000).memory_mb.unwrap();
        let per_hour = (t1 - t0) * 2.0;
        assert!(per_hour < 15.0, "{per_hour}");
    }

    #[test]
    fn stub_from_file_reads_the_real_spectrum() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("spec.yaml");
        std::fs::write(&p, RAMP).unwrap();
        let s = StubSim::from_file(&p, None).unwrap();
        assert_eq!(s.elapsed_secs(), 0.0);
    }

    #[test]
    fn stub_io_sleep_scales_real_time_but_not_simulated_time() {
        let mut io = StubIo::new(0.0, None);
        io.sleep_secs(120.0);
        assert_eq!(io.simulated_sleep_secs, 120.0);
        assert_eq!(io.real_sleep_ms, 0, "time_scale 0 compresses fully");
        assert_eq!(io.sim_offset_ms, 120_000);
    }

    #[test]
    fn stub_capture_without_load_is_a_named_error() {
        let mut io = StubIo::new(1.0, None);
        let err = io.capture("s", None, None, None).unwrap_err();
        assert!(err.contains("no active stub load"), "{err}");
    }

    #[test]
    fn stub_inject_uses_the_real_spectrum_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("spec.yaml");
        std::fs::write(&p, RAMP).unwrap();
        let mut io = StubIo::new(1.0, None);
        let handle = io.inject(&p, None, None).unwrap();
        assert_eq!(handle.run_id, "stub-run-1");
        assert_eq!(handle.spectrum.sha256.len(), 64);
        assert!(io.active_run().is_some());
        let snap = io.capture("s", Some("steady"), None, None).unwrap();
        assert_eq!(snap.label.as_deref(), Some("steady"));
        io.stop_load("stub-run-1").unwrap();
        assert!(io.active_run().is_none());
        let err = io.capture("s", None, None, None).unwrap_err();
        assert!(err.contains("no active stub load"), "{err}");
    }
}
