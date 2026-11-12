//! Wavemaker desktop shell (WM-0601).
//!
//! A thin Tauri host: the native window renders the Vue dashboard and
//! the HTTP plugin lets the webview reach the controller REST API
//! (`/api/v1/status`) without tripping webview CORS. Spectrum editing
//! (WM-0603) goes through local commands that reuse wm-core's
//! validation so the report matches `wm validate` exactly. The load
//! timeline (WM-0604) reuses wm-runtime's ArrivalPlan. Users, roles and
//! the permission matrix (WM-0607) are local to this shell — the
//! controller REST stays single-operator (ADR-008; ADR-009).

mod authz;
mod spectrum;
mod timeline;
mod users;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .manage(users::AuthzState::default())
        .invoke_handler(tauri::generate_handler![
            spectrum::validate_spectrum,
            spectrum::load_spectrum_file,
            spectrum::save_spectrum_file,
            timeline::load_timeline,
            users::permission_matrix,
            users::current_session,
            users::login,
            users::logout,
            users::list_users,
            users::create_user,
            users::delete_user,
            users::set_user_role,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the wavemaker desktop application");
}
