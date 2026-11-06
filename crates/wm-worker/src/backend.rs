//! Native Backend service (WM-0106): capability handshake and the
//! plan/start/stop operations, driven by the wm-core Backend state machine
//! (WM-0109) and the `wavemaker.backend.v1` contract.
//!
//! Rules enforced here (docs/backends.md, docs/spec/protobuf-conventions.md):
//! - incompatible protocol versions are refused at handshake, never
//!   silently downgraded;
//! - ApplyPlan is idempotent by `plan_digest`; Start/Stop are idempotent by
//!   `operation_id`;
//! - every request carrying a stale generation is rejected
//!   (FAILED_PRECONDITION) — late events never overwrite newer state;
//! - illegal state transitions return FAILED_PRECONDITION with the state
//!   machine's reason code.

use std::sync::Mutex;

use tonic::{Request, Response, Status};
use wm_core::backend::{BackendState, BackendStateMachine, StateMachineError};
use wm_core::Generation;
use wm_protocol::backend::v1::backend_service_server::BackendService;
use wm_protocol::backend::v1::{
    ApplyPlanRequest, ApplyPlanResponse, HandshakeRequest, HandshakeResponse, StartRequest,
    StartResponse, StopRequest, StopResponse,
};
use wm_protocol::common::v1::{ErrorCode, OperationResult};
use wm_protocol::PROTOCOL_VERSION;

/// Mutable backend core guarded by the service.
struct BackendCore {
    machine: BackendStateMachine,
    /// Digest of the currently accepted plan (idempotency key).
    plan_digest: Option<String>,
    /// operation_ids already completed, for idempotent replays.
    completed_operations: Vec<String>,
}

/// Native execution backend service. Phase 1 advertises the `native`
/// capability; engine-bot capabilities arrive with Phase 2.
pub struct NativeBackendService {
    capabilities: Vec<String>,
    core: Mutex<BackendCore>,
}

fn result_ok(generation: Generation) -> OperationResult {
    OperationResult {
        code: ErrorCode::Ok as i32,
        reason_code: "ok".into(),
        detail: String::new(),
        generation: generation.0,
    }
}

fn result_failed_precondition(
    reason: &str,
    detail: String,
    generation: Generation,
) -> OperationResult {
    OperationResult {
        code: ErrorCode::FailedPrecondition as i32,
        reason_code: reason.into(),
        detail,
        generation: generation.0,
    }
}

fn result_unsupported(reason: &str, detail: String) -> OperationResult {
    OperationResult {
        code: ErrorCode::Unsupported as i32,
        reason_code: reason.into(),
        detail,
        generation: 0,
    }
}

fn state_error_to_result(err: &StateMachineError, generation: Generation) -> OperationResult {
    match err {
        StateMachineError::InvalidTransition { from, to } => result_failed_precondition(
            "invalid_state_transition",
            format!("backend in state {from:?} cannot transition to {to:?}"),
            generation,
        ),
        StateMachineError::StaleGeneration { got, current } => result_failed_precondition(
            "stale_generation",
            format!("request generation {got} is behind current {current}"),
            generation,
        ),
    }
}

impl NativeBackendService {
    /// Create a backend in the `Created` state with the given capabilities.
    pub fn new(capabilities: Vec<String>) -> Self {
        Self {
            core: Mutex::new(BackendCore {
                machine: BackendStateMachine::new("native-backend"),
                plan_digest: None,
                completed_operations: Vec::new(),
            }),
            capabilities,
        }
    }

    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// Current machine state and generation (for tests and diagnostics).
    pub fn snapshot(&self) -> (BackendState, Generation) {
        let core = self.core.lock().unwrap();
        (core.machine.state(), core.machine.generation())
    }
}

