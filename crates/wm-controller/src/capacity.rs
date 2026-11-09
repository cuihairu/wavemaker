//! Resource capacity descriptions (WM-0503) — what a Worker or Backend
//! DECLARES about its own resources at registration time
//! (`docs/distributed.md` §Worker/Backend Registration).
//!
//! Honesty rules:
//!
//! - declared, never measured: the controller records what the Worker
//!   says about itself, verbatim, and interprets nothing (the same
//!   discipline as ADR-007 params and the Backend weight). Plan
//!   feasibility is WM-0504's job, not the description's.
//! - absent ≠ zero: `None` means "never declared" — an unknown, which
//!   reports and planners must treat as unknown. `Some(0)` means the
//!   Worker explicitly claims it can run nothing of that kind. The two
//!   must never be conflated, so every field is an `Option` and there
//!   are no default values that could fake a declaration.
//! - capacity is re-declarable: a Worker under memory pressure may
//!   send a fresh description; consumers read only the latest one.

use std::collections::BTreeMap;

/// A Worker's or Backend's self-declared resource envelope.
///
/// Every field is optional and independent: a Worker that only knows
/// its bot slot count declares exactly that and nothing else.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ResourceCapacity {
    /// Native runtime: concurrent actor executions this Worker claims
    /// it can host (Mode C).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_slots: Option<u32>,
    /// Engine Bot: concurrent engine-native bots this Worker claims it
    /// can drive (Mode A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_slots: Option<u32>,
    /// Engine sessions this Worker claims it can hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sessions: Option<u32>,
    /// Declared memory envelope, MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u64>,
    /// Declared CPU envelope, logical cores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_cores: Option<f64>,
    /// Extra declared dimensions, verbatim — the controller does not
    /// interpret keys or values (e.g. `{"nic": "10gbe", "zone": "a"}`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl ResourceCapacity {
    /// `true` when nothing at all was declared. This is the report's
    /// "not declared" state — distinct from a declaration of zeros,
    /// which is a real (if useless) envelope.
    pub fn is_undeclared(&self) -> bool {
        self.actor_slots.is_none()
            && self.bot_slots.is_none()
            && self.max_sessions.is_none()
            && self.memory_mib.is_none()
            && self.cpu_cores.is_none()
            && self.extra.is_empty()
    }

    /// The slot count relevant to a backend kind — `None` when the
    /// Worker declared nothing for that kind. Callers must handle the
    /// unknown case themselves; this helper invents no numbers.
    pub fn slots_for(&self, kind: crate::backend_registry::BackendKind) -> Option<u32> {
        match kind {
            crate::backend_registry::BackendKind::NativeRuntime => self.actor_slots,
            crate::backend_registry::BackendKind::EngineBot => self.bot_slots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_registry::BackendKind;

    #[test]
    fn undeclared_and_explicit_zero_are_distinguishable() {
        let undeclared = ResourceCapacity::default();
        assert!(undeclared.is_undeclared());
        let explicit_zero = ResourceCapacity {
            actor_slots: Some(0),
            ..Default::default()
        };
        assert!(!explicit_zero.is_undeclared());
        assert_eq!(explicit_zero.actor_slots, Some(0));
    }

    #[test]
    fn partial_declaration_declares_exactly_what_it_knows() {
        let capacity = ResourceCapacity {
            bot_slots: Some(500),
            ..Default::default()
        };
        assert!(capacity.actor_slots.is_none());
        assert!(capacity.memory_mib.is_none());
        assert_eq!(capacity.slots_for(BackendKind::EngineBot), Some(500));
        assert_eq!(
            capacity.slots_for(BackendKind::NativeRuntime),
            None,
            "undeclared dimension stays unknown, never defaulted"
        );
    }

    #[test]
    fn serde_roundtrip_omits_absent_fields_and_keeps_extra_verbatim() {
        let capacity = ResourceCapacity {
            bot_slots: Some(500),
            extra: BTreeMap::from([("nic".into(), "10gbe".into())]),
            ..Default::default()
        };
        let text = serde_json::to_string(&capacity).expect("serialize");
        assert!(!text.contains("actor_slots"), "absent fields are omitted");
        assert!(!text.contains("cpu_cores"));
        assert!(text.contains("\"bot_slots\":500"));
        assert!(text.contains("\"nic\":\"10gbe\""));
        let back: ResourceCapacity = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, capacity);
        // An entirely empty object deserializes to the undeclared state.
        let empty: ResourceCapacity = serde_json::from_str("{}").expect("empty");
        assert!(empty.is_undeclared());
    }
}
