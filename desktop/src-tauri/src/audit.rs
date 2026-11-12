//! Audit trail for permission-matrix actions (WM-0608).
//!
//! Records WHO (session username) / WHEN (epoch ms) / WHAT (an
//! `authz::Action` from ADR-009) for every gated shell command — both
//! allowed executions and denied attempts. Storage is an append-only
//! JSONL file in the app data directory: the only write path is
//! `append_entry`, no edit or delete API exists, and the reader never
//! rewrites — immutability is by construction, not by policy.
//!
//! Scope (honest, per ADR-009's reuse decision): entries are the seven
//! matrix actions. The controller operations `start_swell`, `stop_swell`,
//! `clean_up_bots` and `apply_plan` are not wired through the desktop
//! shell yet (ADR-008 keeps the controller single-operator), so nothing
//! can record them here today — when they are wired, they pass through
//! the same `users::authorize` chokepoint and land in this trail. Login/
//! logout are session events, not matrix actions, and are not recorded.
//!
//! Recording is fail-closed: if the trail cannot be written (disk full,
//! corrupt file), the gated command fails with a named error rather than
//! executing unaudited (`docs/security.md`: high-risk actions must
//! record the operator).

use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::authz::Action;
use crate::users::{app_data_dir, authorize, AuthzState};

/// One process-wide write lock: seq assignment and appends are serialized
/// so two commands cannot claim the same sequence number.
static AUDIT_LOCK: Mutex<()> = Mutex::new(());

/// One audit entry. `username` is `None` for a denied attempt with no
/// session (`not_authenticated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Strictly increasing, assigned by `record` under the write lock.
    pub seq: u64,
    /// Unix epoch milliseconds (when the attempt was judged).
    pub timestamp_ms: u64,
    pub username: Option<String>,
    pub action: Action,
    pub outcome: AuditOutcome,
    /// Free-text context supplied by the command ("save spectrum
    /// examples/spec.yaml", "create user carol") — never credentials.
    pub detail: String,
}

/// Whether the gate allowed or denied the attempt. A denied attempt is
/// itself an audit event (`docs/security.md`), not an error path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Allowed,
    Denied,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Allowed => "allowed",
            AuditOutcome::Denied => "denied",
        }
    }
}

/// Errors from the trail file — all named, none silent.
#[derive(Debug, PartialEq)]
pub enum AuditError {
    /// I/O failure reading or appending `audit.jsonl`.
    Io(String),
    /// A line exists that is not a valid entry — never skipped silently,
    /// because a gap would fake a complete trail.
    Corrupt { line: usize },
    /// A filter named an action or outcome that does not exist.
    BadFilter(String),
}

/// Read the whole trail in file (== sequence) order. A missing file is an
/// honest "no events yet"; an unparseable line is a named error.
pub fn load_entries(path: &Path) -> Result<Vec<AuditEntry>, AuditError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(AuditError::Io(err.to_string())),
    };
    let mut entries = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry = serde_json::from_str(line)
            .map_err(|_| AuditError::Corrupt { line: idx + 1 })?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Append one entry as a single JSON line. The entry is written verbatim
/// — no caller can rewrite earlier lines through this API.
pub fn append_entry(path: &Path, entry: &AuditEntry) -> Result<(), AuditError> {
    let line = serde_json::to_string(entry).map_err(|e| AuditError::Io(e.to_string()))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| AuditError::Io(e.to_string()))?;
    writeln!(file, "{line}").map_err(|e| AuditError::Io(e.to_string()))?;
    Ok(())
}

/// The filter passed from the webview. Every field is optional; an
/// absent field never filters. Unknown action/outcome strings are a
/// named error rather than a silently-empty result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFilter {
    pub username: Option<String>,
    pub action: Option<String>,
    pub outcome: Option<String>,
    /// Inclusive epoch-ms lower bound.
    pub from_ms: Option<u64>,
    /// Inclusive epoch-ms upper bound.
    pub to_ms: Option<u64>,
    /// Only entries with `seq > since_seq` (polling without re-reading).
    pub since_seq: Option<u64>,
}

