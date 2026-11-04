//! Spectrum configuration schema and field validation.
//!
//! Semantics follow docs/spec/spectrum-semantics.md (BLOCKER-1 closure,
//! §2/§5/§6): arrival, lifetime, concurrency and saturation policy are
//! independent fields and must never be collapsed into a single `users`
//! number. Validation output is always explainable (§6 rule 5): every issue
//! carries a stable code, the offending field and a message with the
//! assumption that produced it.

use serde::{Deserialize, Serialize};

/// Arrival-rate ramp: λ(t) grows exponentially from `from_per_second` to
/// `to_per_second` over `duration_secs`, then holds the end rate
/// (spectrum-semantics §3; post-ramp hold is the documented assumption).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RampSpec {
    pub from_per_second: f64,
    pub to_per_second: f64,
    #[serde(alias = "duration")]
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArrivalSpec {
    pub ramp: RampSpec,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifetimeDistribution {
    Fixed,
    Exponential,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifetimeSpec {
    pub distribution: LifetimeDistribution,
    pub mean_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConcurrencySpec {
    /// Control value only — never converted via naive λ·W (§4.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SaturationPolicy {
    RejectNew,
    Queue,
    Delay,
}

/// Top-level spectrum config (YAML).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpectrumConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival: Option<ArrivalSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<LifetimeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<ConcurrencySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation_policy: Option<SaturationPolicy>,
}

/// Severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One explainable validation output (spectrum-semantics §6 rule 5).
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `E_ARRIVAL_MISSING`.
    pub code: &'static str,
    /// Field path the issue refers to, e.g. `arrival.ramp.from_per_second`.
    pub field: &'static str,
    pub message: String,
}

/// Relative deviation above which `concurrency.target` is flagged against
/// the naive λ·W estimate (§6: threshold is an engineering suggestion).
pub const TARGET_DEVIATION_THRESHOLD: f64 = 0.10;

impl SpectrumConfig {
    /// Validate field constraints; returns all issues (errors + warnings).
    ///
    /// This is the structural pass of WM-0111 `wm validate`. Full active-
    /// curve estimation (§6 rule 2) is a later integration.
    pub fn validate(&self) -> Vec<Violation> {
        let mut out = Vec::new();

        // BLOCKER-1 closure §8.1: the four field groups are indispensable.
        let Some(arrival) = &self.arrival else {
            out.push(missing("E_ARRIVAL_MISSING", "arrival", "`arrival` is required: arrival rate must be expressed explicitly, never collapsed into `users`"));
            return out;
        };
        let Some(lifetime) = &self.lifetime else {
            out.push(missing("E_LIFETIME_MISSING", "lifetime", "`lifetime` is required: mean alone is ambiguous without `lifetime.distribution` (fixed vs exponential diverge >600 active bots at t=600s in the reference scenario)"));
            return out;
        };

        let ramp = &arrival.ramp;
        for (field, value, name) in [
            (
                "arrival.ramp.from_per_second",
                ramp.from_per_second,
                "from_per_second",
            ),
            (
                "arrival.ramp.to_per_second",
                ramp.to_per_second,
                "to_per_second",
            ),
            (
                "arrival.ramp.duration_secs",
                ramp.duration_secs,
                "duration_secs",
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                out.push(Violation {
                    severity: Severity::Error,
                    code: "E_VALUE_NOT_POSITIVE",
                    field,
                    message: format!("{name} must be a positive finite number, got {value}"),
                });
            }
        }

        if !lifetime.mean_secs.is_finite() || lifetime.mean_secs <= 0.0 {
            out.push(Violation {
                severity: Severity::Error,
                code: "E_VALUE_NOT_POSITIVE",
                field: "lifetime.mean_secs",
                message: format!("lifetime mean must be positive, got {}", lifetime.mean_secs),
            });
        }

        if let Some(conc) = &self.concurrency {
            if let Some(max) = conc.max {
                if !max.is_finite() || max <= 0.0 {
                    out.push(Violation {
                        severity: Severity::Error,
                        code: "E_VALUE_NOT_POSITIVE",
                        field: "concurrency.max",
                        message: format!("concurrency.max must be positive, got {max}"),
                    });
                }
            }
            if let (Some(target), Some(max)) = (conc.target, conc.max) {
                if target > max {
                    out.push(Violation {
                        severity: Severity::Error,
                        code: "E_TARGET_ABOVE_MAX",
                        field: "concurrency.target",
                        message: format!(
                            "concurrency.target ({target}) must not exceed concurrency.max ({max})"
                        ),
                    });
                }
            }
        }

        // saturation policy semantics (§5): a hard cap without a policy
        // would silently drop or stall load at saturation.
        let has_max = self.concurrency.as_ref().is_some_and(|c| c.max.is_some());
        match (&self.saturation_policy, has_max) {
            (None, true) => out.push(Violation {
                severity: Severity::Error,
                code: "E_POLICY_MISSING",
                field: "saturation_policy",
                message: "concurrency.max is set but saturation_policy is missing: reaching the cap without a policy would silently drop or stall arrivals; choose reject_new, queue or delay".into(),
            }),
            (Some(_), false) => out.push(Violation {
                severity: Severity::Warning,
                code: "W_POLICY_WITHOUT_CAP",
                field: "saturation_policy",
                message: "saturation_policy has no effect because concurrency.max is not set".into(),
            }),
            _ => {}
        }

        // §6 rule 1/3: arrival + concurrency.target may coexist; compare the
        // target against the naive steady-state λ·W estimate and warn (never
        // reject) when they diverge beyond the threshold.
        if let (Some(arrival), Some(lifetime), Some(conc)) =
            (&self.arrival, &self.lifetime, &self.concurrency)
        {
            if let Some(target) = conc.target {
                let naive = arrival.ramp.to_per_second * lifetime.mean_secs;
                if target > 0.0 {
                    let deviation = (naive - target).abs() / target;
                    if deviation > TARGET_DEVIATION_THRESHOLD {
                        out.push(Violation {
                            severity: Severity::Warning,
                            code: "W_TARGET_NAIVE_LITTLE_DEVIATION",
                            field: "concurrency.target",
                            message: format!(
                                "naive λ·W = {naive:.1} (λ_end={} · mean={}) deviates {:.1}% from concurrency.target ({target}); Little's Law is only valid in steady state (§4.4) — run the active-curve estimator for a real prediction",
                                arrival.ramp.to_per_second, lifetime.mean_secs, deviation * 100.0
                            ),
                        });
                    }
                }
            }
        }

        out
    }

    /// True when no error-severity issues were found.
    pub fn is_valid(&self) -> bool {
        !self
            .validate()
            .iter()
            .any(|v| v.severity == Severity::Error)
    }
}

fn missing(code: &'static str, field: &'static str, message: &str) -> Violation {
    Violation {
        severity: Severity::Error,
        code,
        field,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> SpectrumConfig {
        SpectrumConfig {
            arrival: Some(ArrivalSpec {
                ramp: RampSpec {
                    from_per_second: 1.0,
                    to_per_second: 20.0,
                    duration_secs: 600.0,
                },
            }),
            lifetime: Some(LifetimeSpec {
                distribution: LifetimeDistribution::Fixed,
                mean_secs: 900.0,
            }),
            concurrency: Some(ConcurrencySpec {
                target: Some(5000.0),
                max: Some(5000.0),
            }),
            saturation_policy: Some(SaturationPolicy::Queue),
        }
    }

    #[test]
    fn reference_scenario_is_valid() {
        // spectrum-semantics §1: 1/s -> 20/s over 600s, mean 900s, cap 5000.
        let cfg = base_config();
        let issues = cfg.validate();
        // λ_end·mean = 18000 vs target 5000 -> naive deviation warning is
        // expected and exactly the BLOCKER-1 lesson.
        assert!(cfg.is_valid(), "reference scenario must pass");
        assert!(issues
            .iter()
            .any(|v| v.code == "W_TARGET_NAIVE_LITTLE_DEVIATION"));
    }

    #[test]
    fn missing_arrival_is_an_error() {
        let cfg = SpectrumConfig::default();
        let issues = cfg.validate();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "E_ARRIVAL_MISSING");
        assert!(!cfg.is_valid());
    }

    #[test]
    fn missing_lifetime_is_an_error() {
        let mut cfg = base_config();
        cfg.lifetime = None;
        let issues = cfg.validate();
        assert_eq!(issues[0].code, "E_LIFETIME_MISSING");
        assert!(issues[0].message.contains("distribution"));
    }

    #[test]
    fn non_positive_values_are_errors() {
        let mut cfg = base_config();
        cfg.arrival.as_mut().unwrap().ramp.duration_secs = 0.0;
        cfg.lifetime.as_mut().unwrap().mean_secs = -5.0;
        let issues = cfg.validate();
        assert!(issues
            .iter()
            .any(|v| v.code == "E_VALUE_NOT_POSITIVE" && v.field == "arrival.ramp.duration_secs"));
        assert!(issues
            .iter()
            .any(|v| v.code == "E_VALUE_NOT_POSITIVE" && v.field == "lifetime.mean_secs"));
        assert!(!cfg.is_valid());
    }

    #[test]
    fn target_above_max_is_an_error() {
        let mut cfg = base_config();
        cfg.concurrency.as_mut().unwrap().target = Some(6000.0);
        let issues = cfg.validate();
        assert!(issues.iter().any(|v| v.code == "E_TARGET_ABOVE_MAX"));
    }

    #[test]
    fn cap_without_policy_is_rejected() {
        let mut cfg = base_config();
        cfg.saturation_policy = None;
        let issues = cfg.validate();
        assert!(issues.iter().any(|v| v.code == "E_POLICY_MISSING"));
        assert!(!cfg.is_valid());
    }

    #[test]
    fn policy_without_cap_warns_but_passes() {
        let mut cfg = base_config();
        cfg.concurrency.as_mut().unwrap().max = None;
        let issues = cfg.validate();
        assert!(issues.iter().any(|v| v.code == "W_POLICY_WITHOUT_CAP"));
        assert!(cfg.is_valid());
    }

    #[test]
    fn target_close_to_naive_estimate_is_silent() {
        // λ_end·mean = 20·900 = 18000; pick target 18500 (~2.7% off).
        let mut cfg = base_config();
        cfg.concurrency.as_mut().unwrap().target = Some(18500.0);
        let issues = cfg.validate();
        assert!(!issues
            .iter()
            .any(|v| v.code == "W_TARGET_NAIVE_LITTLE_DEVIATION"));
    }

    #[test]
    fn yaml_roundtrip_uses_snake_case_enums() {
        let yaml = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nconcurrency:\n  max: 5000\nsaturation_policy: reject_new\n";
        let cfg: SpectrumConfig = serde_yaml::from_str(yaml).unwrap();
        let lifetime = cfg.lifetime.as_ref().unwrap();
        assert_eq!(lifetime.distribution, LifetimeDistribution::Exponential);
        assert_eq!(cfg.saturation_policy.unwrap(), SaturationPolicy::RejectNew);
        assert!(cfg.is_valid());
    }
}
