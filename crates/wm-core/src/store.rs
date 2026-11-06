//! Swell metadata and local file storage (WM-0110).
//!
//! Phase 1 storage form follows docs/tech-stack.md ("Is a Database
//! Required?"): a CLI-driven single swell needs only the file system plus
//! JSON — no resident database. Two rules from the docs shape the layout:
//!
//! 1. Metadata storage and experiment data storage are separated
//!    (docs/tech-stack.md). The swell directory therefore holds small
//!    metadata files directly (`metadata.json`, `timeline.jsonl`) and keeps
//!    all experiment data below `evidence/`.
//! 2. Phase boundaries are written into the event timeline
//!    (docs/swell.md): every phase change rewrites `metadata.json`
//!    atomically (temp file + rename, no torn reads) and appends a JSONL
//!    timeline entry.
//!
//! Evidence paths follow the documented convention
//! `<swell_dir>/evidence/<assertion-name>/<timestamp>/`, pre-created on
//! request and never interpreted by the store (docs/adr/ADR-007,
//! docs/spec/assertion-provider.md §6). Timestamped subdirectories are
//! named `<unix-seconds>-<millis>` so lexicographic order is chronological.
//!
//! Swell ids and assertion names must be a single safe path segment
//! (non-empty, no separators, no `.`/`..`) so an id can never escape the
//! store root.

use crate::SwellPhase;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Errors raised by the local swell store.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("swell {0} already exists in the store")]
    SwellExists(String),
    #[error("swell {0} not found in the store")]
    SwellNotFound(String),
    #[error("invalid path segment {0:?}: must be a single non-empty path component without '.'/'..' parts")]
    InvalidSegment(String),
    #[error("swell metadata at {0} is unreadable: {1}")]
    Corrupt(PathBuf, #[source] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Persisted swell metadata (`<root>/<swell_id>/metadata.json`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwellMetadata {
    pub swell_id: String,
    pub name: String,
    pub phase: SwellPhase,
    #[serde(default)]
    pub spectrum_path: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

/// One event-timeline entry (`<root>/<swell_id>/timeline.jsonl`, one JSON
/// object per line).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimelineEvent {
    pub at_unix_ms: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SwellPhase>,
    pub message: String,
}

/// Local file-system store rooted at a directory; each swell gets
/// `<root>/<swell_id>/`.
#[derive(Debug, Clone)]
pub struct SwellStore {
    root: PathBuf,
}

impl SwellStore {
    /// Open (creating if needed) the store root directory.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Absolute path of a swell directory (not required to exist).
    pub fn swell_dir(&self, swell_id: &str) -> PathBuf {
        self.root.join(swell_id)
    }

    /// Create a new swell: its directory, the `evidence/` data subtree,
    /// `metadata.json` and an initial `created` timeline entry.
    pub fn create(&self, mut metadata: SwellMetadata) -> Result<SwellMetadata, StoreError> {
        validate_segment(&metadata.swell_id)?;
        let dir = self.swell_dir(&metadata.swell_id);
        fs::create_dir_all(dir.join("evidence"))?;

        let now = now_unix_ms();
        metadata.created_at_unix_ms = now;
        metadata.updated_at_unix_ms = now;

        let meta_path = dir.join("metadata.json");
        if meta_path.exists() {
            return Err(StoreError::SwellExists(metadata.swell_id.clone()));
        }
        write_atomic(&meta_path, &metadata)?;
        append_event(
            &dir.join("timeline.jsonl"),
            &TimelineEvent {
                at_unix_ms: now,
                kind: "created".into(),
                phase: Some(metadata.phase),
                message: format!("swell {} created", metadata.swell_id),
            },
        )?;
        Ok(metadata)
    }

    /// Load a swell's metadata.
    pub fn get(&self, swell_id: &str) -> Result<SwellMetadata, StoreError> {
        validate_segment(swell_id)?;
        let path = self.swell_dir(swell_id).join("metadata.json");
        if !path.is_file() {
            return Err(StoreError::SwellNotFound(swell_id.to_string()));
        }
        read_metadata(&path)
    }

