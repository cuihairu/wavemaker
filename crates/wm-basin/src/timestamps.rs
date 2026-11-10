//! Timestamp model (WM-0407; docs/basin.md "Timestamp Model").
//!
//! Keep at least: source_timestamp (when the source produced the sample),
//! observed_at (when Wavemaker considers it observed), collected_at (when
//! Wavemaker actually collected). Also record: query window, collection
//! interval, source latency, missing intervals, clock offset estimates,
//! source instance and labels.
//!
//! The doc's honesty rules are structural here: absolute NTP sync is NOT
//! assumed, so delay-style numbers are named for what they are (a raw age
//! by OUR clock, confounded by unknown offset) and a missing clock offset
//! estimate stays absent instead of being fabricated.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::provider::QueryWindow;
use crate::Labels;

/// The three-stamp triple per sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampRecord {
    /// When the source system produced the sample. `None` = the source
    /// gave no timestamp; absence is kept, not invented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_timestamp_unix_ms: Option<i64>,
    /// When Wavemaker considers the sample observed.
    pub observed_at_unix_ms: i64,
    /// When Wavemaker actually collected it.
    pub collected_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimestampError {
    #[error("observed_at {0} is after collected_at {1}")]
    ObservedAfterCollected(i64, i64),
    #[error("missing interval start {0} is after end {1}")]
    InvertedMissingInterval(i64, i64),
    #[error("clock offset method must not be empty")]
    EmptyOffsetMethod,
}

impl TimestampRecord {
    /// Derive the triple from a collected sample. In this codepath the
    /// sample is considered observed at the moment of collection — there
    /// is no separate observation step to claim a tighter time than we
    /// actually have.
    pub fn from_collection(
        source_timestamp_unix_ms: Option<i64>,
        collected_at_unix_ms: i64,
    ) -> Self {
        Self {
            source_timestamp_unix_ms,
            observed_at_unix_ms: collected_at_unix_ms,
            collected_at_unix_ms,
        }
    }

    pub fn validate(&self) -> Result<(), TimestampError> {
        if self.observed_at_unix_ms > self.collected_at_unix_ms {
            return Err(TimestampError::ObservedAfterCollected(
                self.observed_at_unix_ms,
                self.collected_at_unix_ms,
            ));
        }
        Ok(())
    }

    /// Raw delay between the source stamp and our collection clock. This
    /// is an AGE by OUR clock — it conflates true source latency with the
    /// unknown clock offset, so it must be reported under this name, not
    /// as "latency".
    pub fn source_age_ms(&self) -> Option<i64> {
        self.source_timestamp_unix_ms
            .map(|source| self.collected_at_unix_ms - source)
    }
}

/// A clock offset estimate and HOW it was obtained. The method is
/// mandatory: an offset without provenance is indistinguishable from a
/// made-up number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockOffsetEstimate {
    pub offset_ms: i64,
    /// e.g. "ntp", "probe-roundtrip", "shape-alignment". The doc warns
    /// shape alignment does not replace real timestamps — recording the
    /// method keeps that visible to every consumer.
    pub method: String,
}

/// One gap in expected coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingInterval {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

/// The "also record" block of the timestamp model, attached to a
/// collection batch.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub query_window: Option<QueryWindow>,
    /// Scrape / collection cadence the batch was planned with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_interval_ms: Option<i64>,
    /// Estimated clock offset, if (and only if) one was actually
    /// estimated; `None` stays absent on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_offset: Option<ClockOffsetEstimate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_intervals: Vec<MissingInterval>,
    /// The data source instance and labels as reported (provider raw).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_instance: Option<String>,
    #[serde(default, skip_serializing_if = "Labels::is_empty")]
    pub source_labels: Labels,
}

impl CollectionMetadata {
    pub fn validate(&self) -> Result<(), TimestampError> {
        if let Some(offset) = &self.clock_offset {
            if offset.method.trim().is_empty() {
                return Err(TimestampError::EmptyOffsetMethod);
            }
        }
        for interval in &self.missing_intervals {
            if interval.start_unix_ms > interval.end_unix_ms {
                return Err(TimestampError::InvertedMissingInterval(
                    interval.start_unix_ms,
                    interval.end_unix_ms,
                ));
            }
        }
        Ok(())
    }
}

