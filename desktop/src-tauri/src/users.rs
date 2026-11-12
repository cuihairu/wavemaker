//! Tauri commands for login, sessions and user management (WM-0607).
//!
//! The engine-side decision functions live in `authz.rs`; these commands
//! hold the app state (current session, users file path) and turn authz
//! results into JSON for the webview. Every error is a named string the
//! UI renders verbatim — never a silent failure.
//!
//! User management is itself admin-gated through the same engine-side
//! matrix every other command consults (ADR-009 decision 5).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::authz::{self, Action, Role, Session};

/// Shared authz state: the logged-in session (if any). Credentials are
/// never kept here — only identity and role.
#[derive(Default)]
pub struct AuthzState {
    session: Mutex<Option<Session>>,
}

/// Where `users.json` lives: the app's data directory, created lazily.
/// A missing file is honestly "no users" (see authz::load_users).
fn users_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create app data dir: {e}"))?;
    Ok(dir.join("users.json"))
}

/// Who is logged in and with which role — serialized for the webview.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionInfo {
    pub username: String,
    pub role: String,
}

impl SessionInfo {
    fn from_session(session: &Session) -> Self {
        SessionInfo {
            username: session.username.clone(),
            role: session.role.as_str().to_string(),
        }
    }
}

/// One permission-matrix row for the webview: the action, whether each
/// role may perform it (ordered viewer/operator/admin) and the minimum
/// role label used to explain a denial.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatrixRow {
    pub action: String,
    pub viewer: bool,
    pub operator: bool,
    pub admin: bool,
    pub minimum_role: Option<String>,
}

/// The whole permission matrix (ADR-009 decision 3) — the webview renders
/// this table verbatim from the engine's single source of truth.
#[tauri::command]
pub fn permission_matrix() -> Vec<MatrixRow> {
    authz::permission_matrix()
        .iter()
        .map(|entry| MatrixRow {
            action: entry.action.as_str().to_string(),
            viewer: authz::allowed(Role::Viewer, entry.action),
            operator: authz::allowed(Role::Operator, entry.action),
            admin: authz::allowed(Role::Admin, entry.action),
            minimum_role: entry.minimum_role.map(|r| r.as_str().to_string()),
        })
        .collect()
}

/// Who is currently logged in, or null.
#[tauri::command]
pub fn current_session(state: State<'_, AuthzState>) -> Option<SessionInfo> {
    state.session.lock().unwrap().as_ref().map(SessionInfo::from_session)
}

/// Log in against `users.json`. Unknown user and wrong password are
/// distinct named errors; success stores identity and role only — never
/// the password. An empty store bootstraps: the first login creates the
/// first account as admin (otherwise a fresh install could never log
/// in, since creating a user needs an admin session).
#[tauri::command]
pub fn login(
    app: AppHandle,
    state: State<'_, AuthzState>,
    username: String,
    password: String,
) -> Result<SessionInfo, String> {
    let path = users_path(&app)?;
    let mut users = authz::load_users(&path).map_err(|e| format!("cannot load users: {e:?}"))?;
    let (user, bootstrapped) = authz::authenticate_or_bootstrap(&mut users, &username, &password)
        .map_err(|e| format!("login_failed: {e:?}"))?;
    if bootstrapped {
        // First-run setup: persist the freshly created admin account.
        authz::save_users(&path, &users).map_err(|e| format!("cannot save users: {e:?}"))?;
    }
    *state.session.lock().unwrap() = Some(Session {
        username: user.username.clone(),
        role: user.role,
    });
    Ok(SessionInfo {
        username: user.username,
        role: user.role.as_str().to_string(),
    })
}

/// Log out. Idempotent: logging out with no session is fine.
#[tauri::command]
pub fn logout(state: State<'_, AuthzState>) {
    *state.session.lock().unwrap() = None;
}

/// Gate a session-requiring action through the engine-side matrix.
/// Crate-visible so other shell commands (e.g. spectrum save's
/// `edit_config` gate) share the same trust boundary.
pub(crate) fn authorize(state: &AuthzState, action: Action) -> Result<Session, String> {
    let session = state.session.lock().unwrap().clone();
    match session {
        None => Err("not_authenticated".to_string()),
        Some(session) => authz::gate(Some(&session), action)
            .map(|_| session)
            .map_err(|err| err.reason()),
    }
}

