//! Protobuf v1 contracts and generated types for the Wavemaker control plane.
//!
//! Packages follow docs/spec/protobuf-conventions.md (WM-0013):
//! `wavemaker.{common,control,backend,engine,metrics}.v1`. Breaking changes
//! require a new `v2` package; inside v1 only additive evolution is allowed.

/// Wire protocol version carried by every handshake/registration.
///
/// Incompatible versions must be rejected at handshake time — never
/// silently downgraded (protobuf-conventions §2.3).
pub const PROTOCOL_VERSION: u32 = 1;

/// Generated types for `wavemaker.*.v1`, nested to match the proto package
/// paths so cross-package references (`super::super::...`) resolve.
pub mod wavemaker {
    /// `wavemaker.common.v1`
    pub mod common {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/wavemaker.common.v1.rs"));
        }
    }

    /// `wavemaker.control.v1`
    pub mod control {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/wavemaker.control.v1.rs"));
        }
    }

    /// `wavemaker.backend.v1`
    pub mod backend {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/wavemaker.backend.v1.rs"));
        }
    }

    /// `wavemaker.engine.v1`
    pub mod engine {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/wavemaker.engine.v1.rs"));
        }
    }

    /// `wavemaker.metrics.v1`
    pub mod metrics {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/wavemaker.metrics.v1.rs"));
        }
    }
}

pub use wavemaker::{backend, common, control, engine, metrics};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_zero_is_unspecified() {
        // protobuf-conventions §1: explicit zero = UNSPECIFIED; real states
        // start at 1 (OK = 1).
        let code = common::v1::ErrorCode::default();
        assert_eq!(code, common::v1::ErrorCode::Unspecified);
        assert_ne!(code, common::v1::ErrorCode::Ok);
    }

    #[test]
    fn operation_result_defaults_to_unspecified() {
        let r = common::v1::OperationResult::default();
        assert_eq!(r.code(), common::v1::ErrorCode::Unspecified);
        assert!(r.reason_code.is_empty());
    }

    #[test]
    fn ok_result_carries_reason_code() {
        let r = common::v1::OperationResult {
            code: common::v1::ErrorCode::Ok as i32,
            reason_code: "ok".into(),
            detail: String::new(),
            generation: 0,
        };
        assert_eq!(r.code(), common::v1::ErrorCode::Ok);
    }

    #[test]
    fn control_service_client_and_server_exist() {
        // The generated control plane must expose both roles: the controller
        // hosts the server, workers use the client (control_service.proto).
        fn assert_service<T: control::v1::control_service_server::ControlService>() {}
        assert_service::<NoControl>();
        let server = control::v1::control_service_server::ControlServiceServer::new(NoControl);
        let _: control::v1::control_service_server::ControlServiceServer<NoControl> = server;
        let client_ctor: fn(
            tonic::transport::Channel,
        ) -> control::v1::control_service_client::ControlServiceClient<
            tonic::transport::Channel,
        > = control::v1::control_service_client::ControlServiceClient::new;
        let _ = client_ctor;
    }

    /// Minimal server stub used only to satisfy the trait bound check above.
    pub struct NoControl;

    #[tonic::async_trait]
    impl control::v1::control_service_server::ControlService for NoControl {
        async fn register_worker(
            &self,
            _request: tonic::Request<control::v1::RegisterWorkerRequest>,
        ) -> Result<tonic::Response<control::v1::RegisterWorkerResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("stub"))
        }

        async fn heartbeat(
            &self,
            _request: tonic::Request<control::v1::HeartbeatRequest>,
        ) -> Result<tonic::Response<control::v1::HeartbeatResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("stub"))
        }
    }

    #[test]
    fn protocol_version_is_frozen_at_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}
