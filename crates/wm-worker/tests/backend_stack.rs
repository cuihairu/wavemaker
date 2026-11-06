//! Backend service stack test (WM-0106): real gRPC server on an ephemeral
//! port driven by the generated `BackendServiceClient`, exercising the
//! handshake / apply_plan / start / stop lifecycle through the WM-0109
//! state machine.

use std::sync::Arc;

use tonic::transport::{Channel, Server};
use tonic::{Request, Response};
use wm_core::backend::BackendState;
use wm_protocol::backend::v1::backend_service_client::BackendServiceClient;
use wm_protocol::backend::v1::backend_service_server::{BackendService, BackendServiceServer};
use wm_protocol::backend::v1::{
    ApplyPlanRequest, ApplyPlanResponse, HandshakeRequest, HandshakeResponse, StartRequest,
    StartResponse, StopRequest, StopResponse,
};
use wm_protocol::common::v1::ErrorCode;
use wm_worker::backend::NativeBackendService;

/// Arc wrapper so the same service instance serves gRPC and lets the test
/// inspect the state machine.
#[derive(Clone)]
struct SharedBackendService(Arc<NativeBackendService>);

#[tonic::async_trait]
impl BackendService for SharedBackendService {
    async fn handshake(
        &self,
        request: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, tonic::Status> {
        self.0.handshake(request).await
    }

    async fn apply_plan(
        &self,
        request: Request<ApplyPlanRequest>,
    ) -> Result<Response<ApplyPlanResponse>, tonic::Status> {
        self.0.apply_plan(request).await
    }

    async fn start(
        &self,
        request: Request<StartRequest>,
    ) -> Result<Response<StartResponse>, tonic::Status> {
        self.0.start(request).await
    }

    async fn stop(
        &self,
        request: Request<StopRequest>,
    ) -> Result<Response<StopResponse>, tonic::Status> {
        self.0.stop(request).await
    }
}

async fn spawn_backend() -> (BackendServiceClient<Channel>, Arc<NativeBackendService>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let svc = Arc::new(NativeBackendService::new(vec![
        "native".into(),
        "kbengine-bot".into(),
    ]));

    let server = Server::builder()
        .add_service(BackendServiceServer::new(SharedBackendService(Arc::clone(
            &svc,
        ))))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
    tokio::spawn(server);

    let client = BackendServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap();
    (client, svc)
}

#[tokio::test]
async fn full_lifecycle_over_grpc_reaches_stopped() {
    let (mut client, svc) = spawn_backend().await;

    // Handshake with the current protocol version → Ready + capabilities.
    let hs = client
        .handshake(HandshakeRequest {
            protocol_version: wm_protocol::PROTOCOL_VERSION,
            backend_id: "backend-native-1".into(),
            adapter_version: "0.1".into(),
            engine_version: "native".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hs.result.unwrap().code(), ErrorCode::Ok);
    assert_eq!(hs.generation, 1);
    assert_eq!(hs.capabilities, vec!["native", "kbengine-bot"]);
    let (state, _) = svc.snapshot();
    assert_eq!(state, BackendState::Ready);

    // ApplyPlan → PlanAccepted (Ready → Initialized → PlanAccepted).
    let ap = client
        .apply_plan(ApplyPlanRequest {
            swell_id: "swell-001".into(),
            generation: hs.generation,
            plan_digest: "sha256:abc".into(),
            plan_payload: b"plan-bytes".to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ap.result.unwrap().code(), ErrorCode::Ok);
    let (state, _) = svc.snapshot();
    assert_eq!(state, BackendState::PlanAccepted);

    // Same digest replays idempotently.
    let ap2 = client
        .apply_plan(ApplyPlanRequest {
            swell_id: "swell-001".into(),
            generation: hs.generation,
            plan_digest: "sha256:abc".into(),
            plan_payload: b"plan-bytes".to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ap2.result.unwrap().code(), ErrorCode::Ok);

    // Start → Running.
    let st = client
        .start(StartRequest {
            swell_id: "swell-001".into(),
            generation: hs.generation,
            operation_id: "op-start-1".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(st.result.unwrap().code(), ErrorCode::Ok);
    let (state, _) = svc.snapshot();
    assert_eq!(state, BackendState::Running);

    // Start replay with the same operation_id is idempotent.
    let st2 = client
        .start(StartRequest {
            swell_id: "swell-001".into(),
            generation: hs.generation,
            operation_id: "op-start-1".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(st2.result.unwrap().code(), ErrorCode::Ok);

    // Stop → Stopped (Running → AdmissionStopped → Draining → Stopped).
    let sp = client
        .stop(StopRequest {
            swell_id: "swell-001".into(),
            generation: hs.generation,
            operation_id: "op-stop-1".into(),
            grace_period_ms: 500,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(sp.result.unwrap().code(), ErrorCode::Ok);
    let (state, _) = svc.snapshot();
    assert_eq!(state, BackendState::Stopped);

    // Stop replay is idempotent even from Stopped.
    let sp2 = client
        .stop(StopRequest {
            swell_id: "swell-001".into(),
            generation: hs.generation,
            operation_id: "op-stop-1".into(),
            grace_period_ms: 500,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(sp2.result.unwrap().code(), ErrorCode::Ok);
}

#[tokio::test]
async fn protocol_mismatch_is_refused_at_handshake() {
    let (mut client, svc) = spawn_backend().await;
    let hs = client
        .handshake(HandshakeRequest {
            protocol_version: wm_protocol::PROTOCOL_VERSION + 99,
            backend_id: "backend-old".into(),
            adapter_version: "0.1".into(),
            engine_version: "native".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let result = hs.result.unwrap();
    assert_eq!(result.code(), ErrorCode::Unsupported);
    assert_eq!(result.reason_code, "protocol_version_mismatch");
    assert!(hs.capabilities.is_empty(), "no capabilities on refusal");

    // Refused handshake must not advance the state machine.
    let (state, gen) = svc.snapshot();
    assert_eq!(state, BackendState::Created);
    assert_eq!(gen.0, 1);
}

#[tokio::test]
async fn stale_generation_is_rejected_and_state_survives() {
    let (mut client, svc) = spawn_backend().await;
    let hs = client
        .handshake(HandshakeRequest {
            protocol_version: wm_protocol::PROTOCOL_VERSION,
            backend_id: "backend-1".into(),
            adapter_version: "0.1".into(),
            engine_version: "native".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hs.result.unwrap().code(), ErrorCode::Ok);

    // Request stamped with a generation behind the machine's → refused.
    let ap = client
        .apply_plan(ApplyPlanRequest {
            swell_id: "swell-001".into(),
            generation: 0, // stale (current is 1)
            plan_digest: "sha256:old".into(),
            plan_payload: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    let result = ap.result.unwrap();
    assert_eq!(result.code(), ErrorCode::FailedPrecondition);
    assert_eq!(result.reason_code, "stale_generation");

    // State machine untouched by the rejected request.
    let (state, gen) = svc.snapshot();
    assert_eq!(state, BackendState::Ready);
    assert_eq!(gen.0, 1);
}

#[tokio::test]
async fn conflicting_plan_digest_is_a_precondition_failure() {
    let (mut client, svc) = spawn_backend().await;
    client
        .handshake(HandshakeRequest {
            protocol_version: wm_protocol::PROTOCOL_VERSION,
            backend_id: "backend-1".into(),
            adapter_version: "0.1".into(),
            engine_version: "native".into(),
        })
        .await
        .unwrap()
        .into_inner();

    let first = client
        .apply_plan(ApplyPlanRequest {
            swell_id: "swell-001".into(),
            generation: 1,
            plan_digest: "sha256:plan-a".into(),
            plan_payload: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.result.unwrap().code(), ErrorCode::Ok);

    // A different digest while already PlanAccepted is a transition failure.
    let conflict = client
        .apply_plan(ApplyPlanRequest {
            swell_id: "swell-001".into(),
            generation: 1,
            plan_digest: "sha256:plan-b".into(),
            plan_payload: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    let result = conflict.result.unwrap();
    assert_eq!(result.code(), ErrorCode::FailedPrecondition);
    assert_eq!(result.reason_code, "invalid_state_transition");

    // The originally accepted plan is still in force.
    let (state, _) = svc.snapshot();
    assert_eq!(state, BackendState::PlanAccepted);
}
