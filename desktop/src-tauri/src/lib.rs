//! Wavemaker desktop shell (WM-0601).
//!
//! A thin Tauri host: the native window renders the Vue dashboard and
//! the HTTP plugin lets the webview reach the controller REST API
//! (`/api/v1/status`) without tripping webview CORS.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .run(tauri::generate_context!())
        .expect("error while running the wavemaker desktop application");
}
