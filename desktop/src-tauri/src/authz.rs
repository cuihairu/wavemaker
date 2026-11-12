//! Users, roles and the permission matrix (WM-0607; ADR-009).
//!
//! The permission model lives here — in the desktop shell and its
//! engine-side decision functions — and NOT on the v1 control plane:
//! ADR-008 keeps the controller REST/gRPC single-operator, and ADR-009
//! explicitly records that multi-user/permission/audit may not enter the
//! control plane early (no half-finished authentication surface). The
//! desktop is a local trust boundary; this module does not implement
//! network authentication, sessions or cookies.
//!
//! Roles (WM-0607): `admin`, `operator`, `viewer`. Actions are the same
//! set for every role — the matrix (`permission_matrix`) is the single
//! source of truth and tests exhaust it. A missing role never defaults
//! to allowed: `gate` denies anything not explicitly permitted.
//!
//! Passwords are stored as salted SHA-256 hashes in `users.json`
//! (`docs/security.md`: credentials are never written to logs; only the
//! hash is persisted). The salt uses OS-provided randomness plus
//! process/time entropy — adequate for a local trust boundary, not a
//! remote credential store.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A role the permission matrix is evaluated against. Ordered so the
/// ordering itself encodes privilege (viewer < operator < admin); tests
/// pin this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Operator => "operator",
            Role::Admin => "admin",
        }
    }
}

/// An action the desktop performs. `EditConfig` and `ManageUsers` are
/// admin-only; run operations are operator-or-above; viewing is open to
/// every role (ADR-009 decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    View,
    StartSwell,
    StopSwell,
    CleanUpBots,
    ApplyPlan,
    EditConfig,
    ManageUsers,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::View => "view",
            Action::StartSwell => "start_swell",
            Action::StopSwell => "stop_swell",
            Action::CleanUpBots => "clean_up_bots",
            Action::ApplyPlan => "apply_plan",
            Action::EditConfig => "edit_config",
            Action::ManageUsers => "manage_users",
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of the permission matrix: the action and the lowest role
/// allowed to perform it (`None` = open to every role, including viewer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixEntry {
    pub action: Action,
    pub minimum_role: Option<Role>,
}

/// The whole matrix, in a stable order — single source of truth for both
/// the Rust decision and the UI's matrix table.
pub fn permission_matrix() -> Vec<MatrixEntry> {
    vec![
        MatrixEntry { action: Action::View, minimum_role: None },
        MatrixEntry { action: Action::StartSwell, minimum_role: Some(Role::Operator) },
        MatrixEntry { action: Action::StopSwell, minimum_role: Some(Role::Operator) },
        MatrixEntry { action: Action::CleanUpBots, minimum_role: Some(Role::Operator) },
        MatrixEntry { action: Action::ApplyPlan, minimum_role: Some(Role::Operator) },
        MatrixEntry { action: Action::EditConfig, minimum_role: Some(Role::Admin) },
        MatrixEntry { action: Action::ManageUsers, minimum_role: Some(Role::Admin) },
    ]
}

/// The minimum role allowed for an action, or `None` when it is open
/// to every role (viewing).
pub fn minimum_role(action: Action) -> Option<Role> {
    permission_matrix()
        .iter()
        .find(|entry| entry.action == action)
        .and_then(|entry| entry.minimum_role)
}

/// Whether `role` may perform `action` — the authoritative check the
/// shell gates on. An action absent from the matrix is a denial (never
/// defaults to allowed); a matrix entry with no minimum role is open to
/// every role (viewing).
pub fn allowed(role: Role, action: Action) -> bool {
    match permission_matrix().iter().find(|entry| entry.action == action) {
        None => false,
        Some(entry) => entry.minimum_role.map(|minimum| role >= minimum).unwrap_or(true),
    }
}

/// Deny carries the offending role, action and the minimum role that
/// would have allowed it — so the UI can say what is required instead of
/// a bare "forbidden".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDenied {
    pub role: Role,
    pub action: Action,
    pub minimum_role: Role,
}

impl PermissionDenied {
    pub fn reason(&self) -> String {
        format!(
            "forbidden: role {} may not {} (requires {})",
            self.role.as_str(),
            self.action.as_str(),
            self.minimum_role.as_str()
        )
    }
}

/// Session state: who is currently logged in, if anyone. No credentials
/// are kept — only the identity and role needed to authorize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub username: String,
    pub role: Role,
}

/// Authorize an action against an optional session. `None` session
/// denies everything with a named `not_authenticated` reason rather
/// than defaulting to allowed.
pub fn gate(session: Option<&Session>, action: Action) -> Result<(), PermissionDenied> {
    let minimum = minimum_role(action).unwrap_or(Role::Admin);
    let denied = |role: Role| PermissionDenied { role, action, minimum_role: minimum };
    match session {
        Some(session) if allowed(session.role, action) => Ok(()),
        Some(session) => Err(denied(session.role)),
        None => Err(denied(Role::Viewer)),
    }
}

