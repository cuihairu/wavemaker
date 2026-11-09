//! Failure evidence archiving (WM-0704; docs/spec/assertion-provider.md
//! §6).
//!
//! Rules this module encodes:
//! - path layout is frozen: `<swell_dir>/evidence/<assertion-name>/<timestamp>/`,
//!   pre-created by the Controller at registration (§6.2) and injected
//!   into the provider's window context (see
//!   [`AssertionScheduler::attach_evidence_dir`]);
//! - the Controller only ARCHIVES: raw evidence is never converted,
//!   sampled, or summarized — Wavemaker cannot judge what inside a
//!   business-evidence bundle is redundant (§6.4), so this module touches
//!   directory names only, never file contents;
//! - archiving is best-effort but MUST be visible (§6.3): a directory
//!   that failed to materialize is reported as a named failure, and the
//!   report writer keeps the evidence references with the degradation
//!   attached — nothing is silently dropped;
//! - `kind: artifact` evidence references become §14 `raw_data_references`
//!   entries with format `evidence-bundle` (§6.1).

use std::fs;
use std::path::{Path, PathBuf};

use crate::assertions::{AssertionRecord, EvidenceKind};

/// Format label for §14 `raw_data_references` entries that point at
/// assertion evidence bundles (§6.1).
pub const EVIDENCE_BUNDLE_FORMAT: &str = "evidence-bundle";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    #[error("evidence path component {0:?} must not be empty")]
    EmptyComponent(String),
    #[error("evidence path component {0:?} must not contain path separators or traversal")]
    UnsafeComponent(String),
    #[error("evidence directory {0} could not be created: {1}")]
    CreateFailed(String, String),
}

fn checked_component(component: &str) -> Result<&str, EvidenceError> {
    if component.trim().is_empty() {
        return Err(EvidenceError::EmptyComponent(component.into()));
    }
    if component.contains('/') || component.contains('\\') || component == ".." || component == "." {
        return Err(EvidenceError::UnsafeComponent(component.into()));
    }
    Ok(component)
}

/// One §14 `raw_data_references` entry (schema-shaped: `path` +
/// `format`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawDataReference {
    pub path: String,
    pub format: String,
}

/// The outcome of one archive attempt — success and failure look the
/// same shape so a report writer cannot lose either.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArchiveStatus {
    /// The evidence directory as injected into the window context.
    pub dir: String,
    pub exists: bool,
    /// Entry names inside the directory, verbatim and unsorted beyond
    /// the filesystem's order — contents are never read (§6.4).
    pub entries: Vec<String>,
    /// Present when the archive attempt failed; travels with the
    /// references into the report so the degradation is visible (§6.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The Controller-side archive for one swell.
#[derive(Debug, Clone)]
pub struct EvidenceArchive {
    swell_dir: PathBuf,
}

impl EvidenceArchive {
    pub fn new(swell_dir: impl Into<PathBuf>) -> Self {
        Self {
            swell_dir: swell_dir.into(),
        }
    }

    pub fn swell_dir(&self) -> &Path {
        &self.swell_dir
    }

    /// §6.2: pre-create `<swell_dir>/evidence/<name>/<timestamp>/` when
    /// the assertion instance is registered. Idempotent for repeat
    /// registrations of the same instance. The returned path (relative
    /// to the swell dir) is what rides in the window context.
    pub fn prepare(
        &self,
        assertion_name: &str,
        timestamp: &str,
    ) -> Result<String, EvidenceError> {
        let name = checked_component(assertion_name)?;
        let stamp = checked_component(timestamp)?;
        let dir = self
            .swell_dir
            .join("evidence")
            .join(name)
            .join(stamp);
        fs::create_dir_all(&dir).map_err(|err| {
            EvidenceError::CreateFailed(dir.display().to_string(), err.to_string())
        })?;
        Ok(relative_evidence_path(name, stamp))
    }

    /// §6.3: the visibility check a report writer runs at wrap-up — the
    /// directory either exists (with its entry names) or the failure is
    /// returned as a named error. Best-effort: neither outcome throws
    /// away the references.
    pub fn archive_status(&self, evidence_dir: &str) -> ArchiveStatus {
        let full = self.swell_dir.join(evidence_dir);
        match fs::read_dir(&full) {
            Ok(entries) => ArchiveStatus {
                dir: evidence_dir.to_string(),
                exists: true,
                entries: entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect(),
                error: None,
            },
            Err(error) => ArchiveStatus {
                dir: evidence_dir.to_string(),
                exists: false,
                entries: Vec::new(),
                error: Some(format!(
                    "evidence directory {} not archived: {error}",
                    full.display()
                )),
            },
        }
    }
}

/// §6.1: `kind: artifact` evidence references from the assertion
/// records become §14 `raw_data_references` entries (format
/// `evidence-bundle`), in record order — the report never reorders or
/// reinterprets them. Pure mapping; it needs no archive access, so it
/// lives as a free function.
pub fn raw_data_references(records: &[AssertionRecord]) -> Vec<RawDataReference> {
    let mut references = Vec::new();
    for record in records {
        for evidence in &record.evidence {
            if evidence.kind == EvidenceKind::Artifact {
                references.push(RawDataReference {
                    path: evidence.reference.clone(),
                    format: EVIDENCE_BUNDLE_FORMAT.into(),
                });
            }
        }
    }
    references
}