#[tonic::async_trait]
impl BackendService for NativeBackendService {
    async fn handshake(
        &self,
        request: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        let req = request.into_inner();

        if req.protocol_version != PROTOCOL_VERSION {
            return Ok(Response::new(HandshakeResponse {
                result: Some(result_unsupported(
                    "protocol_version_mismatch",
                    format!(
                        "backend speaks protocol v{PROTOCOL_VERSION}; controller offered v{}",
                        req.protocol_version
                    ),
                )),
                generation: 0,
                capabilities: Vec::new(),
            }));
        }

        let mut core = self.core.lock().unwrap();

        // Handshake walks Created/Degraded/Recovering → Handshaking → Ready.
        // Any other state (e.g. Stopped, PlanAccepted) has no legal path back
        // to Handshaking in the docs/backends.md table, so it is refused.
        if core.machine.state() != BackendState::Handshaking {
            let g = core.machine.generation();
            if let Err(e) = core.machine.transition(
                g,
                BackendState::Handshaking,
                "handshake_requested",
                &req.backend_id,
            ) {
                let g = core.machine.generation();
                return Ok(Response::new(HandshakeResponse {
                    result: Some(state_error_to_result(&e, g)),
                    generation: g.0,
                    capabilities: Vec::new(),
                }));
            }
        }

        let gen = core.machine.generation();
        let record = core.machine.transition(
            gen,
            BackendState::Ready,
            "handshake_completed",
            &req.backend_id,
        );
        let generation = match record {
            Ok(rec) => rec.generation,
            Err(e) => {
                let g = core.machine.generation();
                return Ok(Response::new(HandshakeResponse {
                    result: Some(state_error_to_result(&e, g)),
                    generation: g.0,
                    capabilities: Vec::new(),
                }));
            }
        };

        tracing::info!(
            backend_id = %req.backend_id,
            adapter_version = %req.adapter_version,
            engine_version = %req.engine_version,
            generation = generation.0,
            "backend handshake completed"
        );

        Ok(Response::new(HandshakeResponse {
            result: Some(result_ok(generation)),
            generation: generation.0,
            capabilities: self.capabilities.clone(),
        }))
    }

    async fn apply_plan(
        &self,
        request: Request<ApplyPlanRequest>,
    ) -> Result<Response<ApplyPlanResponse>, Status> {
        let req = request.into_inner();
        let mut core = self.core.lock().unwrap();

        // Late events must never overwrite newer state: a request stamped
        // with an older generation is refused before anything else.
        let current = core.machine.generation();
        if req.generation < current.0 {
            return Ok(Response::new(ApplyPlanResponse {
                result: Some(result_failed_precondition(
                    "stale_generation",
                    format!(
                        "request generation {} is behind current {current}",
                        req.generation
                    ),
                    current,
                )),
            }));
        }
        let generation = current;

        // Idempotent replay: the same digest while already accepted is OK.
        if core.plan_digest.as_deref() == Some(req.plan_digest.as_str())
            && core.machine.state() == BackendState::PlanAccepted
        {
            return Ok(Response::new(ApplyPlanResponse {
                result: Some(result_ok(generation)),
            }));
        }

        // Ready -> Initialized -> PlanAccepted (docs/backends.md chain).
        for (to, reason) in [
            (BackendState::Initialized, "backend_initialized"),
            (BackendState::PlanAccepted, "plan_accepted"),
        ] {
            if let Err(e) = core
                .machine
                .transition(generation, to, reason, &req.plan_digest)
            {
                let g = core.machine.generation();
                return Ok(Response::new(ApplyPlanResponse {
                    result: Some(state_error_to_result(&e, g)),
                }));
            }
        }

        core.plan_digest = Some(req.plan_digest);
        tracing::info!(
            swell_id = %req.swell_id,
            generation = generation.0,
            "plan applied"
        );

        Ok(Response::new(ApplyPlanResponse {
            result: Some(result_ok(generation)),
        }))
    }

