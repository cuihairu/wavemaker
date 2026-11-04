//! Axum HTTP surface: `GET /healthz` and `GET /api/v1/status` (WM-0104).

use crate::state::ControllerState;
use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub generation: u64,
    pub capabilities: Vec<String>,
    pub adapter_version: String,
    pub runtime_version: String,
    pub seconds_since_heartbeat: u64,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub protocol_version: u32,
    pub worker_count: usize,
    pub expired_workers: Vec<String>,
    pub workers: Vec<WorkerStatus>,
}

pub fn router(state: Arc<ControllerState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/status", get(status))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn status(State(state): State<Arc<ControllerState>>) -> Json<StatusResponse> {
    let workers: Vec<WorkerStatus> = state
        .workers()
        .into_iter()
        .map(|w| WorkerStatus {
            seconds_since_heartbeat: w.last_heartbeat.elapsed().as_secs(),
            worker_id: w.worker_id,
            generation: w.generation,
            capabilities: w.capabilities,
            adapter_version: w.adapter_version,
            runtime_version: w.runtime_version,
        })
        .collect();
    Json(StatusResponse {
        status: "running".to_string(),
        protocol_version: wm_protocol::PROTOCOL_VERSION,
        worker_count: workers.len(),
        expired_workers: state.expired_workers(),
        workers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = router(Arc::new(ControllerState::new(5000)));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), b"ok");
    }

    #[tokio::test]
    async fn status_lists_registered_workers() {
        let state = Arc::new(ControllerState::new(5000));
        state.register(
            "worker-a",
            vec!["native".into()],
            "0.1".into(),
            "0.1".into(),
        );
        let app = router(state);

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: StatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.status, "running");
        assert_eq!(parsed.protocol_version, wm_protocol::PROTOCOL_VERSION);
        assert_eq!(parsed.worker_count, 1);
        assert_eq!(parsed.workers[0].worker_id, "worker-a");
        assert!(parsed.expired_workers.is_empty());
    }

    #[tokio::test]
    async fn unknown_route_is_404() {
        let app = router(Arc::new(ControllerState::new(5000)));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }
}
