//! Wavemaker worker (WM-0105): gRPC client that registers with the
//! controller and sends heartbeats until shutdown.
//!
//! Role convention (proto/control/v1/control_service.proto): the controller
//! hosts `ControlService`; the worker is the client.

use anyhow::{bail, Context};
use std::time::Duration;
use tokio::sync::watch;
use tonic::transport::Channel;
use wm_protocol::common::v1::ErrorCode;
use wm_protocol::control::v1::control_service_client::ControlServiceClient;
use wm_protocol::control::v1::{HeartbeatRequest, RegisterWorkerRequest, WorkerLoad};
use wm_protocol::PROTOCOL_VERSION;

/// Worker identity and connection options.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Controller control-plane endpoint, e.g. `http://127.0.0.1:7601`.
    pub controller_endpoint: String,
    pub worker_id: String,
    pub capabilities: Vec<String>,
    pub adapter_version: String,
    pub runtime_version: String,
    /// Requested heartbeat period; the controller's granted interval from
    /// registration wins if smaller.
    pub heartbeat_interval_ms: u64,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            controller_endpoint: "http://127.0.0.1:7601".into(),
            worker_id: "worker-0".into(),
            capabilities: vec!["native".into()],
            adapter_version: env!("CARGO_PKG_VERSION").into(),
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            heartbeat_interval_ms: 5000,
        }
    }
}

/// Connected worker session.
pub struct WorkerSession {
    client: ControlServiceClient<Channel>,
    worker_id: String,
    generation: u64,
    interval: Duration,
}

