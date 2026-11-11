//! Spectrum editing commands (WM-0603).
//!
//! Validation reuses wm-core's `SpectrumConfig::validate` — the same
//! engine behind `wm validate` — so the desktop report can never drift
//! from the CLI. YAML parse failures are surfaced as a `parse_error`
//! string; field validation only runs on a parseable config (same
//! ordering as `wm validate`).

use serde::Serialize;
use wm_core::spectrum::{Severity, SpectrumConfig};

/// One explainable validation issue, serialized for the webview.
#[derive(Debug, PartialEq, Serialize)]
pub struct ValidationIssue {
    /// "error" or "warning".
    pub severity: &'static str,
    /// Stable machine-readable code, e.g. `E_ARRIVAL_MISSING`.
    pub code: String,
    /// Field path the issue refers to, e.g. `arrival.ramp.from_per_second`.
    pub field: String,
    pub message: String,
}

/// Result of validating one spectrum document.
#[derive(Debug, PartialEq, Serialize)]
pub struct ValidationReport {
    /// True when the config parsed and no error-severity issue remains.
    pub ok: bool,
    /// YAML parse failure; field validation is skipped when set.
    pub parse_error: Option<String>,
    pub issues: Vec<ValidationIssue>,
}

#[tauri::command]
pub fn validate_spectrum(content: String) -> ValidationReport {
    let cfg: SpectrumConfig = match serde_yaml::from_str(&content) {
        Ok(cfg) => cfg,
        Err(err) => {
            return ValidationReport {
                ok: false,
                parse_error: Some(err.to_string()),
                issues: Vec::new(),
            }
        }
    };
    let issues = cfg
        .validate()
        .into_iter()
        .map(|v| ValidationIssue {
            severity: match v.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            code: v.code.to_string(),
            field: v.field.to_string(),
            message: v.message,
        })
        .collect();
    ValidationReport {
        ok: cfg.is_valid(),
        parse_error: None,
        issues,
    }
}

#[tauri::command]
pub fn load_spectrum_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))
}

#[tauri::command]
pub fn save_spectrum_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content.as_bytes()).map_err(|e| format!("cannot write {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: &str = "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: fixed\n  mean_secs: 900\nconcurrency:\n  target: 5000\n  max: 5000\nsaturation_policy: queue\n";

    #[test]
    fn reference_scenario_is_ok_with_the_naive_deviation_warning() {
        let report = validate_spectrum(REFERENCE.into());
        assert!(report.ok);
        assert!(report.parse_error.is_none());
        assert!(report
            .issues
            .iter()
            .any(|i| i.severity == "warning" && i.code == "W_TARGET_NAIVE_LITTLE_DEVIATION"));
    }

    #[test]
    fn missing_arrival_maps_to_error_issue() {
        let report = validate_spectrum("lifetime:\n  distribution: fixed\n  mean_secs: 900\n".into());
        assert!(!report.ok);
        assert!(report
            .issues
            .iter()
            .any(|i| i.severity == "error" && i.code == "E_ARRIVAL_MISSING" && i.field == "arrival"));
    }

    #[test]
    fn parse_failure_is_reported_not_panicked() {
        let report = validate_spectrum("arrival: [unclosed".into());
        assert!(!report.ok);
        assert!(report.parse_error.is_some());
        assert!(report.issues.is_empty());
    }

    #[test]
    fn empty_document_reports_missing_fields() {
        let report = validate_spectrum(String::new());
        assert!(!report.ok);
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "E_ARRIVAL_MISSING"));
    }

    #[test]
    fn load_and_save_roundtrip_through_the_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spectrum.yaml");
        let path_str = path.display().to_string();

        save_spectrum_file(path_str.clone(), REFERENCE.into()).unwrap();
        assert_eq!(load_spectrum_file(path_str).unwrap(), REFERENCE);
    }

    #[test]
    fn load_missing_file_is_a_named_error() {
        let err = load_spectrum_file("/nonexistent/spectrum.yaml".into()).unwrap_err();
        assert!(err.starts_with("cannot read /nonexistent/spectrum.yaml"));
    }
}