/// List registered users (identity and role only — never hashes).
/// Admin-only via the matrix (`manage_users`).
#[tauri::command]
pub fn list_users(app: AppHandle, state: State<'_, AuthzState>) -> Result<Vec<SessionInfo>, String> {
    authorize(&state, Action::ManageUsers)?;
    let path = users_path(&app)?;
    let users = authz::load_users(&path).map_err(|e| format!("cannot load users: {e:?}"))?;
    Ok(users
        .iter()
        .map(|u| SessionInfo {
            username: u.username.clone(),
            role: u.role.as_str().to_string(),
        })
        .collect())
}

/// Create a user (admin-only). The password is hashed before it ever
/// reaches the file.
#[tauri::command]
pub fn create_user(
    app: AppHandle,
    state: State<'_, AuthzState>,
    username: String,
    password: String,
    role: String,
) -> Result<SessionInfo, String> {
    authorize(&state, Action::ManageUsers)?;
    let role = parse_role(&role)?;
    let path = users_path(&app)?;
    let mut users = authz::load_users(&path).map_err(|e| format!("cannot load users: {e:?}"))?;
    authz::add_user(&mut users, &username, &password, role)
        .map_err(|e| format!("create_user_failed: {e:?}"))?;
    authz::save_users(&path, &users).map_err(|e| format!("cannot save users: {e:?}"))?;
    Ok(SessionInfo {
        username,
        role: role.as_str().to_string(),
    })
}

/// Delete a user (admin-only). Deleting your own account is refused —
/// that would lock the last admin out.
#[tauri::command]
pub fn delete_user(app: AppHandle, state: State<'_, AuthzState>, username: String) -> Result<(), String> {
    let session = authorize(&state, Action::ManageUsers)?;
    if session.username == username {
        return Err("cannot delete your own account".to_string());
    }
    let path = users_path(&app)?;
    let mut users = authz::load_users(&path).map_err(|e| format!("cannot load users: {e:?}"))?;
    let before = users.len();
    users.retain(|u| u.username != username);
    if users.len() == before {
        return Err(format!("delete_user_failed: UnknownUser({username:?})"));
    }
    authz::save_users(&path, &users).map_err(|e| format!("cannot save users: {e:?}"))?;
    Ok(())
}

/// Change a user's role (admin-only).
#[tauri::command]
pub fn set_user_role(
    app: AppHandle,
    state: State<'_, AuthzState>,
    username: String,
    role: String,
) -> Result<(), String> {
    authorize(&state, Action::ManageUsers)?;
    let role = parse_role(&role)?;
    let path = users_path(&app)?;
    let mut users = authz::load_users(&path).map_err(|e| format!("cannot load users: {e:?}"))?;
    let user = users
        .iter_mut()
        .find(|u| u.username == username)
        .ok_or_else(|| format!("set_user_role_failed: UnknownUser({username:?})"))?;
    user.role = role;
    authz::save_users(&path, &users).map_err(|e| format!("cannot save users: {e:?}"))?;
    Ok(())
}

fn parse_role(role: &str) -> Result<Role, String> {
    match role {
        "viewer" => Ok(Role::Viewer),
        "operator" => Ok(Role::Operator),
        "admin" => Ok(Role::Admin),
        other => Err(format!("unknown role {other:?} (viewer|operator|admin)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_rows_mirror_the_engine_matrix() {
        let rows = permission_matrix();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].action, "view");
        assert!(rows[0].viewer && rows[0].operator && rows[0].admin);
        assert_eq!(rows[0].minimum_role, None);
        let stop = rows.iter().find(|r| r.action == "stop_swell").unwrap();
        assert!(!stop.viewer && stop.operator && stop.admin);
        assert_eq!(stop.minimum_role.as_deref(), Some("operator"));
        let edit = rows.iter().find(|r| r.action == "edit_config").unwrap();
        assert!(!edit.viewer && !edit.operator && edit.admin);
        assert_eq!(edit.minimum_role.as_deref(), Some("admin"));
    }

    #[test]
    fn parse_role_accepts_exactly_three_roles() {
        assert_eq!(parse_role("viewer").unwrap(), Role::Viewer);
        assert_eq!(parse_role("operator").unwrap(), Role::Operator);
        assert_eq!(parse_role("admin").unwrap(), Role::Admin);
        assert!(parse_role("root").is_err());
    }
}
