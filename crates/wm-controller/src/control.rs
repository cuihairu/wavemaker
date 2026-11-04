//! `wavemaker.control.v1.ControlService` implementation (WM-0105 server
//! side). The controller hosts the service; workers are gRPC clients.

use crate::state::ControllerState;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use wm_protocol::common::v1::{ErrorCode, OperationResult};
use wm_protocol::control::v1::{
    control_service_server::ControlService, HeartbeatRequest, HeartbeatResponse,
    RegisterWorkerRequest, RegisterWorkerResponse,
};

pub struct ControlServiceImpl {
    pub state: Arc<ControllerState>,
    pub heartbeat_interval_ms: u32,
}

fn ok_result(generation: u64) -> OperationResult {
    OperationResult {
        code: ErrorCode::Ok as i32,
        reason_code: "ok".into(),
        detail: String::new(),
        generation,
    }
}

fn failed(code: ErrorCode, reason: &str, detail: String, generation: u64) -> OperationResult {
    OperationResult {
        code: code as i32,
        reason_code: reason.into(),
        detail,
        generation,
    }
}

#[tonic::async_trait]
impl ControlService for ControlServiceImpl {
    async fn register_worker(
        &self,
        request: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        let req = request.into_inner();

        // Handshake version gate (WM-0013 §2.3): incompatible protocol
        // versions are rejected explicitly, never silently downgraded.
        if req.protocol_version != wm_protocol::PROTOCOL_VERSION {
            return Ok(Response::new(RegisterWorkerResponse {
                result: Some(failed(
                    ErrorCode::Unsupported,
                    "protocol_version_mismatch",
                    format!(
                        "worker speaks protocol v{}, controller requires v{}; refusing handshake",
                        req.protocol_version,
                        wm_protocol::PROTOCOL_VERSION
                    ),
                    0,
                )),
                generation: 0,
                heartbeat_interval_ms: 0,
            }));
        }

        if req.worker_id.is_empty() {
            return Ok(Response::new(RegisterWorkerResponse {
                result: Some(failed(
                    ErrorCode::InvalidArgument,
                    "worker_id_empty",
                    "worker_id must be non-empty".into(),
                    0,
                )),
                generation: 0,
                heartbeat_interval_ms: 0,
            }));
        }

        let generation = self.state.register(
            &req.worker_id,
            req.capabilities,
            req.adapter_version,
            req.runtime_version,
        );
        tracing::info!(worker_id = %req.worker_id, generation, "worker registered");

        Ok(Response::new(RegisterWorkerResponse {
            result: Some(ok_result(generation)),
            generation,
            heartbeat_interval_ms: self.heartbeat_interval_ms,
        }))
    }

    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();
        let known = self.state.worker(&req.worker_id).is_some();

        if !known {
            return Ok(Response::new(HeartbeatResponse {
                result: Some(failed(
                    ErrorCode::NotFound,
                    "worker_not_registered",
                    format!("no registration for worker `{}`", req.worker_id),
                    0,
                )),
                accepted: false,
            }));
        }

        if !self.state.heartbeat(&req.worker_id, req.generation) {
            return Ok(Response::new(HeartbeatResponse {
                result: Some(failed(
                    ErrorCode::Aborted,
                    "stale_generation",
                    format!(
                        "heartbeat generation {} is behind the registered generation",
                        req.generation
                    ),
                    0,
                )),
                accepted: false,
            }));
        }

        tracing::debug!(worker_id = %req.worker_id, generation = req.generation, "heartbeat accepted");
        Ok(Response::new(HeartbeatResponse {
            result: Some(ok_result(req.generation)),
            accepted: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ControllerState;
    use wm_protocol::control::v1::WorkerLoad;

    fn svc() -> ControlServiceImpl {
        ControlServiceImpl {
            state: Arc::new(ControllerState::new(5000)),
            heartbeat_interval_ms: 5000,
        }
    }

    async fn register_ok(svc: &ControlServiceImpl, id: &str) -> RegisterWorkerResponse {
        svc.register_worker(Request::new(RegisterWorkerRequest {
            worker_id: id.into(),
            protocol_version: wm_protocol::PROTOCOL_VERSION,
            adapter_version: "0.1".into(),
            runtime_version: "0.1".into(),
            capabilities: vec!["native".into()],
        }))
        .await
        .unwrap()
        .into_inner()
    }

    #[tokio::test]
    async fn rejects_protocol_version_mismatch() {
        let svc = svc();
        let resp = svc
            .register_worker(Request::new(RegisterWorkerRequest {
                worker_id: "w".into(),
                protocol_version: wm_protocol::PROTOCOL_VERSION + 1,
                adapter_version: String::new(),
                runtime_version: String::new(),
                capabilities: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        let result = resp.result.unwrap();
        assert_eq!(result.code(), ErrorCode::Unsupported);
        assert_eq!(result.reason_code, "protocol_version_mismatch");
        assert!(ControllerState::new(1).workers().is_empty());
    }

    #[tokio::test]
    async fn rejects_empty_worker_id() {
        let svc = svc();
        let resp = svc
            .register_worker(Request::new(RegisterWorkerRequest {
                worker_id: String::new(),
                protocol_version: wm_protocol::PROTOCOL_VERSION,
                adapter_version: String::new(),
                runtime_version: String::new(),
                capabilities: vec![],
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(resp.result.unwrap().code(), ErrorCode::InvalidArgument);
    }

    #[tokio::test]
    async fn register_then_heartbeat_roundtrip() {
        let svc = svc();
        let resp = register_ok(&svc, "worker-a").await;
        assert_eq!(resp.result.unwrap().code(), ErrorCode::Ok);
        assert_eq!(resp.generation, 1);
        assert_eq!(resp.heartbeat_interval_ms, 5000);

        let hb = svc
            .heartbeat(Request::new(HeartbeatRequest {
                worker_id: "worker-a".into(),
                generation: resp.generation,
                load: Some(WorkerLoad::default()),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(hb.accepted);
        assert_eq!(hb.result.unwrap().code(), ErrorCode::Ok);
    }

    #[tokio::test]
    async fn heartbeat_for_unknown_worker_is_not_found() {
        let svc = svc();
        let hb = svc
            .heartbeat(Request::new(HeartbeatRequest {
                worker_id: "ghost".into(),
                generation: 1,
                load: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!hb.accepted);
        let result = hb.result.unwrap();
        assert_eq!(result.code(), ErrorCode::NotFound);
        assert_eq!(result.reason_code, "worker_not_registered");
    }

    #[tokio::test]
    async fn stale_generation_heartbeat_is_aborted() {
        let svc = svc();
        let g1 = register_ok(&svc, "worker-a").await.generation;
        let g2 = register_ok(&svc, "worker-a").await.generation;
        let hb = svc
            .heartbeat(Request::new(HeartbeatRequest {
                worker_id: "worker-a".into(),
                generation: g1,
                load: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!hb.accepted);
        assert_eq!(hb.result.unwrap().code(), ErrorCode::Aborted);
        let _ = g2;
    }
}
