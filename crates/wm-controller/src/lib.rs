//! Wavemaker controller (WM-0104): Axum control-plane skeleton plus the
//! `wavemaker.control.v1` gRPC server that workers register against.

pub mod backend_registry;
pub mod capacity;
pub mod config;
pub mod control;
pub mod http;
pub mod registry;
pub mod server;
pub mod state;

pub use backend_registry::{
    BackendKind, BackendRecord, BackendRegistry, BackendRegistryError, BackendState,
    BackendTransition,
};
pub use capacity::ResourceCapacity;
pub use config::ControllerConfig;
pub use registry::{
    RegistrationDecision, RegistryError, WorkerRecord, WorkerRegistration,
    WorkerRegistry, WorkerState,
};
pub use server::run;
pub use state::ControllerState;
