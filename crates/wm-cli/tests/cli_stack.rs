//! CLI ↔ controller stack test (WM-0111 / WM-0113): a real axum router on
//! an ephemeral port driven through the CLI's `ureq` client, covering
//! `wm run` → `wm status` → `wm stop`.
//!
//! The client is blocking and the server is async, so the server gets its
//! own thread + runtime; running both on one current-thread runtime would
//! deadlock the moment the test blocks on a request.

use std::sync::Arc;

use wm_cli::commands::{self, CommandError};
use wm_controller::http::router;
use wm_controller::state::ControllerState;

/// Bind the controller HTTP API on an ephemeral port; returns its base URL.
fn spawn_controller() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let app = router(Arc::new(ControllerState::new(5000)));
            axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                .await
                .unwrap();
        });
    });

    format!("http://{addr}")
}

fn good_spectrum() -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
    std::io::Write::write_all(
        &mut f,
        b"arrival:\n  ramp:\n    from_per_second: 1\n    to_per_second: 20\n    duration_secs: 600\nlifetime:\n  distribution: exponential\n  mean_secs: 900\nsaturation_policy: queue\n",
    )
    .unwrap();
    f
}

#[test]
fn run_status_stop_roundtrip() {
    let base = spawn_controller();

    // wm run: validate + create the run record.
    let spectrum = good_spectrum();
    let created = commands::run(spectrum.path(), &base, None).expect("run must succeed");
    assert_eq!(created.run_id, "run-1");
    assert_eq!(created.swell_id, "swell-0001");
    assert_eq!(created.state, "created");

    // wm status: the run shows up alongside the (empty) worker list.
    let info = commands::status(&base).expect("status must succeed");
    assert_eq!(info.status, "running");
    assert_eq!(info.worker_count, 0);
    assert_eq!(info.runs.len(), 1);
    assert_eq!(info.runs[0].run_id, "run-1");
    let text = commands::format_status(&info);
    assert!(text.contains("run run-1"), "rendered status: {text}");

    // wm stop: flips the run to stopped.
    let stopped = commands::stop("run-1", &base).expect("stop must succeed");
    assert_eq!(stopped.state, "stopped");

    // Stopping again is idempotent.
    let again = commands::stop("run-1", &base).expect("re-stop must succeed");
    assert_eq!(again.state, "stopped");

    // wm status now reports the stopped run.
    let info = commands::status(&base).expect("status must succeed");
    assert_eq!(info.runs[0].state, "stopped");
}

#[test]
fn stop_unknown_run_is_reported_as_unknown() {
    let base = spawn_controller();
    let err = commands::stop("run-nope", &base).unwrap_err();
    match err {
        CommandError::Unavailable(msg) => {
            assert!(msg.contains("unknown run"), "unexpected message: {msg}")
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
}

#[test]
fn pinned_swell_id_is_honoured() {
    let base = spawn_controller();
    let spectrum = good_spectrum();
    let created =
        commands::run(spectrum.path(), &base, Some("swell-custom")).expect("run must succeed");
    assert_eq!(created.swell_id, "swell-custom");
}
