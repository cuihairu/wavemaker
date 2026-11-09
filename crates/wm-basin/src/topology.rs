//! Service topology placeholder (WM-0402 owns the model).
//!
//! WM-0401 only guarantees the Basin root can carry and serialise the
//! topology container; the service/node model — how servers, gateways,
//! databases, and caches are declared and covered — lands with WM-0402
//! (docs/basin.md "Basin's Role" tree).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Topology {}