    async fn start(
        &self,
        request: Request<StartRequest>,
    ) -> Result<Response<StartResponse>, Status> {
        let req = request.into_inner();
        let mut core = self.core.lock().unwrap();

        let current = core.machine.generation();
        if req.generation < current.0 {
            return Ok(Response::new(StartResponse {
                result: Some(result_failed_precondition(
                    "stale_generation",
                    format!(
                        "request generation {} is behind current {current}",
                        req.generation
                    ),
                    current,
                )),
            }));
        }
        let generation = current;

        // Idempotent replay of a completed start.
        if core
            .completed_operations
            .iter()
            .any(|op| op == &req.operation_id)
        {
            let g = core.machine.generation();
            return Ok(Response::new(StartResponse {
                result: Some(result_ok(g)),
            }));
        }

        if let Err(e) = core.machine.transition(
            generation,
            BackendState::Running,
            "start_requested",
            &req.operation_id,
        ) {
            let g = core.machine.generation();
            return Ok(Response::new(StartResponse {
                result: Some(state_error_to_result(&e, g)),
            }));
        }

        core.completed_operations.push(req.operation_id.clone());
        tracing::info!(swell_id = %req.swell_id, operation_id = %req.operation_id, "backend started");

        let g = core.machine.generation();
        Ok(Response::new(StartResponse {
            result: Some(result_ok(g)),
        }))
    }

    async fn stop(&self, request: Request<StopRequest>) -> Result<Response<StopResponse>, Status> {
        let req = request.into_inner();
        let mut core = self.core.lock().unwrap();

        let current = core.machine.generation();
        if req.generation < current.0 {
            return Ok(Response::new(StopResponse {
                result: Some(result_failed_precondition(
                    "stale_generation",
                    format!(
                        "request generation {} is behind current {current}",
                        req.generation
                    ),
                    current,
                )),
            }));
        }
        let generation = current;

        // Idempotent replay of a completed stop.
        if core
            .completed_operations
            .iter()
            .any(|op| op == &req.operation_id)
        {
            let g = core.machine.generation();
            return Ok(Response::new(StopResponse {
                result: Some(result_ok(g)),
            }));
        }

        // Running/AdmissionStopped -> Draining -> Stopped.
        let steps: &[(BackendState, &str)] = match core.machine.state() {
            BackendState::Running => &[
                (BackendState::AdmissionStopped, "stop_requested"),
                (BackendState::Draining, "draining"),
            ],
            BackendState::AdmissionStopped => &[(BackendState::Draining, "draining")],
            BackendState::Draining => &[],
            _ => {
                let g = core.machine.generation();
                return Ok(Response::new(StopResponse {
                    result: Some(result_failed_precondition(
                        "invalid_state_transition",
                        format!(
                            "backend in state {:?} cannot be stopped",
                            core.machine.state()
                        ),
                        g,
                    )),
                }));
            }
        };

        for (to, reason) in steps {
            if let Err(e) = core
                .machine
                .transition(generation, *to, *reason, &req.operation_id)
            {
                let g = core.machine.generation();
                return Ok(Response::new(StopResponse {
                    result: Some(state_error_to_result(&e, g)),
                }));
            }
        }
        if let Err(e) = core.machine.transition(
            generation,
            BackendState::Stopped,
            "stopped",
            &req.operation_id,
        ) {
            let g = core.machine.generation();
            return Ok(Response::new(StopResponse {
                result: Some(state_error_to_result(&e, g)),
            }));
        }

        core.completed_operations.push(req.operation_id.clone());
        tracing::info!(
            swell_id = %req.swell_id,
            operation_id = %req.operation_id,
            grace_period_ms = req.grace_period_ms,
            "backend stopped"
        );

        let g = core.machine.generation();
        Ok(Response::new(StopResponse {
            result: Some(result_ok(g)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_starts_in_created_with_capabilities() {
        let svc = NativeBackendService::new(vec!["native".into()]);
        let (state, gen) = svc.snapshot();
        assert_eq!(state, BackendState::Created);
        assert_eq!(gen, Generation::INITIAL);
        assert_eq!(svc.capabilities(), ["native"]);
    }

    #[test]
    fn ok_results_carry_generation() {
        let r = result_ok(Generation(7));
        assert_eq!(r.code(), ErrorCode::Ok);
        assert_eq!(r.generation, 7);
    }
}