impl WorkerSession {
    /// Connect and register; refuses to start on protocol-version mismatch
    /// or registration failure (WM-0013 §2.3: no silent degradation).
    pub async fn connect(options: &WorkerOptions) -> anyhow::Result<Self> {
        let client = ControlServiceClient::connect(options.controller_endpoint.clone())
            .await
            .with_context(|| {
                format!(
                    "connecting to controller at {}",
                    options.controller_endpoint
                )
            })?;

        let mut client = client;
        let resp = client
            .register_worker(RegisterWorkerRequest {
                worker_id: options.worker_id.clone(),
                protocol_version: PROTOCOL_VERSION,
                adapter_version: options.adapter_version.clone(),
                runtime_version: options.runtime_version.clone(),
                capabilities: options.capabilities.clone(),
            })
            .await?
            .into_inner();

        let result = resp
            .result
            .context("registration response missing result")?;
        if result.code() != ErrorCode::Ok {
            bail!(
                "registration refused: code={:?} reason={} detail={}",
                result.code(),
                result.reason_code,
                result.detail
            );
        }

        let granted = u64::from(resp.heartbeat_interval_ms).min(options.heartbeat_interval_ms);
        tracing::info!(
            worker_id = %options.worker_id,
            generation = resp.generation,
            heartbeat_interval_ms = granted,
            "worker registered"
        );

        Ok(Self {
            client,
            worker_id: options.worker_id.clone(),
            generation: resp.generation,
            interval: Duration::from_millis(granted),
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Send one heartbeat. Returns false when the controller no longer
    /// accepts this worker (lease expired / stale generation).
    pub async fn heartbeat(&mut self, load: WorkerLoad) -> anyhow::Result<bool> {
        let resp = self
            .client
            .heartbeat(HeartbeatRequest {
                worker_id: self.worker_id.clone(),
                generation: self.generation,
                load: Some(load),
            })
            .await?
            .into_inner();
        let result = resp.result.context("heartbeat response missing result")?;
        if result.code() != ErrorCode::Ok && result.code() != ErrorCode::NotFound {
            tracing::warn!(
                code = ?result.code(),
                reason = %result.reason_code,
                "heartbeat rejected"
            );
        }
        Ok(resp.accepted)
    }
}

/// Run the heartbeat loop until `shutdown` flips to true (WM-0105 stub).
pub async fn run(
    options: WorkerOptions,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let worker_id = options.worker_id.clone();
    let mut session = WorkerSession::connect(&options).await?;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(session.interval()) => {
                let accepted = session.heartbeat(WorkerLoad::default()).await?;
                if !accepted {
                    tracing::warn!("heartbeat not accepted; controller may have dropped the lease");
                }
            }
            _ = shutdown.changed() => {
                tracing::info!(worker_id, "worker shutting down");
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use wm_protocol::control::v1::control_service_server::ControlServiceServer;

    // The worker crate has no dependency on wm-controller; the test spins a
    // minimal in-crate ControlService server instead.
    mod wm_controller_shim {
        use std::sync::Mutex;
        use tonic::{Request, Response, Status};
        use wm_protocol::common::v1::{ErrorCode, OperationResult};
        use wm_protocol::control::v1::control_service_server::ControlService;
        use wm_protocol::control::v1::{
            HeartbeatRequest, HeartbeatResponse, RegisterWorkerRequest, RegisterWorkerResponse,
        };
        use wm_protocol::PROTOCOL_VERSION;

        #[derive(Default)]
        pub struct Registry {
            pub registered: Mutex<Vec<String>>,
        }

        pub struct TestControl {
            pub registry: std::sync::Arc<Registry>,
        }

        fn ok(generation: u64) -> OperationResult {
            OperationResult {
                code: ErrorCode::Ok as i32,
                reason_code: "ok".into(),
                detail: String::new(),
                generation,
            }
        }

        #[tonic::async_trait]
        impl ControlService for TestControl {
            async fn register_worker(
                &self,
                request: Request<RegisterWorkerRequest>,
            ) -> Result<Response<RegisterWorkerResponse>, Status> {
                let req = request.into_inner();
                if req.protocol_version != PROTOCOL_VERSION {
                    return Ok(Response::new(RegisterWorkerResponse {
                        result: Some(OperationResult {
                            code: ErrorCode::Unsupported as i32,
                            reason_code: "protocol_version_mismatch".into(),
                            detail: String::new(),
                            generation: 0,
                        }),
                        generation: 0,
                        heartbeat_interval_ms: 0,
                    }));
                }
                self.registry
                    .registered
                    .lock()
                    .unwrap()
                    .push(req.worker_id.clone());
                Ok(Response::new(RegisterWorkerResponse {
                    result: Some(ok(1)),
                    generation: 1,
                    heartbeat_interval_ms: 50,
                }))
            }

            async fn heartbeat(
                &self,
                _request: Request<HeartbeatRequest>,
            ) -> Result<Response<HeartbeatResponse>, Status> {
                Ok(Response::new(HeartbeatResponse {
                    result: Some(ok(1)),
                    accepted: true,
                }))
            }
        }
    }

    #[tokio::test]
    async fn worker_registers_and_runs_until_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let registry = std::sync::Arc::new(wm_controller_shim::Registry::default());

        let server = tonic::transport::Server::builder()
            .add_service(ControlServiceServer::new(wm_controller_shim::TestControl {
                registry: std::sync::Arc::clone(&registry),
            }))
            .serve_with_incoming(TcpListenerStream::new(listener));
        let server_handle = tokio::spawn(server);

        let options = WorkerOptions {
            controller_endpoint: format!("http://{addr}"),
            worker_id: "worker-test".into(),
            heartbeat_interval_ms: 50,
            ..WorkerOptions::default()
        };

        let (tx, rx) = watch::channel(false);
        let handle = tokio::spawn(run(options, rx));
        tokio::time::sleep(Duration::from_millis(180)).await;
        tx.send(true).unwrap();
        handle.await.unwrap().unwrap();

        assert!(registry
            .registered
            .lock()
            .unwrap()
            .contains(&"worker-test".to_string()));
        server_handle.abort();
    }

    #[tokio::test]
    async fn protocol_mismatch_refuses_to_start() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        struct Rejecting;
        use tonic::{Request, Response, Status};
        use wm_protocol::control::v1::RegisterWorkerResponse;

        #[tonic::async_trait]
        impl wm_protocol::control::v1::control_service_server::ControlService for Rejecting {
            async fn register_worker(
                &self,
                _request: Request<RegisterWorkerRequest>,
            ) -> Result<Response<RegisterWorkerResponse>, Status> {
                Ok(Response::new(RegisterWorkerResponse {
                    result: Some(wm_protocol::common::v1::OperationResult {
                        code: ErrorCode::Unsupported as i32,
                        reason_code: "protocol_version_mismatch".into(),
                        detail: "old controller".into(),
                        generation: 0,
                    }),
                    generation: 0,
                    heartbeat_interval_ms: 0,
                }))
            }

            async fn heartbeat(
                &self,
                _request: Request<HeartbeatRequest>,
            ) -> Result<Response<wm_protocol::control::v1::HeartbeatResponse>, Status> {
                unimplemented!()
            }
        }

        let server = tonic::transport::Server::builder()
            .add_service(ControlServiceServer::new(Rejecting))
            .serve_with_incoming(TcpListenerStream::new(listener));
        let server_handle = tokio::spawn(server);

        let options = WorkerOptions {
            controller_endpoint: format!("http://{addr}"),
            ..WorkerOptions::default()
        };
        let err = match WorkerSession::connect(&options).await {
            Err(e) => e,
            Ok(_) => panic!("expected registration refusal"),
        };
        assert!(err.to_string().contains("registration refused"));

        server_handle.abort();
    }
}
