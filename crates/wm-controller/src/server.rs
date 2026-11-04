//! Controller server entry point: binds the Axum API and the gRPC control
//! service and serves until Ctrl-C (WM-0104).

use crate::config::ControllerConfig;
use crate::control::ControlServiceImpl;
use crate::http;
use crate::state::ControllerState;
use std::sync::Arc;
use wm_protocol::control::v1::control_service_server::ControlServiceServer;

/// Run the controller until shutdown is requested. Returns an error when
/// either listener fails to bind.
pub async fn run(config: ControllerConfig) -> anyhow::Result<()> {
    let state = Arc::new(ControllerState::new(config.heartbeat_interval_ms));

    let grpc = tonic::transport::Server::builder()
        .add_service(ControlServiceServer::new(ControlServiceImpl {
            state: Arc::clone(&state),
            heartbeat_interval_ms: config.heartbeat_interval_ms,
        }))
        .serve(config.grpc_addr.parse()?);

    let http_task = axum::serve(
        tokio::net::TcpListener::bind(&config.http_addr).await?,
        http::router(Arc::clone(&state)),
    );

    tracing::info!(
        http_addr = %config.http_addr,
        grpc_addr = %config.grpc_addr,
        "controller starting"
    );

    tokio::select! {
        r = grpc => r?,
        r = http_task => r?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown requested");
        }
    }
    Ok(())
}
