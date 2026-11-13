//! Test plan file face: load/validate a plan TOML, prepare spectrum
//! injections (hash pinning + concurrency override materialization).

use std::path::Path;

use sha2::{Digest, Sha256};
use wm_core::spectrum::{Severity, SpectrumConfig};
use wm_core::testplan::{parse_plan, PlanViolation, TestPlan, TestPlanError};

/// A spectrum ready for injection: the source file, its content hash (for
/// `expect_sha256` pinning and the run record), and the path actually
/// handed to the backend (an effective copy when a concurrency override
/// was materialized).
#[derive(Debug, Clone)]
pub struct PreparedSpectrum {
    /// Path actually injected (the effective copy when overridden).
    pub path: String,
    /// The plan-referenced source file.
    pub source_path: String,
    /// sha256 hex of the source file bytes.
    pub sha256: String,
    /// Effective copy path when a concurrency override was materialized.
    pub effective_path: Option<String>,
}

/// sha256 hex of a file's bytes.
pub fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// Load and validate a plan file: returns the parsed plan plus its raw text
/// and content hash (both recorded in the run directory so two runs are
/// diffable). Parse/validation failures carry the rendered violations.
pub fn load_plan(path: &Path) -> Result<(TestPlan, String, String), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let plan = parse_plan(&raw).map_err(|e| match e {
        TestPlanError::Parse(msg) => format!("cannot parse plan {}: {msg}", path.display()),
        TestPlanError::Invalid(n) => format!("invalid plan {}: {n} issue(s)", path.display()),
    })?;
    let violations = plan.validate();
    if violations.iter().any(|v| v.severity == Severity::Error) {
        return Err(format!(
            "{}\ninvalid plan {}: {} issue(s)",
            format_plan_violations(&violations),
            path.display(),
            violations.len()
        ));
    }
    let sha = file_sha256(path)?;
    Ok((plan, raw, sha))
}

/// Rewrite step `spectrum` paths that are relative into paths anchored at
/// `base` (the plan file's directory), so a plan runs identically from any
/// working directory. Absolute paths pass through untouched.
pub fn resolve_spectra(plan: &mut TestPlan, base: &Path) {
    for step in &mut plan.steps {
        if let Some(spec) = &mut step.params.spectrum {
            let p = Path::new(spec.as_str());
            if p.is_relative() {
                *spec = base.join(p).display().to_string();
            }
        }
    }
}

