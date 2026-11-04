//! Subcommand implementations. Phase 1: `validate` is fully wired to the
//! spectrum field checker; run/status/stop are skeletons that refuse to
//! pretend success.

use std::path::Path;
use wm_core::spectrum::{Severity, SpectrumConfig, Violation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Ok = 0,
    /// Validation found error-severity issues.
    Invalid = 1,
    /// Subcommand exists but is not wired in the Phase 1 skeleton.
    NotImplemented = 2,
}

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

pub fn run() -> ExitCode {
    eprintln!("wm run: not implemented in the Phase 1 skeleton (WM-0111 tracks the wiring)");
    ExitCode::NotImplemented
}

pub fn status() -> ExitCode {
    eprintln!("wm status: not implemented in the Phase 1 skeleton (query the controller at /api/v1/status manually)");
    ExitCode::NotImplemented
}

pub fn stop() -> ExitCode {
    eprintln!("wm stop: not implemented in the Phase 1 skeleton");
    ExitCode::NotImplemented
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
    fn skeleton_commands_decline() {
        assert_eq!(run(), ExitCode::NotImplemented);
        assert_eq!(status(), ExitCode::NotImplemented);
        assert_eq!(stop(), ExitCode::NotImplemented);
    }
}
