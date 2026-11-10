//! Assertion registration and execution-window scheduling (WM-0702;
//! docs/spec/assertion-provider.md §4).
//!
//! The three execution windows all land on existing lifecycle hooks — no
//! new lifecycle phases: after_scenario fires when a scenario finishes
//! (window = the scenario's actual start..end), after_recovery fires when
//! a recovery handshake completes (window = fault-injection start ..
//! recovery completion; the WM-0509 hook feeds it), and periodic probes
//! sample on a fixed interval (record-only — never instantaneous
//! circuit-breaking).
//!
//! Scheduling honesty rules encoded here:
//! - assertions never fabricate results: a spec whose provider
//!   implementation is not registered yields `inconclusive` via
//!   [`inconclusive_no_show`], never pass;
//! - after_scenario/after_recovery windows are mandatory: specs that
//!   never fired by swell wrap-up surface as `inconclusive` through
//!   [`AssertionScheduler::finalize_unfired`] — "registered but never
//!   checked" is visible, not silently dropped;
//! - each fired window executes exactly once and accumulates outcomes in
//!   order, so the report's results[] cannot lose or double-count runs;
//! - `checked_at` for no-show records comes from the caller's clock
//!   (RFC3339 string per the report schema) — the scheduler never
//!   invents timestamps.

use std::collections::BTreeMap;

use crate::assertions::{
    inconclusive_no_show, AssertionProviderContract, AssertionRecord, AssertionSpec, OnFail,
    Trigger, TriggerSpec, WindowContext,
};

/// One executed assertion: the record plus the spec's failure policy —
/// the report consumes `record`; circuit-breaking (WM-0507's
/// StopAdmission) consumes `on_fail`. They travel together so the
/// coupling data cannot diverge from what the report shows.
#[derive(Debug, Clone, PartialEq)]
pub struct AssertionOutcome {
    pub record: AssertionRecord,
    pub on_fail: OnFail,
}

/// Registered provider implementations, keyed by the spec's `provider`
/// identifier. Specs are configuration; providers are code — the join is
/// resolved at execution time and a missing entry is a no-show.
#[derive(Default)]
pub struct ProviderSet<'a> {
    entries: BTreeMap<String, &'a dyn AssertionProviderContract>,
}

impl<'a> ProviderSet<'a> {
    pub fn put(&mut self, name: &str, provider: &'a dyn AssertionProviderContract) {
        self.entries.insert(name.to_string(), provider);
    }

    pub fn resolve(&self, name: &str) -> Option<&'a dyn AssertionProviderContract> {
        self.entries.get(name).copied()
    }
}

/// Scope of a finished scenario (§4.1): the actual start..end the
/// scenario ran for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioWindow {
    pub swell_id: String,
    pub backend_id: String,
    pub scenario: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

/// Scope of a completed recovery handshake (§4.2): from fault-injection
/// start to recovery completion (WM-0509's hook reports these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryWindow {
    pub swell_id: String,
    pub backend_id: String,
    pub fault_start_unix_ms: i64,
    pub recovered_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssertionSchedulerError {
    #[error("assertion {0} is invalid: {1}")]
    InvalidSpec(String, String),
    #[error("assertion name {0} is declared more than once; names are unique within a report")]
    DuplicateName(String),
}

/// The registry + window scheduler for one swell's assertions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssertionScheduler {
    specs: Vec<AssertionSpec>,
    /// Next due instant (unix ms) per spec index; `None` = first periodic
    /// probe not yet observed.
    next_periodic_due: Vec<Option<i64>>,
    /// Specs (by index) that have fired at least once through a mandatory
    /// window (after_scenario / after_recovery).
    mandatory_fired: Vec<bool>,
    /// §6.2: per-assertion evidence directory pre-created by the
    /// Controller at registration; injected into every window the
    /// assertion executes in.
    evidence_dirs: BTreeMap<String, String>,
    outcomes: Vec<AssertionOutcome>,
}

impl AssertionScheduler {
    /// Register a swell's assertion set. Every spec must be valid and
    /// names are unique within the report (spec §3).
    pub fn register(specs: Vec<AssertionSpec>) -> Result<Self, AssertionSchedulerError> {
        let mut seen = std::collections::BTreeSet::new();
        for spec in &specs {
            spec.validate().map_err(|err| {
                AssertionSchedulerError::InvalidSpec(spec.name.clone(), err.to_string())
            })?;
            if !seen.insert(spec.name.clone()) {
                return Err(AssertionSchedulerError::DuplicateName(spec.name.clone()));
            }
        }
        let count = specs.len();
        Ok(Self {
            specs,
            next_periodic_due: vec![None; count],
            mandatory_fired: vec![false; count],
            evidence_dirs: BTreeMap::new(),
            outcomes: Vec::new(),
        })
    }

