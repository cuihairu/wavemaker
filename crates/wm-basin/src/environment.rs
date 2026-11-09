//! The environment under test (docs/basin.md — Basin "represents the
//! environment under test").

use serde::{Deserialize, Serialize};

use crate::Labels;

/// A named deployment the swells run against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Stable identifier (the configuration's `basin.ref`).
    pub id: String,
    /// Free-form deployment facts worth carrying into reports (region,
    /// engine version, cluster size). BTreeMap keeps report output stable.
    #[serde(default)]
    pub labels: Labels,
}