/// Expected windows that produced no data, merged where adjacent —
/// WM-0408's gap detector consumes this primitive. `produced` holds the
/// (start, end) of windows that DID return data.
pub fn compute_missing_intervals(
    expected: &[QueryWindow],
    produced: &BTreeSet<(i64, i64)>,
) -> Vec<MissingInterval> {
    let mut missing: Vec<MissingInterval> = Vec::new();
    for window in expected {
        if !produced.contains(&(window.start_unix_ms, window.end_unix_ms)) {
            // Adjacent expected windows merge instead of reporting one gap
            // per tick.
            if let Some(last) = missing.last_mut() {
                if last.end_unix_ms == window.start_unix_ms {
                    last.end_unix_ms = window.end_unix_ms;
                    continue;
                }
            }
            missing.push(MissingInterval {
                start_unix_ms: window.start_unix_ms,
                end_unix_ms: window.end_unix_ms,
            });
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triple_derives_with_observed_at_collection_time() {
        let record = TimestampRecord::from_collection(Some(1_710_000_000_000), 1_710_000_000_250);
        assert_eq!(record.source_timestamp_unix_ms, Some(1_710_000_000_000));
        assert_eq!(record.observed_at_unix_ms, 1_710_000_000_250);
        assert_eq!(record.collected_at_unix_ms, 1_710_000_000_250);
        record.validate().expect("valid");
        assert_eq!(record.source_age_ms(), Some(250));
    }

    #[test]
    fn absent_source_stamp_stays_absent_and_age_is_none() {
        let record = TimestampRecord::from_collection(None, 5_000);
        assert_eq!(record.source_timestamp_unix_ms, None);
        assert_eq!(record.source_age_ms(), None);
        let text = serde_yaml::to_string(&record).expect("serialise");
        assert!(!text.contains("source_timestamp"));
    }

    #[test]
    fn observed_after_collected_is_rejected() {
        let record = TimestampRecord {
            source_timestamp_unix_ms: Some(100),
            observed_at_unix_ms: 300,
            collected_at_unix_ms: 200,
        };
        assert_eq!(
            record.validate(),
            Err(TimestampError::ObservedAfterCollected(300, 200))
        );
    }

    #[test]
    fn clock_offset_requires_provenance_and_round_trips() {
        let mut metadata = CollectionMetadata::default();
        metadata.validate().expect("empty metadata valid");

        metadata.clock_offset = Some(ClockOffsetEstimate {
            offset_ms: -12,
            method: "  ".into(),
        });
        assert_eq!(metadata.validate(), Err(TimestampError::EmptyOffsetMethod));

        metadata.clock_offset = Some(ClockOffsetEstimate {
            offset_ms: -12,
            method: "probe-roundtrip".into(),
        });
        metadata.validate().expect("valid with method");

        let text = serde_yaml::to_string(&metadata).expect("serialise");
        assert!(text.contains("probe-roundtrip"));
        let back: CollectionMetadata = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, metadata);
    }

    #[test]
    fn inverted_missing_interval_is_rejected() {
        let metadata = CollectionMetadata {
            missing_intervals: vec![MissingInterval {
                start_unix_ms: 100,
                end_unix_ms: 50,
            }],
            ..CollectionMetadata::default()
        };
        assert_eq!(
            metadata.validate(),
            Err(TimestampError::InvertedMissingInterval(100, 50))
        );
    }

    #[test]
    fn missing_windows_merge_into_contiguous_gaps() {
        let expected: Vec<QueryWindow> = (0..4)
            .map(|k| QueryWindow {
                start_unix_ms: k * 1000,
                end_unix_ms: (k + 1) * 1000,
                step_ms: 1000,
            })
            .collect();
        // Window 1 produced data; windows 0, 2, 3 did not.
        let mut produced = BTreeSet::new();
        produced.insert((1000, 2000));

        let missing = compute_missing_intervals(&expected, &produced);
        assert_eq!(
            missing,
            vec![
                MissingInterval {
                    start_unix_ms: 0,
                    end_unix_ms: 1000
                },
                MissingInterval {
                    start_unix_ms: 2000,
                    end_unix_ms: 4000
                },
            ]
        );

        let all_there: BTreeSet<(i64, i64)> = expected
            .iter()
            .map(|w| (w.start_unix_ms, w.end_unix_ms))
            .collect();
        assert!(compute_missing_intervals(&expected, &all_there).is_empty());
    }

    #[test]
    fn full_metadata_records_the_doc_block() {
        let metadata = CollectionMetadata {
            query_window: Some(QueryWindow {
                start_unix_ms: 0,
                end_unix_ms: 1000,
                step_ms: 500,
            }),
            collection_interval_ms: Some(500),
            clock_offset: Some(ClockOffsetEstimate {
                offset_ms: 3,
                method: "shape-alignment".into(),
            }),
            missing_intervals: vec![MissingInterval {
                start_unix_ms: 1000,
                end_unix_ms: 1500,
            }],
            source_instance: Some("prometheus:9090".into()),
            source_labels: {
                let mut labels = Labels::new();
                labels.insert("job".into(), "kbengine".into());
                labels
            },
        };
        metadata.validate().expect("valid");
        let text = serde_yaml::to_string(&metadata).expect("serialise");
        let back: CollectionMetadata = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, metadata);
        assert!(text.contains("shape-alignment"));
        assert!(text.contains("prometheus:9090"));
    }
}
