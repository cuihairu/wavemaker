//! Recovery handshake and state synchronization (WM-0509) —
//! distributed.md lost-contact policy 5: after recovery, re-handshake
//! and synchronize state.
//!
//! A Backend that recovered from lost contact has, by definition, a
//! STALE view: it missed commands and epoch bumps while it was gone.
//! The handshake is therefore not a formality — it is a field-by-field
//! comparison of the recovering Backend's view against the
//! controller's authoritative record:
//!
//! - every mismatch is a NAMED divergence carrying both views; the
//!   controller never assumes a recovered backend is fine;
//! - the handshake compares only — it never mutates the registry.
//!   Actual state changes still flow through `report_transition`,
//!   whose generation guard (WM-0508) admits the backend's reports
//!   once it has ADOPTED the authoritative epoch;
//! - the sync output carries the authoritative values, so "adopt the
//!   controller's view, then report at that epoch" is a complete,
//!   honest round trip (tested end to end).

use crate::backend_registry::{BackendRegistry, BackendRegistryError, BackendState};

/// What a recovering Backend reports about itself at the handshake.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryView {
    /// The epoch the Backend last observed before losing contact.
    pub observed_generation: u64,
    /// The state the Backend believes it is in.
    pub observed_state: BackendState,
    /// The swell the Backend believes it is bound to (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swell_binding: Option<String>,
    /// The plan digest the Backend last accepted (if it holds a plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_digest: Option<String>,
}

/// One named field-level mismatch. Both views travel: the Backend can
/// reconcile from the record itself, no guessing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryDivergence {
    pub field: String,
    /// What the recovering Backend believed (verbatim).
    pub backend_view: String,
    /// What the controller's authoritative record says (verbatim).
    pub controller_view: String,
}

/// The handshake result. `accepted` only when every compared field
/// agrees; otherwise `divergences` lists every mismatch and the
/// authoritative values name what the Backend must adopt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoverySync {
    pub backend_id: String,
    pub accepted: bool,
    pub authoritative_generation: u64,
    pub authoritative_state: BackendState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoritative_swell_binding: Option<String>,
    /// The controller's authoritative plan digest for the swell, when
    /// the caller supplied one (the controller owns the plans).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoritative_plan_digest: Option<String>,
    pub divergences: Vec<RecoveryDivergence>,
}

/// What the controller holds as authority for the plan digest — the
/// registry stores transitions, the PLAN lives above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanAuthority<'a> {
    pub plan_digest: Option<&'a str>,
}

fn last_swell_binding(record: &crate::backend_registry::BackendRecord) -> Option<String> {
    record
        .transitions
        .iter()
        .rev()
        .find_map(|transition| transition.swell_id.clone())
}