    /// §6.2: register the pre-created evidence directory for one
    /// assertion instance; it rides in every window that assertion
    /// executes in, so the provider knows where to write raw evidence.
    pub fn attach_evidence_dir(&mut self, assertion_name: &str, dir: String) {
        self.evidence_dirs.insert(assertion_name.to_string(), dir);
    }

    pub fn specs(&self) -> &[AssertionSpec] {
        &self.specs
    }

    pub fn outcomes(&self) -> &[AssertionOutcome] {
        &self.outcomes
    }

    /// after_scenario (§4.1): a scenario finished; window = the
    /// scenario's actual start..end. `checked_at_rfc3339` is the caller's
    /// clock at the moment of execution.
    pub fn on_scenario_finished(
        &mut self,
        providers: &ProviderSet<'_>,
        context: &ScenarioWindow,
        checked_at_rfc3339: &str,
    ) -> Vec<AssertionOutcome> {
        let mut fired = Vec::new();
        let mut matches = Vec::new();
        for (index, spec) in self.specs.iter().enumerate() {
            for trigger in &spec.when {
                if let TriggerSpec::AfterScenario { after_scenario } = trigger {
                    if after_scenario == &context.scenario {
                        matches.push(index);
                    }
                }
            }
        }
        for index in matches {
            self.mandatory_fired[index] = true;
            let window = WindowContext {
                swell_id: context.swell_id.clone(),
                backend_id: context.backend_id.clone(),
                window_start_unix_ms: context.start_unix_ms,
                window_end_unix_ms: context.end_unix_ms,
                trigger: Trigger::AfterScenario,
                evidence_dir: None,
            };
            fired.extend(self.execute_spec(index, providers, window, checked_at_rfc3339));
        }
        fired
    }

    /// after_recovery (§4.2): a recovery handshake completed; window =
    /// fault-injection start .. recovery completion. `after_recovery:
    /// false` disables the window — it never fires.
    pub fn on_recovery_completed(
        &mut self,
        providers: &ProviderSet<'_>,
        context: &RecoveryWindow,
        checked_at_rfc3339: &str,
    ) -> Vec<AssertionOutcome> {
        let mut fired = Vec::new();
        let mut matches = Vec::new();
        for (index, spec) in self.specs.iter().enumerate() {
            for trigger in &spec.when {
                if let TriggerSpec::AfterRecovery { after_recovery } = trigger {
                    if *after_recovery {
                        matches.push(index);
                    }
                }
            }
        }
        for index in matches {
            self.mandatory_fired[index] = true;
            let window = WindowContext {
                swell_id: context.swell_id.clone(),
                backend_id: context.backend_id.clone(),
                window_start_unix_ms: context.fault_start_unix_ms,
                window_end_unix_ms: context.recovered_at_unix_ms,
                trigger: Trigger::AfterRecovery,
                evidence_dir: None,
            };
            fired.extend(self.execute_spec(index, providers, window, checked_at_rfc3339));
        }
        fired
    }

    /// periodic (§4.3): probes sample on the fixed interval during
    /// execution. The first observed tick fires immediately; each later
    /// firing lands on an interval boundary, and the probe window covers
    /// the last interval so the provider can scope "what changed since
    /// the previous probe". Probes record only — the caller must never
    /// feed them into instantaneous circuit-breaking.
    pub fn on_tick(
        &mut self,
        providers: &ProviderSet<'_>,
        swell_id: &str,
        backend_id: &str,
        now_unix_ms: i64,
        checked_at_rfc3339: &str,
    ) -> Vec<AssertionOutcome> {
        let mut fired = Vec::new();
        let mut due_now = Vec::new();
        for (index, spec) in self.specs.iter().enumerate() {
            for trigger in &spec.when {
                if let TriggerSpec::Periodic { periodic } = trigger {
                    let interval_ms = (periodic.interval_s * 1000) as i64;
                    let due = self.next_periodic_due[index].unwrap_or(now_unix_ms);
                    if now_unix_ms >= due {
                        due_now.push((index, due, interval_ms));
                    }
                }
            }
        }
        for (index, due, interval_ms) in due_now {
            let window = WindowContext {
                swell_id: swell_id.into(),
                backend_id: backend_id.into(),
                window_start_unix_ms: (due - interval_ms).max(0),
                window_end_unix_ms: due,
                trigger: Trigger::Periodic,
                evidence_dir: None,
            };
            self.next_periodic_due[index] = Some(due + interval_ms);
            fired.extend(self.execute_spec(index, providers, window, checked_at_rfc3339));
        }
        fired
    }

