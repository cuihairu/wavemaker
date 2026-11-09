//! Wavemaker controller (WM-0104): Axum control-plane skeleton plus the
//! `wavemaker.control.v1` gRPC server that workers register against.

pub mod aggregation;
pub mod backend_registry;
pub mod capacity;
pub mod config;
pub mod control;
pub mod http;
pub mod mixed_swell;
pub mod persistence;
pub mod recovery;
pub mod registry;
pub mod server;
pub mod shard_plan;
pub mod state;

pub use aggregation::{
    aggregate_metrics, AggregationError, AggregationIssue, BackendAggregate,
    MetricsAggregation, WorkerMetrics,
};
pub use backend_registry::{
    BackendKind, BackendRecord, BackendRegistry, BackendRegistryError, BackendSnapshot,
    BackendState, BackendTransition,
};
pub use capacity::ResourceCapacity;
pub use mixed_swell::{
    build_mixed_swell_report, verify_mixed_plan_conformance, KindGroupReport,
    MixedReportError, MixedSwellReport, PlanConformanceError,
};
pub use persistence::{
    load_snapshot, restore_controller, save_snapshot, ControllerSnapshot, PersistenceError,
};
pub use recovery::{PlanAuthority, RecoveryDivergence, RecoverySync, RecoveryView};
pub use shard_plan::{
    MovedEntry, PlanBackend, PlanEntry, PlanError, PlannedArrival, ReplanRecord,
    ShardPlan, ShardRule,
};
pub use config::ControllerConfig;
pub use registry::{
    AdmissionVerdict, RegistrationDecision, RegistryError, WorkerRecord,
    WorkerRegistration, WorkerRegistry, WorkerSnapshot, WorkerState,
};
pub use server::run;
pub use state::ControllerState;