    /// Load every swell's metadata, sorted by swell id.
    pub fn list(&self) -> Result<Vec<SwellMetadata>, StoreError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path().join("metadata.json");
            if path.is_file() {
                out.push(read_metadata(&path)?);
            }
        }
        out.sort_by(|a, b| a.swell_id.cmp(&b.swell_id));
        Ok(out)
    }

    /// Record a phase transition: rewrite `metadata.json` atomically and
    /// append a `phase` timeline entry (docs/swell.md phase boundaries).
    pub fn update_phase(
        &self,
        swell_id: &str,
        phase: SwellPhase,
        message: &str,
    ) -> Result<SwellMetadata, StoreError> {
        let mut metadata = self.get(swell_id)?;
        metadata.phase = phase;
        metadata.updated_at_unix_ms = now_unix_ms();
        write_atomic(&self.swell_dir(swell_id).join("metadata.json"), &metadata)?;
        append_event(
            &self.swell_dir(swell_id).join("timeline.jsonl"),
            &TimelineEvent {
                at_unix_ms: metadata.updated_at_unix_ms,
                kind: "phase".into(),
                phase: Some(phase),
                message: message.to_string(),
            },
        )?;
        Ok(metadata)
    }

    /// Read the full event timeline of a swell.
    pub fn timeline(&self, swell_id: &str) -> Result<Vec<TimelineEvent>, StoreError> {
        validate_segment(swell_id)?;
        let path = self.swell_dir(swell_id).join("timeline.jsonl");
        if !path.is_file() {
            return Err(StoreError::SwellNotFound(swell_id.to_string()));
        }
        let mut events = Vec::new();
        for line in fs::read_to_string(path)?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            events.push(serde_json::from_str(line).map_err(|e| {
                StoreError::Corrupt(self.swell_dir(swell_id).join("timeline.jsonl"), e)
            })?);
        }
        Ok(events)
    }

    /// Pre-create and return the evidence directory
    /// `<swell_dir>/evidence/<assertion_name>/<timestamp>/` for one
    /// assertion instance (ADR-007 / assertion-provider §6: the controller
    /// pre-creates, providers write, the store never interprets content).
    pub fn evidence_dir(
        &self,
        swell_id: &str,
        assertion_name: &str,
    ) -> Result<PathBuf, StoreError> {
        validate_segment(assertion_name)?;
        let dir = self
            .swell_dir(swell_id)
            .join("evidence")
            .join(assertion_name)
            .join(timestamp_name());
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// A single safe path component: non-empty, no separators, not `.` or `..`.
fn validate_segment(segment: &str) -> Result<(), StoreError> {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.contains(['/', '\\'])
        || segment.chars().any(char::is_whitespace)
    {
        return Err(StoreError::InvalidSegment(segment.to_string()));
    }
    Ok(())
}

/// Timestamped directory name for one evidence instance; lexicographic
/// order is chronological (`<unix-seconds>-<nanos>`).
fn timestamp_name() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}-{:09}", now.as_secs(), now.subsec_nanos())
}

/// Write `value` as pretty JSON via a temp file + rename so readers never
/// observe a torn `metadata.json`.
fn write_atomic<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(value).expect("metadata serializes");
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

