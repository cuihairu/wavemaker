//! Controller stack test: real gRPC server on an ephemeral port driven by
//! the generated `ControlServiceClient` (WM-0113 groundwork).

use std::sync::Arc;
use tokio::net::TcpListener;
use wm_controller::control::ControlServiceImpl;
use wm_controller::state::ControllerState;
use wm_protocol::common::v1::ErrorCode;
use wm_protocol::control::v1::control_service_client::ControlServiceClient;
use wm_protocol::control::v1::control_service_server::ControlServiceServer;
use wm_protocol::control::v1::{HeartbeatRequest, RegisterWorkerRequest};

#[tokio::test]
async fn worker_can_register_and_heartbeat_over_grpc() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state = Arc::new(ControllerState::new(5000));
    let server = tonic::transport::Server::builder()
        .add_service(ControlServiceServer::new(ControlServiceImpl {
            state: Arc::clone(&state),
            heartbeat_interval_ms: 5000,
        }))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));

    let server_handle = tokio::spawn(server);
    let mut client = ControlServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap();

    // Incompatible protocol version is refused at handshake.
    let refused = client
        .register_worker(RegisterWorkerRequest {
            worker_id: "w-bad".into(),
            protocol_version: 99,
            adapter_version: "0.1".into(),
            runtime_version: "0.1".into(),
            capabilities: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        refused.result.unwrap().code(),
        ErrorCode::Unsupported,
        "protocol mismatch must be rejected, never downgraded"
    );

    let reg = client
        .register_worker(RegisterWorkerRequest {
            worker_id: "worker-1".into(),
            protocol_version: wm_protocol::PROTOCOL_VERSION,
            adapter_version: "0.1".into(),
            runtime_version: "0.1".into(),
            capabilities: vec!["native".into()],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reg.result.unwrap().code(), ErrorCode::Ok);
    assert_eq!(reg.generation, 1);
    assert_eq!(reg.heartbeat_interval_ms, 5000);

    let hb = client
        .heartbeat(HeartbeatRequest {
            worker_id: "worker-1".into(),
            generation: reg.generation,
            load: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(hb.accepted);

    // Registration is visible through controller state (HTTP status reads
    // from the same map).
    let entry = state.worker("worker-1").expect("registered");
    assert_eq!(entry.capabilities, vec!["native".to_string()]);

    server_handle.abort();
}