impl BackendRegistry {
    /// Compare a recovering Backend's view with the authoritative
    /// record. Read-only: nothing is mutated, accepted or not — after
    /// reconciliation the Backend reports transitions at the
    /// authoritative epoch through `report_transition` like everyone
    /// else.
    pub fn recovery_handshake(
        &self,
        backend_id: &str,
        view: RecoveryView,
        plan_authority: PlanAuthority<'_>,
    ) -> Result<RecoverySync, BackendRegistryError> {
        let record = self
            .backend(backend_id)
            .ok_or_else(|| BackendRegistryError::UnknownBackend(backend_id.into()))?;
        let authoritative_swell = last_swell_binding(&record);
        let mut divergences = Vec::new();
        if view.observed_generation != record.generation {
            divergences.push(RecoveryDivergence {
                field: "generation".into(),
                backend_view: view.observed_generation.to_string(),
                controller_view: record.generation.to_string(),
            });
        }
        if view.observed_state != record.state {
            divergences.push(RecoveryDivergence {
                field: "state".into(),
                backend_view: format!("{:?}", view.observed_state),
                controller_view: format!("{:?}", record.state),
            });
        }
        if view.swell_binding != authoritative_swell {
            divergences.push(RecoveryDivergence {
                field: "swell_binding".into(),
                backend_view: format!("{:?}", view.swell_binding),
                controller_view: format!("{:?}", authoritative_swell),
            });
        }
        if view.plan_digest != plan_authority.plan_digest.map(str::to_string) {
            divergences.push(RecoveryDivergence {
                field: "plan_digest".into(),
                backend_view: format!("{:?}", view.plan_digest),
                controller_view: format!("{:?}", plan_authority.plan_digest),
            });
        }
        Ok(RecoverySync {
            backend_id: backend_id.into(),
            accepted: divergences.is_empty(),
            authoritative_generation: record.generation,
            authoritative_state: record.state,
            authoritative_swell_binding: authoritative_swell,
            authoritative_plan_digest: plan_authority.plan_digest.map(str::to_string),
            divergences,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_registry::BackendKind;

    fn registry_with_backend_in_running() -> BackendRegistry {
        let registry = BackendRegistry::new();
        registry
            .register("kb-conf", BackendKind::EngineBot, 1.0)
            .expect("register");
        let steps = [
            (BackendState::Handshaking, "handshake-ok"),
            (BackendState::Ready, "handshake-complete"),
            (BackendState::Initialized, "init-ok"),
            (BackendState::PlanAccepted, "plan-applied"),
            (BackendState::Running, "start-ok"),
        ];
        for (state, reason) in steps {
            let generation = registry.backend("kb-conf").expect("record").generation;
            registry
                .report_transition(
                    "kb-conf",
                    generation,
                    state,
                    Some("sw-1".into()),
                    reason,
                    "",
                )
                .expect("step");
        }
        registry
    }

    fn matching_view(registry: &BackendRegistry) -> RecoveryView {
        let record = registry.backend("kb-conf").expect("record");
        RecoveryView {
            observed_generation: record.generation,
            observed_state: record.state,
            swell_binding: Some("sw-1".into()),
            plan_digest: Some("plan-digest-7".into()),
        }
    }

    #[test]
    fn fully_agreeing_views_are_accepted_without_divergence() {
        let registry = registry_with_backend_in_running();
        let view = matching_view(&registry);
        let sync = registry
            .recovery_handshake(
                "kb-conf",
                view,
                PlanAuthority {
                    plan_digest: Some("plan-digest-7"),
                },
            )
            .expect("known backend");
        assert!(sync.accepted);
        assert!(sync.divergences.is_empty());
        assert_eq!(sync.authoritative_state, BackendState::Running);
        assert_eq!(sync.authoritative_swell_binding.as_deref(), Some("sw-1"));
    }

    #[test]
    fn every_divergence_is_named_with_both_views_and_mutates_nothing() {
        let registry = registry_with_backend_in_running();
        // The controller moved on while the backend was gone:
        // admission stopped + draining (epochs 7, 8).
        for (state, reason) in [
            (BackendState::AdmissionStopped, "lost-contact-freeze"),
            (BackendState::Draining, "drain-policy"),
        ] {
            let generation = registry.backend("kb-conf").expect("record").generation;
            registry
                .report_transition(
                    "kb-conf",
                    generation,
                    state,
                    Some("sw-1".into()),
                    reason,
                    "",
                )
                .expect("controller-recorded step");
        }
        let before = registry.backend("kb-conf").expect("record");
        // The backend missed everything: it believes it is still
        // Running at epoch 4, bound to another swell, on an old plan.
        let view = RecoveryView {
            observed_generation: 4,
            observed_state: BackendState::Running,
            swell_binding: Some("sw-9".into()),
            plan_digest: Some("old-plan-digest".into()),
        };
        let sync = registry
            .recovery_handshake(
                "kb-conf",
                view,
                PlanAuthority {
                    plan_digest: Some("plan-digest-7"),
                },
            )
            .expect("known backend");
        assert!(!sync.accepted);
        let fields: Vec<&str> = sync
            .divergences
            .iter()
            .map(|divergence| divergence.field.as_str())
            .collect();
        assert_eq!(fields, vec!["generation", "state", "swell_binding", "plan_digest"]);
        let generation_divergence = &sync.divergences[0];
        assert_eq!(generation_divergence.backend_view, "4");
        assert_eq!(generation_divergence.controller_view, "8");
        // The sync names what the backend must adopt.
        assert_eq!(sync.authoritative_generation, 8);
        assert_eq!(sync.authoritative_state, BackendState::Draining);
        assert_eq!(sync.authoritative_swell_binding.as_deref(), Some("sw-1"));
        assert_eq!(
            sync.authoritative_plan_digest.as_deref(),
            Some("plan-digest-7")
        );
        // The handshake is read-only: a rejected handshake leaves zero
        // trace — reconciliation is explicit, never a side effect.
        let after = registry.backend("kb-conf").expect("record");
        assert_eq!(before, after);
    }

    #[test]
    fn adopt_authority_then_report_at_that_epoch_completes_the_round_trip() {
        let registry = registry_with_backend_in_running();
        // ... the backend loses contact; the controller stops
        // admission and drains (epoch 6, state Draining).
        for (state, reason) in [
            (BackendState::AdmissionStopped, "lost-contact-freeze"),
            (BackendState::Draining, "drain-policy"),
        ] {
            let generation = registry.backend("kb-conf").expect("record").generation;
            registry
                .report_transition(
                    "kb-conf",
                    generation,
                    state,
                    Some("sw-1".into()),
                    reason,
                    "",
                )
                .expect("controller-recorded step");
        }
        // The recovering backend shows up with its stale view.
        let stale_view = RecoveryView {
            observed_generation: 5,
            observed_state: BackendState::Running,
            swell_binding: Some("sw-1".into()),
            plan_digest: Some("plan-digest-7".into()),
        };
        let sync = registry
            .recovery_handshake(
                "kb-conf",
                stale_view,
                PlanAuthority {
                    plan_digest: Some("plan-digest-7"),
                },
            )
            .expect("known backend");
        assert!(!sync.accepted);
        // The backend ADOPTS the authoritative view...
        let adopted = RecoveryView {
            observed_generation: sync.authoritative_generation,
            observed_state: sync.authoritative_state,
            swell_binding: sync.authoritative_swell_binding,
            plan_digest: sync.authoritative_plan_digest,
        };
        let resync = registry
            .recovery_handshake(
                "kb-conf",
                adopted,
                PlanAuthority {
                    plan_digest: Some("plan-digest-7"),
                },
            )
            .expect("known backend");
        assert!(resync.accepted, "adopting authority closes every divergence");
        // ...and reports further transitions AT the authoritative
        // epoch — exactly what the generation guard admits.
        let generation = registry.backend("kb-conf").expect("record").generation;
        let applied = registry
            .report_transition(
                "kb-conf",
                generation,
                BackendState::Stopped,
                Some("sw-1".into()),
                "drained",
                "",
            )
            .expect("post-resync report applies");
        assert_eq!(applied.generation, 9);
    }

    #[test]
    fn unknown_backend_handshake_is_refused() {
        let registry = BackendRegistry::new();
        let view = RecoveryView {
            observed_generation: 1,
            observed_state: BackendState::Created,
            swell_binding: None,
            plan_digest: None,
        };
        assert!(matches!(
            registry.recovery_handshake("ghost", view, PlanAuthority { plan_digest: None }),
            Err(BackendRegistryError::UnknownBackend(_))
        ));
    }
}