/// Query the trail: filter, keep sequence order. Pure and testable.
pub fn filter_entries(entries: &[AuditEntry], filter: &AuditFilter) -> Result<Vec<AuditEntry>, AuditError> {
    let action = match &filter.action {
        None => None,
        Some(text) => Some(parse_action(text).ok_or_else(|| AuditError::BadFilter(text.clone()))?),
    };
    let outcome = match &filter.outcome {
        None => None,
        Some(text) => Some(parse_outcome(text).ok_or_else(|| AuditError::BadFilter(text.clone()))?),
    };
    let matched = entries
        .iter()
        .filter(|entry| {
            filter.username.as_deref().map(|u| entry.username.as_deref() == Some(u)).unwrap_or(true)
                && action.map(|a| entry.action == a).unwrap_or(true)
                && outcome.map(|o| entry.outcome == o).unwrap_or(true)
                && filter.from_ms.map(|from| entry.timestamp_ms >= from).unwrap_or(true)
                && filter.to_ms.map(|to| entry.timestamp_ms <= to).unwrap_or(true)
                && filter.since_seq.map(|since| entry.seq > since).unwrap_or(true)
        })
        .cloned()
        .collect();
    Ok(matched)
}

/// Record one judged attempt: assign the next sequence number under the
/// write lock and append. Fail-closed — the caller (the authorize
/// chokepoint) turns an error into a named command failure.
pub fn record(
    path: &Path,
    action: Action,
    allowed: bool,
    username: Option<String>,
    detail: &str,
) -> Result<(), String> {
    let _guard = AUDIT_LOCK.lock().map_err(|_| "audit_lock_poisoned".to_string())?;
    let entries = load_entries(path).map_err(|e| format!("audit_read_failed: {e:?}"))?;
    let seq = entries.last().map(|e| e.seq + 1).unwrap_or(1);
    let entry = AuditEntry {
        seq,
        timestamp_ms: now_ms(),
        username,
        action,
        outcome: if allowed { AuditOutcome::Allowed } else { AuditOutcome::Denied },
        detail: detail.to_string(),
    };
    append_entry(path, &entry).map_err(|e| format!("audit_write_failed: {e:?}"))
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn parse_action(text: &str) -> Option<Action> {
    crate::authz::permission_matrix()
        .iter()
        .map(|entry| entry.action)
        .find(|action| action.as_str() == text)
}

fn parse_outcome(text: &str) -> Option<AuditOutcome> {
    match text {
        "allowed" => Some(AuditOutcome::Allowed),
        "denied" => Some(AuditOutcome::Denied),
        _ => None,
    }
}

/// One trail row for the webview: action/outcome as the strings the
/// engine's matrix uses, everything verbatim.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditRow {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub username: Option<String>,
    pub action: String,
    pub outcome: String,
    pub detail: String,
}

impl From<&AuditEntry> for AuditRow {
    fn from(entry: &AuditEntry) -> Self {
        AuditRow {
            seq: entry.seq,
            timestamp_ms: entry.timestamp_ms,
            username: entry.username.clone(),
            action: entry.action.as_str().to_string(),
            outcome: entry.outcome.as_str().to_string(),
            detail: entry.detail.clone(),
        }
    }
}