fn read_metadata(path: &Path) -> Result<SwellMetadata, StoreError> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(|e| StoreError::Corrupt(path.to_path_buf(), e))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Append one JSON line to a `timeline.jsonl` file.
pub(crate) fn append_event(path: &Path, event: &TimelineEvent) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut line = serde_json::to_string(event).expect("timeline event serializes");
    line.push('\n');
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SwellStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SwellStore::open(dir.path().join("store")).unwrap();
        (dir, store)
    }

    fn metadata(id: &str) -> SwellMetadata {
        SwellMetadata {
            swell_id: id.to_string(),
            name: format!("{}-load", id),
            phase: SwellPhase::Planned,
            spectrum_path: Some("spectrum.yaml".into()),
            created_at_unix_ms: now_unix_ms(),
            updated_at_unix_ms: now_unix_ms(),
        }
    }

    #[test]
    fn create_then_get_roundtrips_metadata() {
        let (_d, store) = store();
        let created = store.create(metadata("swell-0001")).unwrap();
        assert_eq!(created.phase, SwellPhase::Planned);

        let got = store.get("swell-0001").unwrap();
        assert_eq!(got, created);
        assert_eq!(got.name, "swell-0001-load");
        assert_eq!(got.spectrum_path.as_deref(), Some("spectrum.yaml"));

        // Layout per tech-stack.md: metadata lives directly under the swell
        // directory, experiment data is separated below `evidence/`.
        assert!(store
            .swell_dir("swell-0001")
            .join("metadata.json")
            .is_file());
        assert!(store
            .swell_dir("swell-0001")
            .join("timeline.jsonl")
            .is_file());
        assert!(store.swell_dir("swell-0001").join("evidence").is_dir());
    }

    #[test]
    fn create_duplicate_swell_is_rejected() {
        let (_d, store) = store();
        store.create(metadata("swell-0001")).unwrap();
        let err = store.create(metadata("swell-0001")).unwrap_err();
        assert!(matches!(err, StoreError::SwellExists(id) if id == "swell-0001"));
    }

    #[test]
    fn invalid_swell_ids_are_rejected() {
        let (_d, store) = store();
        for bad in ["", "a/b", "..", ".", "../escape", "swell-0001/../x"] {
            let err = store.create(metadata(bad)).unwrap_err();
            assert!(
                matches!(err, StoreError::InvalidSegment(_)),
                "id {bad:?} must be rejected, got {err:?}"
            );
        }
        // Nothing escaped the store root.
        assert_eq!(store.list().unwrap(), Vec::<SwellMetadata>::new());
    }

    #[test]
    fn update_phase_rewrites_metadata_and_appends_timeline() {
        let (_d, store) = store();
        store.create(metadata("swell-0001")).unwrap();

        let updated = store
            .update_phase("swell-0001", SwellPhase::Running, "arrivals admitted")
            .unwrap();
        assert_eq!(updated.phase, SwellPhase::Running);

        let got = store.get("swell-0001").unwrap();
        assert_eq!(got.phase, SwellPhase::Running);
        assert!(got.created_at_unix_ms <= got.updated_at_unix_ms);

        let timeline = store.timeline("swell-0001").unwrap();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].kind, "created");
        assert_eq!(timeline[1].kind, "phase");
        assert_eq!(timeline[1].phase, Some(SwellPhase::Running));
        assert_eq!(timeline[1].message, "arrivals admitted");
    }

    #[test]
    fn timeline_and_metadata_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        {
            let store = SwellStore::open(&root).unwrap();
            store.create(metadata("swell-0002")).unwrap();
            store
                .update_phase("swell-0002", SwellPhase::Completed, "report ready")
                .unwrap();
        }
        let store = SwellStore::open(&root).unwrap();
        let got = store.get("swell-0002").unwrap();
        assert_eq!(got.phase, SwellPhase::Completed);
        let timeline = store.timeline("swell-0002").unwrap();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[1].kind, "phase");
    }

    #[test]
    fn evidence_dir_follows_documented_convention() {
        let (_d, store) = store();
        store.create(metadata("swell-0001")).unwrap();

        let first = store
            .evidence_dir("swell-0001", "no-duplicate-roles")
            .unwrap();
        let swell = store.swell_dir("swell-0001");
        let expected_parent = swell.join("evidence").join("no-duplicate-roles");
        assert!(
            first.starts_with(&expected_parent) && first.is_dir(),
            "evidence dir {first:?} must live under {expected_parent:?}"
        );

        // A second instance gets its own timestamped directory.
        let second = store
            .evidence_dir("swell-0001", "no-duplicate-roles")
            .unwrap();
        assert_ne!(first, second);
        assert!(second.is_dir());

        // Pre-created directories are listed by the store as data, but the
        // metadata files stay untouched (metadata/data separation).
        assert!(swell.join("metadata.json").is_file());
    }

    #[test]
    fn evidence_dir_rejects_unsafe_assertion_name() {
        let (_d, store) = store();
        store.create(metadata("swell-0001")).unwrap();
        for bad in ["", "a/b", "..", "."] {
            let err = store.evidence_dir("swell-0001", bad).unwrap_err();
            assert!(matches!(err, StoreError::InvalidSegment(_)));
        }
    }

    #[test]
    fn list_returns_all_swell_metadata_sorted() {
        let (_d, store) = store();
        store.create(metadata("swell-0003")).unwrap();
        store.create(metadata("swell-0001")).unwrap();
        store.create(metadata("swell-0002")).unwrap();

        let all = store.list().unwrap();
        let ids: Vec<&str> = all.iter().map(|m| m.swell_id.as_str()).collect();
        assert_eq!(ids, ["swell-0001", "swell-0002", "swell-0003"]);
    }

    #[test]
    fn get_missing_swell_is_not_found() {
        let (_d, store) = store();
        let err = store.get("swell-404").unwrap_err();
        assert!(matches!(err, StoreError::SwellNotFound(id) if id == "swell-404"));
    }

    #[test]
    fn append_event_is_recorded_in_timeline() {
        let (_d, store) = store();
        store.create(metadata("swell-0001")).unwrap();
        let swell_dir = store.swell_dir("swell-0001");
        let event = TimelineEvent {
            at_unix_ms: now_unix_ms(),
            kind: "note".into(),
            phase: None,
            message: "calibration artifact registered".into(),
        };
        let path = swell_dir.join("timeline.jsonl");
        append_event(&path, &event).unwrap();
        let timeline = store.timeline("swell-0001").unwrap();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[1], event);
    }
}
