//! Structured logging and tracing setup shared by all Wavemaker binaries
//! (WM-0112).
//!
//! JSON output by default (`WM_LOG_FORMAT=text` switches to human-readable);
//! level comes from `RUST_LOG` and defaults to `info`.

use tracing_subscriber::EnvFilter;

/// Initialize the global tracing subscriber. Idempotent: a second call is a
/// no-op (binaries and tests may both call it).
pub fn init(component: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let text = std::env::var("WM_LOG_FORMAT").is_ok_and(|v| v == "text");
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr);

    if text {
        let _ = builder.compact().try_init();
    } else {
        let _ = builder.json().flatten_event(true).try_init();
    }

    tracing::debug!(component, "tracing initialized");
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_is_safe_to_call_twice() {
        super::init("test-a");
        super::init("test-b");
    }

    #[test]
    fn events_are_recorded_without_panic() {
        super::init("test-record");
        tracing::info!("visible-event");
    }
}