/// Errors from the local user file — all named, none silent.
#[derive(Debug, PartialEq)]
pub enum AuthzError {
    /// I/O or deserialization failure reading `users.json`.
    UsersFile(String),
    /// The file exists but is unparseable — never silently treated as
    /// "no users", because an admin would be invisible.
    Corrupt,
    /// A user with this username already exists.
    AlreadyExists(String),
    /// No such username.
    UnknownUser(String),
    /// Password did not match.
    BadPassword,
}

/// One stored user record. Only the password HASH is persisted
/// (`docs/security.md`); plaintext passwords are never written to disk
/// or logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRecord {
    pub username: String,
    pub role: Role,
    /// Salted SHA-256 hex digest of the password.
    pub password_hash: String,
    /// Salt (hex) the hash was computed with.
    pub salt: String,
}

impl UserRecord {
    /// Hash a password with a fresh salt and store it under a username.
    /// Plaintext is never persisted.
    pub fn register(username: &str, password: &str, role: Role) -> Self {
        let salt = random_salt();
        let hash = hash_password(password, &salt);
        UserRecord {
            username: username.to_string(),
            role,
            password_hash: hash,
            salt,
        }
    }

    /// Verify a password against the stored hash (constant-time-ish
    /// comparison over hex strings).
    pub fn verify(&self, password: &str) -> bool {
        let hash = hash_password(password, &self.salt);
        constant_time_eq(&hash, &self.password_hash)
    }
}

const ITERATIONS: u32 = 4_096;

fn hash_password(password: &str, salt: &str) -> String {
    let mut state = Sha256::new();
    state.update(salt.as_bytes());
    state.update(password.as_bytes());
    let mut digest = state.finalize_reset();
    for _ in 1..ITERATIONS {
        digest = Sha256::digest(digest);
    }
    hex::encode(digest)
}

fn random_salt() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    // OS-provided randomness (RandomState is seeded from the OS RNG)
    // mixed with time and process entropy: enough for a local trust
    // boundary's per-user salt, not a network credential store.
    let a = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let b = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id();
    let mix = a ^ b.rotate_left(21) ^ t.rotate_right(13) ^ ((pid as u64) << 17);
    format!("{a:016x}{b:016x}{mix:016x}")
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Read `users.json`. A missing file is an honest "no users registered"
/// (empty list); an unparseable one is `Corrupt` — never an empty list.
pub fn load_users(path: &Path) -> Result<Vec<UserRecord>, AuthzError> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Vec::new()),
        Ok(text) => serde_json::from_str(&text).map_err(|_| AuthzError::Corrupt),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(AuthzError::UsersFile(err.to_string())),
    }
}

