//! Controller address resolution and raw REST forwarding (WM-MCP-001).
//!
//! The MCP layer **forwards** the controller's responses verbatim — it does
//! not re-decode them into its own model. Parsing logic lives on the
//! controller (`crates/wm-controller/src/http.rs`); typed client calls for
//! run create/stop live in `wm-cli`. Only the address fallback chain and the
//! transport error vocabulary are defined here, once.
//!
//! Address precedence mirrors the CLI: `wm --controller` first, then the
//! `WM_CONTROLLER_URL` environment variable, then
//! `commands::DEFAULT_CONTROLLER` — so an agent and the CLI always agree on
//! where the controller is.

/// Resolve the controller base URL: explicit argument, then the
/// `WM_CONTROLLER_URL` environment variable (the CLI's env), then the CLI's
/// default `http://127.0.0.1:7600`.
pub fn controller_base(explicit: Option<&str>) -> String {
    explicit
        .map(str::to_string)
        .or_else(|| std::env::var("WM_CONTROLLER_URL").ok())
        .unwrap_or_else(|| wm_cli::commands::DEFAULT_CONTROLLER.to_string())
}

fn trim_slashes(mut base: String) -> String {
    while base.ends_with('/') {
        base.pop();
    }
    base
}

/// `GET {base}{path}` and return the body verbatim. Errors carry the same
/// vocabulary as the CLI's `CommandError::Unavailable` messages.
pub fn get(base: &str, path: &str) -> Result<String, String> {
    let url = format!("{}{}", trim_slashes(base.to_string()), path);
    match ureq::get(&url).call() {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("bad response from {url}: {e}")),
        Err(ureq::Error::Status(code, resp)) => Err(format!("HTTP {code} {}", resp.status_text())),
        Err(ureq::Error::Transport(t)) => Err(format!("controller unavailable: {t}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_defaults_then_env_then_explicit() {
        std::env::remove_var("WM_CONTROLLER_URL");
        assert_eq!(controller_base(None), "http://127.0.0.1:7600");

        std::env::set_var("WM_CONTROLLER_URL", "http://127.0.0.1:9999");
        assert_eq!(controller_base(None), "http://127.0.0.1:9999");
        assert_eq!(
            controller_base(Some("http://127.0.0.1:1")),
            "http://127.0.0.1:1",
            "explicit argument wins over env"
        );
        std::env::remove_var("WM_CONTROLLER_URL");
    }

    #[test]
    fn get_trailing_slashes_are_trimmed() {
        // A unreachable controller still exercises the URL shaping path
        // (trailing slashes must not produce `//api/...`).
        let err = get("http://127.0.0.1:1//", "/api/v1/status").unwrap_err();
        assert!(err.contains("controller unavailable"), "{err}");
    }
}