/// Render plan violations for terminal output (same shape as
/// `commands::format_violations`).
pub fn format_plan_violations(violations: &[PlanViolation]) -> String {
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

/// Prepare a spectrum injection: hash the source file, and when a
/// concurrency override is set, materialize an effective copy (source YAML
/// with `concurrency.target` replaced) into `run_dir` so what the backend
/// actually received stays auditable. `expect_sha256` comparison is the
/// caller's (the runner records both).
pub fn prepare_injection(
    spectrum_path: &Path,
    concurrency: Option<f64>,
    run_dir: Option<&Path>,
) -> Result<PreparedSpectrum, String> {
    let sha = file_sha256(spectrum_path)?;
    let source = spectrum_path.display().to_string();
    let Some(target) = concurrency else {
        return Ok(PreparedSpectrum {
            path: source.clone(),
            source_path: source,
            sha256: sha,
            effective_path: None,
        });
    };
    let raw =
        std::fs::read_to_string(spectrum_path).map_err(|e| format!("cannot read {source}: {e}"))?;
    let mut cfg: SpectrumConfig =
        serde_yaml::from_str(&raw).map_err(|e| format!("cannot parse spectrum {source}: {e}"))?;
    cfg.concurrency = Some(wm_core::spectrum::ConcurrencySpec {
        target: Some(target),
        max: cfg.concurrency.as_ref().and_then(|c| c.max),
    });
    let effective = serde_yaml::to_string(&cfg)
        .map_err(|e| format!("cannot render effective spectrum: {e}"))?;
    let dir = run_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let dest = dir.join("spectrum.effective.yaml");
    std::fs::write(&dest, &effective)
        .map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    Ok(PreparedSpectrum {
        path: dest.display().to_string(),
        source_path: source,
        sha256: sha,
        effective_path: Some(dest.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    const GOOD: &str =
        "arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nsaturation_policy: queue\n";

    #[test]
    fn load_plan_returns_raw_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = write_file(
            dir.path(),
            "smoke.toml",
            "[meta]\nname = \"smoke\"\n\n[[steps]]\nid = \"a\"\naction = \"report\"\n",
        );
        let (plan, raw, sha) = load_plan(&plan_path).unwrap();
        assert_eq!(plan.meta.name, "smoke");
        assert!(raw.contains("[[steps]]"));
        assert_eq!(sha.len(), 64);
    }

    #[test]
    fn resolve_spectra_anchors_relative_paths_at_the_plan_dir() {
        // A genuinely absolute path for the platform (backslashes would be
        // TOML escapes, so the temp dir goes in with forward slashes).
        let abs = std::env::temp_dir()
            .display()
            .to_string()
            .replace('\\', "/");
        let mut plan = parse_plan(&format!(
            r#"
[meta]
name = "demo"

[[steps]]
id = "load"
action = "inject_load"
params = {{ spectrum = "../spectra/demo.yaml", duration_secs = 60 }}

[[steps]]
id = "pin"
action = "inject_load"
params = {{ spectrum = "{abs}/kept.yaml", duration_secs = 60 }}
"#
        ))
        .unwrap();
        resolve_spectra(&mut plan, Path::new("examples/plans"));
        let specs: Vec<&str> = plan
            .steps
            .iter()
            .filter_map(|s| s.params.spectrum.as_deref())
            .collect();
        // Compare as paths: the joined rendering uses the platform
        // separator (backslash on Windows), the `..` segment stays.
        let expected = Path::new("examples/plans").join("../spectra/demo.yaml");
        assert_eq!(Path::new(specs[0]), expected);
        assert_eq!(
            specs[1],
            format!("{abs}/kept.yaml"),
            "absolute path untouched"
        );
    }

    #[test]
    fn load_plan_reports_violations_not_parse_only() {
        let dir = tempfile::tempdir().unwrap();
        // Parses fine, but steps is empty: an Error-severity violation.
        // (steps must sit at the document top level, before [meta].)
        let plan_path = write_file(
            dir.path(),
            "empty.toml",
            "steps = []\n\n[meta]\nname = \"x\"\n",
        );
        let err = load_plan(&plan_path).unwrap_err();
        assert!(err.contains("invalid plan"), "{err}");
        assert!(err.contains("1 issue(s)"), "{err}");
    }

    #[test]
    fn load_plan_missing_file_is_named() {
        let err = load_plan(Path::new("/nonexistent/plan.toml")).unwrap_err();
        assert!(err.contains("cannot read"), "{err}");
    }

    #[test]
    fn format_plan_violations_mirrors_spectrum_shape() {
        let plan = parse_plan("[meta]\nname = \"x\"\n\n[[steps]]\nid = \"a\"\naction = \"hold\"\n")
            .unwrap();
        let text = format_plan_violations(&plan.validate());
        assert!(text.contains("ERROR E_SECS_REQUIRED"), "{text}");
        assert!(text.contains("steps[id=a].params.secs"), "{text}");
    }

    #[test]
    fn prepare_injection_materializes_concurrency_override() {
        let dir = tempfile::tempdir().unwrap();
        let run_dir = tempfile::tempdir().unwrap();
        let spec = write_file(dir.path(), "spec.yaml", GOOD);
        let prepared = prepare_injection(&spec, Some(42.0), Some(run_dir.path())).unwrap();
        assert_eq!(prepared.sha256.len(), 64);
        assert_eq!(prepared.source_path, spec.display().to_string());
        let effective = prepared.effective_path.unwrap();
        let text = std::fs::read_to_string(&effective).unwrap();
        assert!(text.contains("target: 42"), "{text}");
        // The injected path IS the effective copy.
        assert_eq!(prepared.path, effective);
    }

    #[test]
    fn prepare_injection_without_override_injects_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let spec = write_file(dir.path(), "spec.yaml", GOOD);
        let prepared = prepare_injection(&spec, None, None).unwrap();
        assert!(prepared.effective_path.is_none());
        assert_eq!(prepared.path, spec.display().to_string());
    }

    #[test]
    fn file_sha256_is_stable_hex() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("x")).unwrap();
        f.write_all(b"hello").unwrap();
        let sha = file_sha256(&dir.path().join("x")).unwrap();
        // sha256("hello")
        assert_eq!(
            sha,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
