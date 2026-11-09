//! Assertion Provider contract (WM-0701; docs/spec/assertion-provider.md §3,
//! ADR-007; wire shape mirrors `schemas/report/v1.json` `assertions` §15).
//!
//! Boundary frozen by ADR-007: Wavemaker is NOT the oracle. It schedules
//! (§4 windows), registers (results into the report), and couples
//! (quality/validity) — every judgment of "correct business state" belongs
//! to an external provider. Core types here carry NO business concepts.
//!
//! Statuses are a CLOSED set of three: pass | fail | inconclusive. A
//! provider that never returns is `inconclusive` (see
//! [`inconclusive_no_show`]) — "did not run" must never render as pass.
//! The process-boundary transport (HTTP/gRPC/CLI) is frozen separately in
//! WM-0703; the Execute/AssertionResult shape here is the contract.

use serde::{Deserialize, Serialize};

use serde_json::{Map, Value};

// --- Swell assertions config schema (spec §3) -----------------------------

/// What happens to the report/swell when the assertion fails (§3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    /// Default: record in the report only.
    #[default]
    Record,
    /// Participates in circuit-breaking (via the existing
    /// StopAdmission mechanism, WM-0507 — no new machinery).
    MarkCritical,
}

/// Execution windows (§4) — all on existing lifecycle hooks, no new
/// lifecycle phases.
///
/// The config schema is the spec §3 example, verbatim:
/// `after_scenario: <name>` / `after_recovery: true` /
/// `periodic: {interval_s: N}` — one single-key mapping per entry.
/// serde_yaml 0.9 cannot parse that shape through an externally-tagged
/// enum (it demands YAML tags), so the variants are untagged and their
/// field names ARE the config keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TriggerSpec {
    /// After a named scenario completes; window = the scenario's actual
    /// start..end.
    AfterScenario {
        after_scenario: String,
    },
    /// After a Worker/Backend recovery handshake completes (WM-0509's
    /// hook); window = fault injection start .. recovery completion.
    AfterRecovery {
        after_recovery: bool,
    },
    /// Periodic probe during execution; probes only record — they never
    /// participate in instantaneous circuit breaking.
    Periodic {
        periodic: PeriodicInterval,
    },
}

/// The `periodic:` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodicInterval {
    pub interval_s: u64,
}

/// One registered assertion instance, frozen with the Swell's plan
/// (ADR-006 plan-as-data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertionSpec {
    /// Provider implementation identifier.
    pub provider: String,
    /// Assertion instance name, unique within the report.
    pub name: String,
    /// Execution windows; at least one.
    pub when: Vec<TriggerSpec>,
    /// Execution timeout in seconds; must be positive.
    pub timeout_s: u64,
    #[serde(default)]
    pub on_fail: OnFail,
    /// Opaque business parameters passed through verbatim — Wavemaker
    /// cannot interpret them and must not try.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssertionSpecError {
    #[error("assertion name must not be empty")]
    EmptyName,
    #[error("assertion provider must not be empty")]
    EmptyProvider,
    #[error("assertion {0} declares no execution window")]
    NoTrigger(String),
    #[error("assertion {0} timeout must be positive")]
    ZeroTimeout(String),
    #[error("assertion {0} periodic interval must be positive")]
    ZeroInterval(String),
    #[error("assertion {0} after_scenario needs a scenario name")]
    EmptyScenario(String),
}

