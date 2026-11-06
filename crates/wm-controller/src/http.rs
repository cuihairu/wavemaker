//! Axum HTTP surface: `GET /healthz`, `GET /api/v1/status` (WM-0104) and
//! the run endpoints behind `wm run` / `wm stop` (WM-0111).

use crate::state::{ControllerState, RunEntry};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
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
    pub runs: Vec<RunEntry>,
}

/// Request body for `POST /api/v1/runs`.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CreateRunRequest {
    /// Optional caller-pinned swell id; the controller assigns one otherwise.
    #[serde(default)]
    pub swell_id: Option<String>,
    /// Spectrum config path this run is based on (already validated by
    /// `wm validate` on the client side).
    #[serde(default)]
    pub spectrum_path: String,
}

pub fn router(state: Arc<ControllerState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/status", get(status))
        .route("/api/v1/runs", post(create_run))
        .route("/api/v1/runs/{run_id}", get(get_run))
        .route("/api/v1/runs/{run_id}/stop", post(stop_run))
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
        runs: state.runs(),
    })
}

async fn create_run(
    State(state): State<Arc<ControllerState>>,
    Json(req): Json<CreateRunRequest>,
) -> (StatusCode, Json<RunEntry>) {
    let entry = state.create_run(req.swell_id, req.spectrum_path);
    tracing::info!(
        run_id = %entry.run_id,
        swell_id = %entry.swell_id,
        spectrum_path = %entry.spectrum_path,
        "run created"
    );
    (StatusCode::CREATED, Json(entry))
}

async fn get_run(
    State(state): State<Arc<ControllerState>>,
    Path(run_id): Path<String>,
) -> Result<Json<RunEntry>, StatusCode> {
    state.run(&run_id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn stop_run(
    State(state): State<Arc<ControllerState>>,
    Path(run_id): Path<String>,
) -> Result<(StatusCode, Json<RunEntry>), StatusCode> {
    let entry = state.stop_run(&run_id).ok_or(StatusCode::NOT_FOUND)?;
    tracing::info!(run_id = %entry.run_id, "run stopped");
    Ok((StatusCode::OK, Json(entry)))
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

    fn json_body(value: &impl Serialize) -> Body {
        Body::from(serde_json::to_vec(value).unwrap())
    }

    #[tokio::test]
    async fn run_lifecycle_create_get_stop() {
        let state = Arc::new(ControllerState::new(5000));

        // Create: 201 with an assigned run/swell id.
        let app = router(Arc::clone(&state));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/v1/runs")
                    .header("content-type", "application/json")
                    .body(json_body(&CreateRunRequest {
                        swell_id: None,
                        spectrum_path: "spectrum.yaml".into(),
                    }))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let created: RunEntry = serde_json::from_slice(&body).unwrap();
        assert_eq!(created.run_id, "run-1");
        assert_eq!(created.swell_id, "swell-0001");
        assert_eq!(created.state, crate::state::RunState::Created);

        // Fetch by id.
        let app = router(Arc::clone(&state));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/v1/runs/{}", created.run_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // Stop: flips to stopped.
        let app = router(Arc::clone(&state));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/runs/{}/stop", created.run_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let stopped: RunEntry = serde_json::from_slice(&body).unwrap();
        assert_eq!(stopped.state, crate::state::RunState::Stopped);

        // Status now lists the run.
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
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed: StatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.runs.len(), 1);
        assert_eq!(parsed.runs[0].state, crate::state::RunState::Stopped);
    }

    #[tokio::test]
    async fn unknown_run_is_404() {
        let state = Arc::new(ControllerState::new(5000));

        let app = router(Arc::clone(&state));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/runs/run-404")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        let app = router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/v1/runs/run-404/stop")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let state = Arc::new(ControllerState::new(5000));
        let entry = state.create_run(None, "spectrum.yaml".into());

        for _ in 0..2 {
            let app = router(Arc::clone(&state));
            let resp = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri(format!("/api/v1/runs/{}/stop", entry.run_id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), 200);
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            let run: RunEntry = serde_json::from_slice(&body).unwrap();
            assert_eq!(run.state, crate::state::RunState::Stopped);
        }
    }
}
