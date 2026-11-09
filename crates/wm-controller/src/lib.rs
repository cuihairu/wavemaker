//! Wavemaker controller (WM-0104): Axum control-plane skeleton plus the
//! `wavemaker.control.v1` gRPC server that workers register against.

pub mod config;
pub mod control;
pub mod http;
pub mod registry;
pub mod server;
pub mod state;

pub use config::ControllerConfig;
pub use registry::{
    RegistrationDecision, RegistryError, WorkerRecord, WorkerRegistration,
    WorkerRegistry, WorkerState,
};
pub use server::run;
pub use state::ControllerState;