impl AssertionSpec {
    pub fn validate(&self) -> Result<(), AssertionSpecError> {
        if self.name.trim().is_empty() {
            return Err(AssertionSpecError::EmptyName);
        }
        if self.provider.trim().is_empty() {
            return Err(AssertionSpecError::EmptyProvider);
        }
        if self.when.is_empty() {
            return Err(AssertionSpecError::NoTrigger(self.name.clone()));
        }
        if self.timeout_s == 0 {
            return Err(AssertionSpecError::ZeroTimeout(self.name.clone()));
        }
        for trigger in &self.when {
            match trigger {
                TriggerSpec::AfterScenario { after_scenario }
                    if after_scenario.trim().is_empty() =>
                {
                    return Err(AssertionSpecError::EmptyScenario(
                        self.name.clone(),
                    ));
                }
                TriggerSpec::Periodic {
                    periodic: PeriodicInterval { interval_s },
                } if *interval_s == 0 => {
                    return Err(AssertionSpecError::ZeroInterval(
                        self.name.clone(),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

// --- Execute contract (spec §3.1) -----------------------------------------

/// Why an execution window fired (report schema `trigger` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    AfterScenario,
    AfterRecovery,
    Periodic,
    Other,
}

/// What the provider is told about the window (§3.1): scope identifiers,
/// the time window to check, and the trigger reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowContext {
    pub swell_id: String,
    pub backend_id: String,
    pub window_start_unix_ms: i64,
    pub window_end_unix_ms: i64,
    pub trigger: Trigger,
    /// §6.2: the pre-created evidence directory
    /// (`<swell_dir>/evidence/<assertion-name>/<timestamp>/`) the
    /// provider writes raw evidence into. Request-side only — it is not
    /// part of the report result record, so it is skipped when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowContextError {
    #[error("window context swell_id must not be empty")]
    EmptySwellId,
    #[error("window start {0} is after end {1}")]
    InvertedWindow(i64, i64),
}

impl WindowContext {
    pub fn validate(&self) -> Result<(), WindowContextError> {
        if self.swell_id.trim().is_empty() {
            return Err(WindowContextError::EmptySwellId);
        }
        if self.window_start_unix_ms > self.window_end_unix_ms {
            return Err(WindowContextError::InvertedWindow(
                self.window_start_unix_ms,
                self.window_end_unix_ms,
            ));
        }
        Ok(())
    }
}

/// The one action in the contract: Execute(name, params, window_context)
/// -> AssertionResult. In-process seam; the process-boundary transport is
/// WM-0703's freeze.
pub trait AssertionProviderContract {
    fn execute(
        &self,
        name: &str,
        params: &Map<String, Value>,
        context: &WindowContext,
    ) -> AssertionRecord;
}

// --- AssertionResult (spec §3.2; schema §15) -------------------------------

/// Closed set of three — there is no fourth implicit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionStatus {
    /// Check completed within the window, no violation found.
    Pass,
    /// Check completed and found violations: forces report/validity
    /// degradation (R5), never automatic swell termination.
    Fail,
    /// Check could not complete (timeout, provider unavailable, partial
    /// coverage): handled via quality Missing/ObserverDegraded paths,
    /// never disguised as pass.
    Inconclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Artifact,
    Query,
    Other,
}

/// One evidence reference; Wavemaker archives references verbatim and
/// never interprets the content (§6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// The result record — field names and shapes mirror the `assertions`
/// section of `schemas/report/v1.json` (`checked_at` is an RFC3339
/// string there; conversion from the pipeline's unix-ms stamps happens
/// at report writing, not here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionRecord {
    pub provider: String,
    pub name: String,
    pub status: AssertionStatus,
    /// RFC3339 timestamp (schema `format: date-time`).
    pub checked_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<RecordWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// At least one reference (schema `minItems: 1`).
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordWindow {
    /// RFC3339 timestamps (schema `format: date-time`).
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssertionRecordError {
    #[error("assertion record name must not be empty")]
    EmptyName,
    #[error("assertion record provider must not be empty")]
    EmptyProvider,
    #[error("assertion record checked_at must not be empty")]
    EmptyCheckedAt,
    #[error("assertion record needs at least one evidence reference")]
    NoEvidence,
    #[error("evidence reference must not be empty")]
    EmptyEvidenceRef,
}

impl AssertionRecord {
    pub fn validate(&self) -> Result<(), AssertionRecordError> {
        if self.name.trim().is_empty() {
            return Err(AssertionRecordError::EmptyName);
        }
        if self.provider.trim().is_empty() {
            return Err(AssertionRecordError::EmptyProvider);
        }
        if self.checked_at.trim().is_empty() {
            return Err(AssertionRecordError::EmptyCheckedAt);
        }
        if self.evidence.is_empty() {
            return Err(AssertionRecordError::NoEvidence);
        }
        for evidence in &self.evidence {
            if evidence.reference.trim().is_empty() {
                return Err(AssertionRecordError::EmptyEvidenceRef);
            }
        }
        Ok(())
    }
}

/// The no-show rule: a provider that never returned IS inconclusive —
/// the only permitted rendering of "did not run" (§3.2's forbidden
/// fourth status). `checked_at_rfc3339` is when the scheduler gave up
/// waiting: the no-show itself is an observation with a timestamp, and
/// the record stays fully valid.
pub fn inconclusive_no_show(
    provider: &str,
    name: &str,
    checked_at_rfc3339: &str,
    reason: &str,
) -> AssertionRecord {
    AssertionRecord {
        provider: provider.into(),
        name: name.into(),
        status: AssertionStatus::Inconclusive,
        checked_at: checked_at_rfc3339.into(),
        window: None,
        trigger: None,
        message: Some(format!("provider did not return: {reason}")),
        evidence: vec![EvidenceRef {
            kind: EvidenceKind::Other,
            reference: format!("scheduler://no-show/{name}"),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> AssertionSpec {
        AssertionSpec {
            provider: "game-admin-api".into(),
            name: "no-duplicate-roles".into(),
            when: vec![
                TriggerSpec::AfterScenario {
                    after_scenario: "create-role".into(),
                },
                TriggerSpec::AfterRecovery {
                    after_recovery: true,
                },
                TriggerSpec::Periodic {
                    periodic: PeriodicInterval { interval_s: 60 },
                },
            ],
            timeout_s: 30,
            on_fail: OnFail::Record,
            params: Map::new(),
        }
    }

    #[test]
    fn spec_yaml_round_trips_the_doc_example_shape() {
        let yaml = r#"
provider: game-admin-api
name: no-duplicate-roles
when:
  - after_scenario: create-role
  - after_recovery: true
  - periodic:
      interval_s: 60
timeout_s: 30
on_fail: record
params:
  endpoint: http://gm.internal/audit/duplicate-roles
  window: swell
"#;
        let parsed: AssertionSpec = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(parsed.provider, "game-admin-api");
        assert_eq!(parsed.when.len(), 3);
        assert_eq!(
            parsed.when[0],
            TriggerSpec::AfterScenario {
                after_scenario: "create-role".into()
            }
        );
        assert_eq!(
            parsed.when[1],
            TriggerSpec::AfterRecovery {
                after_recovery: true
            }
        );
        assert_eq!(
            parsed.when[2],
            TriggerSpec::Periodic {
                periodic: PeriodicInterval { interval_s: 60 }
            }
        );
        assert_eq!(parsed.on_fail, OnFail::Record);
        assert_eq!(
            parsed.params.get("window").and_then(|value| value.as_str()),
            Some("swell")
        );
        parsed.validate().expect("valid");

        let text = serde_yaml::to_string(&parsed).expect("serialise");
        let back: AssertionSpec = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, parsed);
    }

    #[test]
    fn on_fail_defaults_to_record() {
        let yaml = "
provider: p
name: n
when:
  - after_recovery: true
timeout_s: 5
";
        let parsed: AssertionSpec = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(parsed.on_fail, OnFail::Record);
    }

    #[test]
    fn spec_validation_refuses_broken_declarations() {
        let mut broken = spec();
        broken.name = " ".into();
        assert_eq!(
            broken.validate(),
            Err(AssertionSpecError::EmptyName)
        );

        let mut broken = spec();
        broken.provider = "".into();
        assert_eq!(
            broken.validate(),
            Err(AssertionSpecError::EmptyProvider)
        );

        let mut broken = spec();
        broken.when = vec![];
        assert_eq!(
            broken.validate(),
            Err(AssertionSpecError::NoTrigger("no-duplicate-roles".into()))
        );

        let mut broken = spec();
        broken.timeout_s = 0;
        assert!(matches!(
            broken.validate(),
            Err(AssertionSpecError::ZeroTimeout(_))
        ));

        let mut broken = spec();
        broken.when = vec![TriggerSpec::Periodic {
            periodic: PeriodicInterval { interval_s: 0 },
        }];
        assert!(matches!(
            broken.validate(),
            Err(AssertionSpecError::ZeroInterval(_))
        ));

        let mut broken = spec();
        broken.when = vec![TriggerSpec::AfterScenario {
            after_scenario: "  ".into(),
        }];
        assert!(matches!(
            broken.validate(),
            Err(AssertionSpecError::EmptyScenario(_))
        ));
    }

    #[test]
    fn status_set_is_closed_three_state() {
        for (text, status) in [
            ("pass", AssertionStatus::Pass),
            ("fail", AssertionStatus::Fail),
            ("inconclusive", AssertionStatus::Inconclusive),
        ] {
            let parsed: AssertionStatus =
                serde_yaml::from_str(text).expect("parse");
            assert_eq!(parsed, status);
        }
        let fourth: Result<AssertionStatus, _> =
            serde_yaml::from_str::<AssertionStatus>("skipped");
        assert!(fourth.is_err(), "no fourth implicit status may parse");
    }

    #[test]
    fn window_context_validates_scope_and_range() {
        let context = WindowContext {
            swell_id: "sw-1".into(),
            backend_id: "kb-conf".into(),
            window_start_unix_ms: 100,
            window_end_unix_ms: 200,
            trigger: Trigger::AfterRecovery,
            evidence_dir: None,
        };
        context.validate().expect("valid");

        let mut inverted = context.clone();
        inverted.window_start_unix_ms = 300;
        assert_eq!(
            inverted.validate(),
            Err(WindowContextError::InvertedWindow(300, 200))
        );

        let mut anonymous = context;
        anonymous.swell_id = " ".into();
        assert_eq!(
            anonymous.validate(),
            Err(WindowContextError::EmptySwellId)
        );
    }

    #[test]
    fn record_round_trips_with_schema_field_names() {
        let record = AssertionRecord {
            provider: "game-admin-api".into(),
            name: "no-duplicate-roles".into(),
            status: AssertionStatus::Fail,
            checked_at: "2026-11-10T00:00:00Z".into(),
            window: Some(RecordWindow {
                start: "2026-11-10T00:00:00Z".into(),
                end: "2026-11-10T00:05:00Z".into(),
            }),
            trigger: Some(Trigger::AfterScenario),
            message: Some(
                "3 duplicate role pairs found for accounts 1042,1087,2210".into(),
            ),
            evidence: vec![
                EvidenceRef {
                    kind: EvidenceKind::Artifact,
                    reference: "swell-001/evidence/no-duplicate-roles-diff.json".into(),
                },
                EvidenceRef {
                    kind: EvidenceKind::Query,
                    reference:
                        "SELECT count(*) FROM roles GROUP BY account HAVING count(*)>1"
                            .into(),
                },
            ],
        };
        record.validate().expect("valid");
        let json = serde_json::to_string(&record).expect("json");
        assert!(json.contains("\"ref\""));
        assert!(!json.contains("\"inconclusive\""));
        assert!(json.contains("\"fail\""));
        let back: AssertionRecord = serde_json::from_str(&json).expect("reparse");
        assert_eq!(back, record);
    }

    #[test]
    fn record_without_evidence_is_refused() {
        let record = AssertionRecord {
            provider: "p".into(),
            name: "n".into(),
            status: AssertionStatus::Pass,
            checked_at: "2026-11-10T00:00:00Z".into(),
            window: None,
            trigger: None,
            message: None,
            evidence: vec![],
        };
        assert_eq!(
            record.validate(),
            Err(AssertionRecordError::NoEvidence)
        );
    }

    #[test]
    fn no_show_renders_inconclusive_never_pass() {
        let record = inconclusive_no_show(
            "game-admin-api",
            "no-duplicate-roles",
            "2026-11-10T00:00:30Z",
            "timeout after 30s",
        );
        assert_eq!(record.checked_at, "2026-11-10T00:00:30Z");
        assert_eq!(record.status, AssertionStatus::Inconclusive);
        assert!(record
            .message
            .as_ref()
            .expect("message")
            .contains("provider did not return"));
        record.validate().expect("no-show still valid");
    }

    #[test]
    fn contract_trait_one_action_one_result() {
        struct DuplicateRoleAudit;

        impl AssertionProviderContract for DuplicateRoleAudit {
            fn execute(
                &self,
                name: &str,
                params: &Map<String, Value>,
                context: &WindowContext,
            ) -> AssertionRecord {
                // The provider — not Wavemaker — owns the semantics. The
                // endpoint it queries comes from the passthrough params.
                let endpoint = params
                    .get("endpoint")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                AssertionRecord {
                    provider: "game-admin-api".into(),
                    name: name.into(),
                    status: if endpoint.is_empty() {
                        AssertionStatus::Inconclusive
                    } else {
                        AssertionStatus::Pass
                    },
                    checked_at: "2026-11-10T00:00:00Z".into(),
                    window: Some(RecordWindow {
                        start: format!("{}", context.window_start_unix_ms),
                        end: format!("{}", context.window_end_unix_ms),
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

        let context = WindowContext {
            swell_id: "sw-1".into(),
            backend_id: "kb-conf".into(),
            window_start_unix_ms: 0,
            window_end_unix_ms: 1000,
            trigger: Trigger::Periodic,
            evidence_dir: None,
        };
        let mut params = Map::new();
        params.insert(
            "endpoint".into(),
            Value::String("http://gm.internal/audit".into()),
        );
        let record = DuplicateRoleAudit.execute("no-duplicate-roles", &params, &context);
        assert_eq!(record.status, AssertionStatus::Pass);
        assert_eq!(record.trigger, Some(Trigger::Periodic));

        let record = DuplicateRoleAudit.execute("no-duplicate-roles", &Map::new(), &context);
        assert_eq!(record.status, AssertionStatus::Inconclusive);
    }
}