    /// Swell wrap-up (§4 "收尾前全部收齐；逾期未返回按 inconclusive"):
    /// every registered after_scenario/after_recovery spec that never
    /// fired is rendered as a valid inconclusive record — registered but
    /// never checked stays visible in the report. Periodic probes are
    /// opportunistic samples and are exempt.
    pub fn finalize_unfired(
        &mut self,
        checked_at_rfc3339: &str,
        reason: &str,
    ) -> Vec<AssertionOutcome> {
        let mut rendered = Vec::new();
        for (index, spec) in self.specs.iter().enumerate() {
            let mandatory = spec.when.iter().any(|trigger| {
                matches!(
                    trigger,
                    TriggerSpec::AfterScenario { .. } | TriggerSpec::AfterRecovery { .. }
                )
            });
            if mandatory && !self.mandatory_fired[index] {
                self.mandatory_fired[index] = true;
                let outcome = AssertionOutcome {
                    record: inconclusive_no_show(
                        &spec.provider,
                        &spec.name,
                        checked_at_rfc3339,
                        reason,
                    ),
                    on_fail: spec.on_fail,
                };
                self.outcomes.push(outcome.clone());
                rendered.push(outcome);
            }
        }
        rendered
    }

    /// Execute one spec in one window; a spec whose provider
    /// implementation is missing is a no-show → inconclusive.
    fn execute_spec(
        &mut self,
        index: usize,
        providers: &ProviderSet<'_>,
        mut window: WindowContext,
        checked_at_rfc3339: &str,
    ) -> Vec<AssertionOutcome> {
        let spec = &self.specs[index];
        if window.evidence_dir.is_none() {
            if let Some(dir) = self.evidence_dirs.get(&spec.name) {
                window.evidence_dir = Some(dir.clone());
            }
        }
        let outcome = match providers.resolve(&spec.provider) {
            Some(provider) => AssertionOutcome {
                record: provider.execute(&spec.name, &spec.params, &window),
                on_fail: spec.on_fail,
            },
            None => AssertionOutcome {
                record: inconclusive_no_show(
                    &spec.provider,
                    &spec.name,
                    checked_at_rfc3339,
                    &format!("provider {} is not registered", spec.provider),
                ),
                on_fail: spec.on_fail,
            },
        };
        self.outcomes.push(outcome.clone());
        vec![outcome]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    use crate::assertions::{
        AssertionStatus, EvidenceKind, EvidenceRef, PeriodicInterval, RecordWindow,
    };

    fn providers_with(recording: &Recording) -> ProviderSet<'_> {
        let mut providers = ProviderSet::default();
        providers.put("audit", recording);
        providers
    }

    fn scenario_spec(name: &str, scenario: &str) -> AssertionSpec {
        AssertionSpec {
            provider: "audit".into(),
            name: name.into(),
            when: vec![TriggerSpec::AfterScenario {
                after_scenario: scenario.into(),
            }],
            timeout_s: 30,
            on_fail: OnFail::Record,
            params: Map::new(),
        }
    }

    impl ScenarioWindow {
        fn sample() -> Self {
            Self {
                swell_id: "sw-1".into(),
                backend_id: "kb-conf".into(),
                scenario: "create-role".into(),
                start_unix_ms: 1_000,
                end_unix_ms: 9_000,
            }
        }
    }

    /// A provider that records every window it was given and returns a
    /// pass carrying the trigger back — enough to assert scheduling.
    struct Recording {
        seen_windows: std::cell::RefCell<Vec<WindowContext>>,
    }