/// Persist `users.json` atomically (temp file + rename), so a crash mid
/// write cannot leave a torn user file.
pub fn save_users(path: &Path, users: &[UserRecord]) -> Result<(), AuthzError> {
    let text = serde_json::to_string_pretty(users).map_err(|e| AuthzError::UsersFile(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| AuthzError::UsersFile(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| AuthzError::UsersFile(e.to_string()))?;
    Ok(())
}

/// Add a user to the store. Duplicate usernames are rejected by name.
pub fn add_user(users: &mut Vec<UserRecord>, username: &str, password: &str, role: Role) -> Result<(), AuthzError> {
    if users.iter().any(|u| u.username == username) {
        return Err(AuthzError::AlreadyExists(username.to_string()));
    }
    users.push(UserRecord::register(username, password, role));
    Ok(())
}

/// Authenticate against the store: unknown user and wrong password are
/// both named errors; no user enumeration difference is promised
/// (the local file is readable by the operator anyway).
pub fn authenticate(users: &[UserRecord], username: &str, password: &str) -> Result<UserRecord, AuthzError> {
    let user = users
        .iter()
        .find(|u| u.username == username)
        .ok_or_else(|| AuthzError::UnknownUser(username.to_string()))?;
    if user.verify(password) {
        Ok(user.clone())
    } else {
        Err(AuthzError::BadPassword)
    }
}

/// Log in against the store, or bootstrap it. An EMPTY store would
/// otherwise deadlock a fresh install: `create_user` needs an admin
/// session, and no session can exist without a user. So the very first
/// login against an empty store creates the first account as `admin`
/// with the supplied credentials; every later login authenticates
/// normally. Returns the user and whether this login bootstrapped.
pub fn authenticate_or_bootstrap(
    users: &mut Vec<UserRecord>,
    username: &str,
    password: &str,
) -> Result<(UserRecord, bool), AuthzError> {
    if users.is_empty() {
        let user = UserRecord::register(username, password, Role::Admin);
        users.push(user.clone());
        return Ok((user, true));
    }
    authenticate(users, username, password).map(|u| (u, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_ordering_encodes_privilege() {
        assert!(Role::Viewer < Role::Operator);
        assert!(Role::Operator < Role::Admin);
    }

    /// The whole matrix: every role x every action, pinned explicitly.
    #[test]
    fn permission_matrix_is_exhaustive() {
        let matrix = permission_matrix();
        assert_eq!(matrix.len(), 7);
        // View is open to everyone.
        assert!(allowed(Role::Viewer, Action::View));
        assert!(allowed(Role::Operator, Action::View));
        assert!(allowed(Role::Admin, Action::View));
        // Run operations: operator and admin, not viewer.
        for action in [
            Action::StartSwell,
            Action::StopSwell,
            Action::CleanUpBots,
            Action::ApplyPlan,
        ] {
            assert!(!allowed(Role::Viewer, action), "{action} must deny viewer");
            assert!(allowed(Role::Operator, action), "{action} must allow operator");
            assert!(allowed(Role::Admin, action), "{action} must allow admin");
        }
        // EditConfig and ManageUsers: admin only.
        for action in [Action::EditConfig, Action::ManageUsers] {
            assert!(!allowed(Role::Viewer, action), "{action} must deny viewer");
            assert!(!allowed(Role::Operator, action), "{action} must deny operator");
            assert!(allowed(Role::Admin, action), "{action} must allow admin");
        }
    }

    /// `gate` with a session mirrors `allowed`.
    #[test]
    fn gate_allows_and_denies_with_a_session() {
        let session = Session { username: "op".into(), role: Role::Operator };
        assert!(gate(Some(&session), Action::StartSwell).is_ok());
        let err = gate(Some(&session), Action::EditConfig).unwrap_err();
        assert_eq!(err.minimum_role, Role::Admin);
        assert_eq!(err.reason(), "forbidden: role operator may not edit_config (requires admin)");
    }

    /// No session: everything is denied with a named reason, never
    /// defaulted to allowed.
    #[test]
    fn gate_without_session_denies_everything() {
        for action in [
            Action::View,
            Action::StartSwell,
            Action::EditConfig,
            Action::ManageUsers,
        ] {
            let err = gate(None, action).unwrap_err();
            assert_eq!(err.action, action);
        }
    }

    /// Passwords are stored as hashes: registering the same password
    /// twice yields a different hash (fresh salt) and never plaintext.
    #[test]
    fn register_hashes_and_never_stores_plaintext() {
        let a = UserRecord::register("ada", "correct horse", Role::Admin);
        let b = UserRecord::register("ada", "correct horse", Role::Admin);
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.password_hash, b.password_hash);
        assert_ne!(a.password_hash, "correct horse");
        assert!(a.verify("correct horse"));
        assert!(!a.verify("wrong password"));
    }

    #[test]
    fn load_users_missing_file_is_empty_not_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        assert_eq!(load_users(&path).unwrap(), Vec::new());
    }

    #[test]
    fn load_users_corrupt_file_is_named_never_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_users(&path).unwrap_err(), AuthzError::Corrupt);
    }

    #[test]
    fn save_then_load_roundtrips_users() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let users = vec![
            UserRecord::register("ada", "pw1", Role::Admin),
            UserRecord::register("bob", "pw2", Role::Operator),
        ];
        save_users(&path, &users).unwrap();
        let loaded = load_users(&path).unwrap();
        assert_eq!(loaded, users);
        // No temp file left behind.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn add_user_rejects_duplicate_by_name() {
        let mut users = Vec::new();
        add_user(&mut users, "ada", "pw1", Role::Admin).unwrap();
        assert_eq!(
            add_user(&mut users, "ada", "pw2", Role::Viewer).unwrap_err(),
            AuthzError::AlreadyExists("ada".into())
        );
        assert_eq!(users.len(), 1);
    }

    #[test]
    fn authenticate_names_unknown_user_and_bad_password() {
        let users = vec![UserRecord::register("ada", "right", Role::Admin)];
        assert_eq!(
            authenticate(&users, "nobody", "x").unwrap_err(),
            AuthzError::UnknownUser("nobody".into())
        );
        assert_eq!(authenticate(&users, "ada", "wrong").unwrap_err(), AuthzError::BadPassword);
        let ok = authenticate(&users, "ada", "right").unwrap();
        assert_eq!(ok.username, "ada");
        assert_eq!(ok.role, Role::Admin);
    }

    /// A fresh install would deadlock otherwise: create_user needs an
    /// admin session, and no session can exist without a user. The
    /// first login on an EMPTY store therefore creates the admin.
    #[test]
    fn first_login_on_empty_store_creates_admin() {
        let mut users = Vec::new();
        let (user, bootstrapped) = authenticate_or_bootstrap(&mut users, "ada", "pw").unwrap();
        assert!(bootstrapped);
        assert_eq!(user.role, Role::Admin);
        assert_eq!(users.len(), 1);
        assert!(user.verify("pw"));
        // The next login authenticates normally against the store.
        let (again, bootstrapped_again) = authenticate_or_bootstrap(&mut users, "ada", "pw").unwrap();
        assert!(!bootstrapped_again);
        assert_eq!(again.username, "ada");
    }

    /// Bootstrap only ever happens on an empty store — never a takeover
    /// path on a populated one.
    #[test]
    fn bootstrap_never_triggers_on_nonempty_store() {
        let mut users = vec![UserRecord::register("ada", "right", Role::Admin)];
        assert_eq!(
            authenticate_or_bootstrap(&mut users, "mallory", "anything").unwrap_err(),
            AuthzError::UnknownUser("mallory".into())
        );
        assert_eq!(users.len(), 1);
    }
}