/// The relative path injected into window contexts, deterministic across
/// platforms for the same components.
fn relative_evidence_path(name: &str, stamp: &str) -> String {
    format!("evidence/{name}/{stamp}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::{
        AssertionStatus, EvidenceKind, EvidenceRef, RecordWindow, Trigger,
    };

    fn record_with_evidence(
        name: &str,
        evidence: Vec<EvidenceRef>,
    ) -> AssertionRecord {
        AssertionRecord {
            provider: "game-admin-api".into(),
            name: name.into(),
            status: AssertionStatus::Fail,
            checked_at: "2026-11-10T00:00:00Z".into(),
            window: Some(RecordWindow {
                start: "2026-11-10T00:00:00Z".into(),
                end: "2026-11-10T00:05:00Z".into(),
            }),
            trigger: Some(Trigger::AfterScenario),
            message: Some("3 duplicate role pairs".into()),
            evidence,
        }
    }

    #[test]
    fn prepare_creates_the_layout_and_returns_the_injection_path() {
        let swell = tempfile::tempdir().expect("tempdir");
        let archive = EvidenceArchive::new(swell.path());
        let dir = archive
            .prepare("no-duplicate-roles", "20261110T000000Z")
            .expect("prepare");
        assert_eq!(dir, "evidence/no-duplicate-roles/20261110T000000Z");
        let full = swell.path().join(&dir);
        assert!(full.is_dir(), "directory pre-created");
        // Idempotent.
        let again = archive
            .prepare("no-duplicate-roles", "20261110T000000Z")
            .expect("prepare again");
        assert_eq!(again, dir);
    }

    #[test]
    fn prepare_refuses_unsafe_components() {
        let swell = tempfile::tempdir().expect("tempdir");
        let archive = EvidenceArchive::new(swell.path());
        assert_eq!(
            archive.prepare("", "stamp"),
            Err(EvidenceError::EmptyComponent("".into()))
        );
        assert_eq!(
            archive.prepare("name", "  "),
            Err(EvidenceError::EmptyComponent("  ".into()))
        );
        assert!(matches!(
            archive.prepare("../escape", "stamp"),
            Err(EvidenceError::UnsafeComponent(_))
        ));
        assert!(matches!(
            archive.prepare("name", "a/b"),
            Err(EvidenceError::UnsafeComponent(_))
        ));
        assert!(matches!(
            archive.prepare("name", "."),
            Err(EvidenceError::UnsafeComponent(_))
        ));
    }

    #[test]
    fn artifact_references_become_evidence_bundle_entries() {
        let records = vec![
            record_with_evidence(
                "no-duplicate-roles",
                vec![
                    EvidenceRef {
                        kind: EvidenceKind::Artifact,
                        reference:
                            "evidence/no-duplicate-roles/20261110T000000Z/roles-diff.json"
                                .into(),
                    },
                    EvidenceRef {
                        kind: EvidenceKind::Query,
                        reference: "SELECT count(*) FROM roles GROUP BY account".into(),
                    },
                ],
            ),
            record_with_evidence(
                "reward-consistency",
                vec![EvidenceRef {
                    kind: EvidenceKind::Artifact,
                    reference:
                        "evidence/reward-consistency/20261110T000100Z/settlement.json"
                            .into(),
                }],
            ),
            // A query-only record contributes nothing to §14.
            record_with_evidence(
                "online-count",
                vec![EvidenceRef {
                    kind: EvidenceKind::Query,
                    reference: "SELECT 1".into(),
                }],
            ),
        ];
        let references = raw_data_references(&records);
        assert_eq!(
            references,
            vec![
                RawDataReference {
                    path:
                        "evidence/no-duplicate-roles/20261110T000000Z/roles-diff.json"
                            .into(),
                    format: "evidence-bundle".into(),
                },
                RawDataReference {
                    path:
                        "evidence/reward-consistency/20261110T000100Z/settlement.json"
                            .into(),
                    format: "evidence-bundle".into(),
                },
            ]
        );
    }

    #[test]
    fn archive_status_lists_entries_without_reading_contents() {
        let swell = tempfile::tempdir().expect("tempdir");
        let archive = EvidenceArchive::new(swell.path());
        let dir = archive.prepare("no-duplicate-roles", "20261110T000000Z").expect("prepare");
        let full = swell.path().join(&dir);
        // Raw evidence is arbitrary bytes; the archive never opens it.
        std::fs::write(full.join("roles-diff.json"), [0u8, 159, 146, 150]).expect("write");
        std::fs::write(full.join("provider.log"), b"raw text").expect("write");

        let status = archive.archive_status(&dir);
        assert!(status.exists);
        assert!(status.error.is_none());
        let mut names = status.entries.clone();
        names.sort();
        assert_eq!(names, vec!["provider.log", "roles-diff.json"]);
    }

    #[test]
    fn missing_evidence_directory_is_a_named_visible_failure() {
        let swell = tempfile::tempdir().expect("tempdir");
        let archive = EvidenceArchive::new(swell.path());
        let status = archive.archive_status("evidence/never-created/20261110T000000Z");
        assert!(!status.exists);
        assert!(status.entries.is_empty());
        let error = status.error.expect("named failure");
        assert!(error.contains("not archived"));
    }

    #[test]
    fn create_failure_carries_the_reason() {
        // A FILE where the directory belongs makes create_dir_all fail.
        let swell = tempfile::tempdir().expect("tempdir");
        std::fs::write(swell.path().join("evidence"), b"occupied").expect("write");
        let archive = EvidenceArchive::new(swell.path());
        let prepared = archive.prepare("no-duplicate-roles", "20261110T000000Z");
        assert!(matches!(
            prepared,
            Err(EvidenceError::CreateFailed(_, _))
        ));
    }
}