    impl Recording {
        fn new() -> Self {
            Self {
                seen_windows: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl AssertionProviderContract for Recording {
        fn execute(
            &self,
            name: &str,
            _params: &Map<String, Value>,
            context: &WindowContext,
        ) -> AssertionRecord {
            self.seen_windows.borrow_mut().push(context.clone());
            AssertionRecord {
                provider: "audit".into(),
                name: name.into(),
                status: AssertionStatus::Pass,
                checked_at: "2026-11-10T00:00:00Z".into(),
                window: Some(RecordWindow {
                    start: context.window_start_unix_ms.to_string(),
                    end: context.window_end_unix_ms.to_string(),
                }),
                trigger: Some(context.trigger),
                message: None,
                evidence: vec![EvidenceRef {
                    kind: EvidenceKind::Query,
                    reference: "SELECT 1".into(),
                }],
            }
        }
    }

    #[test]
    fn registration_validates_and_refuses_duplicates() {
        AssertionScheduler::register(vec![scenario_spec("a", "s"), scenario_spec("b", "t")])
            .expect("valid set");

        let duplicate =
            AssertionScheduler::register(vec![scenario_spec("a", "s"), scenario_spec("a", "t")]);
        assert_eq!(
            duplicate,
            Err(AssertionSchedulerError::DuplicateName("a".into()))
        );

        let mut invalid = scenario_spec("a", "s");
        invalid.timeout_s = 0;
        assert!(matches!(
            AssertionScheduler::register(vec![invalid]),
            Err(AssertionSchedulerError::InvalidSpec(_, _))
        ));
    }

    #[test]
    fn scenario_finish_fires_matching_window_with_actual_times() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let mut scheduler = AssertionScheduler::register(vec![
            scenario_spec("dupes", "create-role"),
            scenario_spec("other", "delete-role"),
        ])
        .expect("register");

        let fired = scheduler.on_scenario_finished(
            &providers,
            &ScenarioWindow::sample(),
            "2026-11-10T00:01:00Z",
        );
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].record.status, AssertionStatus::Pass);
        assert_eq!(fired[0].record.trigger, Some(Trigger::AfterScenario));
        {
            let seen = recording.seen_windows.borrow();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].window_start_unix_ms, 1_000);
            assert_eq!(seen[0].window_end_unix_ms, 9_000);
            assert_eq!(seen[0].trigger, Trigger::AfterScenario);
        }
        assert_eq!(scheduler.outcomes().len(), 1);

        // A second, differently-scoped scenario event fires only its own
        // spec; outcomes accumulate in order.
        scheduler.on_scenario_finished(
            &providers,
            &ScenarioWindow {
                scenario: "delete-role".into(),
                ..ScenarioWindow::sample()
            },
            "2026-11-10T00:02:00Z",
        );
        assert_eq!(scheduler.outcomes().len(), 2);
    }

    #[test]
    fn recovery_window_uses_fault_start_to_recovery_end() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let enabled = AssertionSpec {
            name: "reward-audit".into(),
            when: vec![TriggerSpec::AfterRecovery {
                after_recovery: true,
            }],
            ..scenario_spec("reward-audit", "unused")
        };
        let disabled = AssertionSpec {
            name: "reward-disabled".into(),
            when: vec![TriggerSpec::AfterRecovery {
                after_recovery: false,
            }],
            ..scenario_spec("reward-disabled", "unused")
        };
        let mut scheduler =
            AssertionScheduler::register(vec![enabled, disabled]).expect("register");

        let fired = scheduler.on_recovery_completed(
            &providers,
            &RecoveryWindow {
                swell_id: "sw-1".into(),
                backend_id: "kb-conf".into(),
                fault_start_unix_ms: 5_000,
                recovered_at_unix_ms: 12_000,
            },
            "2026-11-10T00:03:00Z",
        );
        // after_recovery: false never fires; true fires exactly once.
        assert_eq!(fired.len(), 1);
        let seen = recording.seen_windows.borrow();
        assert_eq!(seen[0].window_start_unix_ms, 5_000);
        assert_eq!(seen[0].window_end_unix_ms, 12_000);
        assert_eq!(seen[0].trigger, Trigger::AfterRecovery);
    }

    #[test]
    fn periodic_fires_immediately_then_on_boundaries() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let mut scheduler = AssertionScheduler::register(vec![AssertionSpec {
            name: "probe".into(),
            when: vec![TriggerSpec::Periodic {
                periodic: PeriodicInterval { interval_s: 60 },
            }],
            ..scenario_spec("probe", "unused")
        }])
        .expect("register");

        assert_eq!(
            scheduler
                .on_tick(&providers, "sw-1", "kb", 1_000, "2026-11-10T00:00:01Z")
                .len(),
            1,
            "first observed tick fires immediately"
        );
        assert_eq!(
            scheduler
                .on_tick(&providers, "sw-1", "kb", 31_000, "2026-11-10T00:00:31Z")
                .len(),
            0,
            "between boundaries nothing fires"
        );
        let fired = scheduler.on_tick(&providers, "sw-1", "kb", 61_000, "2026-11-10T00:01:01Z");
        assert_eq!(fired.len(), 1);
        let seen = recording.seen_windows.borrow();
        assert_eq!(seen[1].trigger, Trigger::Periodic);
        assert_eq!(seen[1].window_end_unix_ms, 61_000);
        assert_eq!(scheduler.outcomes().len(), 2);
    }

    #[test]
    fn unregistered_provider_is_inconclusive_never_pass() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let spec = AssertionSpec {
            provider: "ghost".into(),
            ..scenario_spec("orphan", "create-role")
        };
        let mut scheduler = AssertionScheduler::register(vec![spec]).expect("register");
        let fired = scheduler.on_scenario_finished(
            &providers,
            &ScenarioWindow::sample(),
            "2026-11-10T00:01:00Z",
        );
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].record.status, AssertionStatus::Inconclusive);
        assert_eq!(fired[0].record.checked_at, "2026-11-10T00:01:00Z");
        assert!(fired[0]
            .record
            .message
            .as_ref()
            .expect("message")
            .contains("ghost"));
    }

    #[test]
    fn finalize_unfired_covers_mandatory_windows_only() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let mut scheduler = AssertionScheduler::register(vec![
            scenario_spec("never-fired", "scenario-that-never-ran"),
            AssertionSpec {
                name: "probe".into(),
                when: vec![TriggerSpec::Periodic {
                    periodic: PeriodicInterval { interval_s: 60 },
                }],
                ..scenario_spec("probe", "unused")
            },
        ])
        .expect("register");
        // The periodic probe runs; the mandatory scenario window never does.
        scheduler.on_tick(&providers, "sw-1", "kb", 0, "2026-11-10T00:00:00Z");

        let rendered = scheduler.finalize_unfired(
            "2026-11-10T01:00:00Z",
            "swell finished before the provider returned",
        );
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].record.status, AssertionStatus::Inconclusive);
        assert_eq!(rendered[0].record.checked_at, "2026-11-10T01:00:00Z");
        // Second finalize is idempotent — the missing window is named once.
        assert!(scheduler
            .finalize_unfired("2026-11-10T01:00:01Z", "again")
            .is_empty());
    }

    #[test]
    fn attached_evidence_dir_rides_in_the_window_context() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let mut scheduler =
            AssertionScheduler::register(vec![scenario_spec("no-duplicate-roles", "create-role")])
                .expect("register");
        // §6.2: the Controller pre-creates the directory at registration
        // and attaches it; every window the assertion executes in
        // carries it.
        scheduler.attach_evidence_dir(
            "no-duplicate-roles",
            "evidence/no-duplicate-roles/20261110T000000Z".into(),
        );
        scheduler.on_scenario_finished(
            &providers,
            &ScenarioWindow::sample(),
            "2026-11-10T00:01:00Z",
        );
        {
            let seen = recording.seen_windows.borrow();
            assert_eq!(
                seen[0].evidence_dir.as_deref(),
                Some("evidence/no-duplicate-roles/20261110T000000Z")
            );
        }
        // An assertion without an attached dir carries None — no path is
        // invented on its behalf.
        let mut bare =
            AssertionScheduler::register(vec![scenario_spec("other-audit", "create-role")])
                .expect("register");
        bare.on_scenario_finished(
            &providers,
            &ScenarioWindow::sample(),
            "2026-11-10T00:01:00Z",
        );
        let seen = recording.seen_windows.borrow();
        assert_eq!(seen[1].evidence_dir, None);
    }

    #[test]
    fn outcomes_carry_on_fail_for_circuit_breaking() {
        let recording = Recording::new();
        let providers = providers_with(&recording);
        let critical = AssertionSpec {
            name: "critical-audit".into(),
            on_fail: OnFail::MarkCritical,
            ..scenario_spec("critical-audit", "create-role")
        };
        let mut scheduler = AssertionScheduler::register(vec![critical]).expect("register");
        let fired = scheduler.on_scenario_finished(
            &providers,
            &ScenarioWindow::sample(),
            "2026-11-10T00:01:00Z",
        );
        assert_eq!(fired[0].on_fail, OnFail::MarkCritical);
        assert_eq!(scheduler.outcomes()[0].on_fail, OnFail::MarkCritical);
    }
}