/// Query the audit trail. Every logged-in role may read it — it is a
/// `view` surface (ADR-009), stored locally next to the users file,
/// which the machine operator can read anyway. The query itself is an
/// authorized attempt and is recorded like any other, so the trail also
/// answers "who looked at the trail".
#[tauri::command]
pub fn query_audit(
    app: AppHandle,
    state: State<'_, AuthzState>,
    filter: Option<AuditFilter>,
) -> Result<Vec<AuditRow>, String> {
    authorize(&app, &state, Action::View, "query audit log")?;
    let filter = filter.unwrap_or_default();
    let path = app_data_dir(&app)?.join("audit.jsonl");
    let entries = load_entries(&path).map_err(|e| format!("cannot read audit trail: {e:?}"))?;
    let matched = filter_entries(&entries, &filter).map_err(|e| format!("bad audit filter: {e:?}"))?;
    Ok(matched.iter().map(AuditRow::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entry(seq: u64, username: Option<&str>, action: Action, allowed: bool, ts: u64) -> AuditEntry {
        AuditEntry {
            seq,
            timestamp_ms: ts,
            username: username.map(String::from),
            action,
            outcome: if allowed { AuditOutcome::Allowed } else { AuditOutcome::Denied },
            detail: format!("entry {seq}"),
        }
    }

    /// Immutability by construction: appending grows the file and every
    /// earlier byte stays exactly where it was. No rewrite ever happens.
    #[test]
    fn append_is_append_only_and_earlier_bytes_stay_intact() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        append_entry(&path, &entry(1, Some("ada"), Action::EditConfig, true, 100)).unwrap();
        let before = std::fs::read(&path).unwrap();
        append_entry(&path, &entry(2, Some("ada"), Action::ManageUsers, true, 200)).unwrap();
        let after = std::fs::read(&path).unwrap();
        assert!(after.len() > before.len());
        assert_eq!(&after[..before.len()], &before[..]);
        let loaded = load_entries(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], entry(1, Some("ada"), Action::EditConfig, true, 100));
        assert_eq!(loaded[1], entry(2, Some("ada"), Action::ManageUsers, true, 200));
    }

    /// Recording assigns strictly increasing sequence numbers — a trail
    /// with gaps or duplicates cannot be trusted.
    #[test]
    fn record_assigns_strictly_increasing_seqs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        for i in 1..=5 {
            record(&path, Action::EditConfig, true, Some("ada".into()), "save").unwrap();
            let _ = i;
        }
        let loaded = load_entries(&path).unwrap();
        let seqs: Vec<u64> = loaded.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn load_missing_file_is_empty_not_corrupt() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        assert_eq!(load_entries(&path).unwrap(), Vec::new());
    }

    /// A damaged line is a named error, never skipped: a silently
    /// shortened trail would fake completeness.
    #[test]
    fn corrupt_line_is_a_named_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let good = serde_json::to_string(&entry(1, Some("ada"), Action::EditConfig, true, 100)).unwrap();
        std::fs::write(&path, format!("{good}\n{{ not json\n")).unwrap();
        let err = load_entries(&path).unwrap_err();
        assert_eq!(err, AuditError::Corrupt { line: 2 });
    }

    #[test]
    fn filter_by_username_and_action() {
        let entries = vec![
            entry(1, Some("ada"), Action::EditConfig, true, 100),
            entry(2, Some("carol"), Action::EditConfig, false, 200),
            entry(3, Some("ada"), Action::ManageUsers, true, 300),
        ];
        let by_user = filter_entries(
            &entries,
            &AuditFilter { username: Some("ada".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(by_user.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 3]);
        let by_action = filter_entries(
            &entries,
            &AuditFilter { action: Some("manage_users".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(by_action.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn filter_by_outcome_and_time_range_and_since_seq() {
        let entries = vec![
            entry(1, Some("ada"), Action::EditConfig, true, 100),
            entry(2, Some("ada"), Action::EditConfig, false, 200),
            entry(3, Some("ada"), Action::EditConfig, true, 300),
            entry(4, Some("ada"), Action::EditConfig, true, 400),
        ];
        let denied = filter_entries(
            &entries,
            &AuditFilter { outcome: Some("denied".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(denied.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2]);
        let window = filter_entries(
            &entries,
            &AuditFilter { from_ms: Some(200), to_ms: Some(300), ..Default::default() },
        )
        .unwrap();
        assert_eq!(window.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
        let since = filter_entries(
            &entries,
            &AuditFilter { since_seq: Some(2), ..Default::default() },
        )
        .unwrap();
        assert_eq!(since.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3, 4]);
    }

    /// Filters combine (AND); an unknown action string is a named error,
    /// never a silently-empty result.
    #[test]
    fn filter_combines_and_names_unknown_action() {
        let entries = vec![
            entry(1, Some("ada"), Action::EditConfig, true, 100),
            entry(2, Some("ada"), Action::ManageUsers, false, 200),
            entry(3, Some("carol"), Action::EditConfig, true, 300),
        ];
        let both = filter_entries(
            &entries,
            &AuditFilter {
                username: Some("ada".into()),
                action: Some("edit_config".into()),
                outcome: Some("allowed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(both.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1]);
        assert_eq!(
            filter_entries(
                &entries,
                &AuditFilter { action: Some("reboot".into()), ..Default::default() },
            )
            .unwrap_err(),
            AuditError::BadFilter("reboot".into())
        );
    }
}
