//! Core domain types shared across the Wavemaker crates.
//!
//! Phase 1 scope: identifiers, generation/epoch bookkeeping, the swell
//! phase model and spectrum config validation. Anything larger (engine
//! runtime, metrics) lives elsewhere.

pub mod backend;
pub mod hdr;
pub mod spectrum;
pub mod store;

use std::fmt;

/// Opaque swell identifier (plan §12: `swell-001` style).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SwellId(pub String);

/// Opaque worker identifier used in control-plane registration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct WorkerId(pub String);

/// Opaque execution backend identifier (plan §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BackendId(pub String);

/// Monotonic generation/epoch counter.
///
/// Every state event carries a generation; late events from an older
/// generation must never overwrite newer state (protobuf-conventions §2.4,
/// plan §4.1.1/§7.3/§12.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u64);

impl Generation {
    pub const INITIAL: Generation = Generation(1);

    pub fn next(self) -> Generation {
        Generation(self.0 + 1)
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gen-{}", self.0)
    }
}

impl fmt::Display for SwellId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for WorkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for BackendId {
    fn from(s: &str) -> Self {
        BackendId(s.to_string())
    }
}

impl From<String> for BackendId {
    fn from(s: String) -> Self {
        BackendId(s)
    }
}

impl From<&str> for SwellId {
    fn from(s: &str) -> Self {
        SwellId(s.to_string())
    }
}

impl From<String> for SwellId {
    fn from(s: String) -> Self {
        SwellId(s)
    }
}

/// Swell lifecycle phase (plan §13.1, minimal subset for the control plane).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwellPhase {
    /// Config accepted, not yet running.
    Planned,
    /// Arrivals are being admitted.
    Running,
    /// No new arrivals; existing bots draining.
    Draining,
    /// Finished, report available.
    Completed,
    /// Terminated by error or stop command.
    Aborted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_monotonic() {
        let g = Generation::INITIAL;
        assert_eq!(g.next(), Generation(2));
        assert!(g.next() > g);
    }

    #[test]
    fn ids_render_without_decoration() {
        assert_eq!(SwellId("swell-001".into()).to_string(), "swell-001");
        assert_eq!(WorkerId("worker-a".into()).to_string(), "worker-a");
    }

    #[test]
    fn swell_phase_serializes_snake_case() {
        let yaml = serde_yaml::to_string(&SwellPhase::Draining).unwrap();
        assert_eq!(yaml.trim(), "draining");
    }
}
