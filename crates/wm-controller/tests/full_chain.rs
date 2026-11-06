//! Full-chain integration test (WM-0113): Controller ↔ Worker ↔ Supervisor.
//!
//! One test spins up the real controller (gRPC `ControlService` + Axum HTTP
//! on ephemeral ports), connects a real `WorkerSession` to it, and drives a
//! real `NativeProcessLauncher` child — the Phase 1 exit criteria in one
//! place:
//!   - the Controller can register and monitor a Worker;
//!   - the Supervisor can start, detect, and stop a child process;
//!   - abnormal process exits are recorded correctly;
//!   - a protocol-version mismatch is refused at handshake.

use std::sync::Arc;
use std::time::Duration;

use wm_controller::control::ControlServiceImpl;
use wm_controller::http;
use wm_controller::state::ControllerState;
use wm_protocol::common::v1::ErrorCode;
use wm_protocol::control::v1::control_service_client::ControlServiceClient;
use wm_protocol::control::v1::control_service_server::ControlServiceServer;
use wm_protocol::control::v1::RegisterWorkerRequest;
use wm_supervisor::{
    ExitRecord, NativeProcessLauncher, ProcessSpec, ProcessState, ProcessSupervisor,
};
use wm_worker::{WorkerOptions, WorkerSession};

/// Run the controller's gRPC + HTTP surfaces on ephemeral ports and return
/// (grpc_url, http_url, state). Both servers live until the process ends;
/// tests are short-lived so no explicit teardown is needed.
async fn start_controller() -> (String, String, Arc<ControllerState>) {
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http_listener.local_addr().unwrap();

    let state = Arc::new(ControllerState::new(500));

    let grpc_state = Arc::clone(&state);
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(ControlServiceServer::new(ControlServiceImpl {
                state: grpc_state,
                heartbeat_interval_ms: 500,
            }))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            )),
    );

    let http_state = Arc::clone(&state);
    tokio::spawn(async move {
        axum::serve(http_listener, http::router(http_state))
            .await
            .unwrap();
    });

    (
        format!("http://{grpc_addr}"),
        format!("http://{http_addr}"),
        state,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_registers_monitors_and_worker_heartbeats() {
    let (grpc_url, http_url, state) = start_controller().await;

    // A real worker session registers against the real controller.
    let options = WorkerOptions {
        controller_endpoint: grpc_url,
        worker_id: "worker-full".into(),
        heartbeat_interval_ms: 100,
        ..WorkerOptions::default()
    };
    let mut session = WorkerSession::connect(&options)
        .await
        .expect("worker must register");
    assert_eq!(session.generation(), 1);

    // Heartbeats are accepted while the lease is fresh.
    let accepted = session
        .heartbeat(Default::default())
        .await
        .expect("heartbeat must round-trip");
    assert!(accepted, "fresh heartbeat must be accepted");

    // The controller's HTTP status view lists the worker (spawn_blocking:
    // `ureq` is synchronous and would otherwise stall a runtime worker).
    let status_url = format!("{http_url}/api/v1/status");
    let body = tokio::task::spawn_blocking(move || {
        ureq::get(&status_url)
            .call()
            .unwrap()
            .into_string()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(body.contains("worker-full"), "status body: {body}");
    assert!(state.worker("worker-full").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protocol_mismatch_is_refused_at_registration() {
    let (grpc_url, _http_url, state) = start_controller().await;

    let mut client = ControlServiceClient::connect(grpc_url).await.unwrap();
    let resp = client
        .register_worker(RegisterWorkerRequest {
            worker_id: "worker-old".into(),
            protocol_version: wm_protocol::PROTOCOL_VERSION + 7,
            adapter_version: "0.1".into(),
            runtime_version: "0.1".into(),
            capabilities: vec![],
        })
        .await
        .unwrap()
        .into_inner();

    let result = resp.result.unwrap();
    assert_eq!(result.code(), ErrorCode::Unsupported);
    assert_eq!(result.reason_code, "protocol_version_mismatch");
    assert!(
        state.worker("worker-old").is_none(),
        "a refused handshake must not register the worker"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supervisor_starts_detects_and_stops_a_child() {
    let mut launcher = NativeProcessLauncher::new();

    // Start a long-lived child and observe it as Running.
    let spec = ProcessSpec::new("sleep", &["30"]);
    let pid = launcher.start(&spec).await.expect("start must succeed");
    assert!(pid > 0);
    match launcher.status().await.unwrap() {
        ProcessState::Running { pid: observed } => assert_eq!(observed, pid),
        other => panic!("expected Running, got {other:?}"),
    }

    // Graceful stop: SIGTERM within the grace period.
    let record = launcher
        .terminate(Duration::from_secs(5))
        .await
        .expect("terminate must succeed");
    assert!(
        !record.uptime.is_zero(),
        "exit record must carry the uptime"
    );
    match launcher.status().await.unwrap() {
        ProcessState::Exited(_) => {}
        other => panic!("expected Exited after terminate, got {other:?}"),
    }

    // A second terminate on an already-collected child is refused, not a
    // silent success.
    let err = launcher.terminate(Duration::from_secs(1)).await;
    assert!(
        err.is_err(),
        "terminate after exit must not pretend success"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abnormal_child_exit_is_recorded() {
    let mut launcher = NativeProcessLauncher::new();

    // Exit code 3 with a stderr line: the classic abnormal exit.
    let mut spec = ProcessSpec::new("sh", &["-c", "echo boom >&2; exit 3"]);
    spec.stdout = wm_supervisor::StreamSink::Capture;
    spec.stderr = wm_supervisor::StreamSink::Capture;
    launcher.start(&spec).await.expect("start must succeed");

    let record: ExitRecord = loop {
        match launcher.status().await.unwrap() {
            ProcessState::Exited(record) => break record,
            ProcessState::Running { .. } => tokio::time::sleep(Duration::from_millis(10)).await,
            ProcessState::Prepared => panic!("child vanished without an exit record"),
        }
    };

    assert_eq!(record.code, Some(3));
    assert!(record.signal.is_none(), "clean signal-free exit");
    assert!(record.is_abnormal(), "non-zero exit is abnormal");
    assert!(
        String::from_utf8_lossy(&record.stderr_tail).contains("boom"),
        "stderr tail must carry the log tail"
    );
}
