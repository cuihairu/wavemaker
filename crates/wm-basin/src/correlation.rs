//! Correlation context (docs/basin.md tree: Basin -> Correlation Context).
//!
//! Carries what later report items need to line up generator, server, and
//! experiment data. WM-0401 fixes the identifiers; WM-0414 (timeline
//! alignment) and WM-0416 (correlation reports) extend the semantics.

use serde::{Deserialize, Serialize};

use crate::BasinError;

/// The swell this Basin instance serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrelationContext {
    pub swell_id: String,
    /// The swell's wall-clock window, fixed once the swell actually runs
    /// (None while planning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRange>,
}

impl CorrelationContext {
    pub fn validate(&self) -> Result<(), BasinError> {
        if let Some(range) = &self.time_range {
            range.validate()?;
        }
        Ok(())
    }
}

/// A wall-clock window in Unix milliseconds. i64 keeps room for dates far
/// beyond the project's lifetime and matches the proto timestamp model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

impl TimeRange {
    pub fn validate(&self) -> Result<(), BasinError> {
        if self.start_unix_ms > self.end_unix_ms {
            return Err(BasinError::InvertedTimeRange(
                self.start_unix_ms,
                self.end_unix_ms,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_range_rejects_inverted_windows() {
        let ok = TimeRange {
            start_unix_ms: 100,
            end_unix_ms: 200,
        };
        assert_eq!(ok.validate(), Ok(()));
        let inverted = TimeRange {
            start_unix_ms: 200,
            end_unix_ms: 100,
        };
        assert_eq!(
            inverted.validate(),
            Err(BasinError::InvertedTimeRange(200, 100))
        );
    }

    #[test]
    fn correlation_without_time_range_is_valid_and_compact() {
        let context = CorrelationContext {
            swell_id: "sw-1".into(),
            time_range: None,
        };
        assert_eq!(context.validate(), Ok(()));
        let text = serde_yaml::to_string(&context).expect("serialise");
        assert!(!text.contains("time_range"), "None stays off the wire");
    }

    #[test]
    fn correlation_with_time_range_round_trips() {
        let context = CorrelationContext {
            swell_id: "sw-1".into(),
            time_range: Some(TimeRange {
                start_unix_ms: 1_000,
                end_unix_ms: 2_000,
            }),
        };
        let text = serde_yaml::to_string(&context).expect("serialise");
        let back: CorrelationContext = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, context);
    }
}
