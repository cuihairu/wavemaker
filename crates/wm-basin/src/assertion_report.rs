//! Report §15 assertions section, data-source tagging, and the R5 Test
//! Validity degradation (WM-0705; docs/spec/assertion-provider.md §5,
//! schemas/report/v1.json).
//!
//! The section is optional in the schema so pre-ADR-007 reports stay
//! valid — emitting it is the report writer's choice; this module makes
//! the emitted shape faithful and the coupling mechanical:
//!
//! - R5 (schema if/then): when any assertion result has status `fail`,
//!   `test_validity.verdict` MUST be Invalid or Partial — a business
//!   semantics violation cannot coexist with a `Valid` verdict. Each
//!   failing assertion adds `assertion:<name>:fail` to
//!   `failed_criteria`. `inconclusive` never triggers R5 (it degrades
//!   data quality, not validity).
//! - §5.2: the section's data source is `AssertionProvider` — not
//!   Planned, not Actual, not Basin. Mixing tags would break R2's
//!   semantics.
//! - §5.3: `fail` degrades VALIDITY, not data quality (the evidence
//!   arrived; the business failed). `inconclusive` degrades data
//!   quality (Missing/Partial) because the check's data is absent.
//!   Confidence is deliberately NOT forced here (the constraint layer
//!   leaves it to R3 to avoid semantic conflicts).

use serde::{Deserialize, Serialize};

use crate::assertions::{AssertionRecord, AssertionStatus};
use crate::labels::{DataQualityLabel, QualityLabelSet};

/// Origin of a section's data (schema `data_source` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DataSource {
    Planned,
    Actual,
    Basin,
    Generator,
    AssertionProvider,
}

/// §16.3 / R5 verdict (schema `test_validity.verdict` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidityVerdict {
    Valid,
    Invalid,
    Partial,
}

/// §16 `test_validity` section content (schema-shaped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestValidity {
    pub verdict: ValidityVerdict,
    /// Schema `failed_criteria`; R5's additions append
    /// `assertion:<name>:fail` entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_criteria: Vec<String>,
}

/// §5.1 R5: a failing assertion invalidates a `Valid` verdict. The
/// minimal permitted degradation is `Partial` — whether it must go to
/// `Invalid` is §16.3's business (the existing stacked conditions), not
/// the assertion channel's. `inconclusive` is deliberately not R5.
pub fn enforce_r5(base: TestValidity, records: &[AssertionRecord]) -> TestValidity {
    let failed: Vec<&AssertionRecord> = records
        .iter()
        .filter(|record| record.status == AssertionStatus::Fail)
        .collect();
    if failed.is_empty() {
        return base;
    }
    let verdict = match base.verdict {
        ValidityVerdict::Valid => ValidityVerdict::Partial,
        other => other,
    };
    let mut failed_criteria = base.failed_criteria;
    for record in failed {
        let criterion = format!("assertion:{}:fail", record.name);
        if !failed_criteria.contains(&criterion) {
            failed_criteria.push(criterion);
        }
    }
    TestValidity {
        verdict,
        failed_criteria,
    }
}

/// §15 `assertions` section content (schema-shaped; `quality` is the
/// §11.3 label set the report writer renders into `data_quality`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionsSection {
    pub results: Vec<AssertionRecord>,
    /// §5.2: every section carrying assertion results is tagged
    /// AssertionProvider — nothing else may be blended in.
    pub sources: Vec<DataSource>,
    pub quality: QualityLabelSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Assemble the section from the scheduler's accumulated outcomes.
///
/// §5.3 data-quality coupling: `inconclusive` results mean the check's
/// data is absent → Missing + Partial. `fail` results leave data
/// quality alone. An empty result set is itself a gap (named Missing)
/// — a section that claims assertions but carries none is not Complete.
pub fn assemble_assertions_section(records: &[AssertionRecord]) -> AssertionsSection {
    let mut quality = QualityLabelSet::default();
    if records.is_empty() {
        quality.insert(DataQualityLabel::Missing);
    } else if records
        .iter()
        .any(|record| record.status == AssertionStatus::Inconclusive)
    {
        quality.insert(DataQualityLabel::Missing);
        quality.insert(DataQualityLabel::Partial);
    } else {
        quality.insert(DataQualityLabel::Complete);
    }
    AssertionsSection {
        results: records.to_vec(),
        sources: vec![DataSource::AssertionProvider],
        quality,
        notes: if records.is_empty() {
            Some("empty result set".into())
        } else {
            None
        },
    }
}

/// §5.1: the §14 `raw_data_references` artifacts for this section —
/// schema-shaped (`path` + `format`, `evidence-bundle` for assertion
/// evidence bundles). Delegates to the archive's mapping so the §15→§14
/// registration cannot drift from what the archive prepared.
pub fn raw_data_artifacts(records: &[AssertionRecord]) -> Vec<RawDataArtifact> {
    let archive = crate::evidence::raw_data_references(records);
    archive
        .into_iter()
        .map(|entry| RawDataArtifact {
            path: entry.path,
            format: entry.format,
        })
        .collect()
}

/// One §14 artifact entry (schema-shaped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDataArtifact {
    pub path: String,
    pub format: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::{EvidenceKind, EvidenceRef};

    fn record(name: &str, status: AssertionStatus) -> AssertionRecord {
        AssertionRecord {
            provider: "game-admin-api".into(),
            name: name.into(),
            status,
            checked_at: "2026-11-10T00:00:00Z".into(),
            window: None,
            trigger: Some(crate::assertions::Trigger::AfterScenario),
            message: match status {
                AssertionStatus::Fail => Some("3 duplicate role pairs found".into()),
                _ => None,
            },
            evidence: vec![EvidenceRef {
                kind: EvidenceKind::Query,
                reference: "SELECT 1".into(),
            }],
        }
    }

    #[test]
    fn a_fail_degrades_valid_to_partial_and_lists_criteria() {
        let base = TestValidity {
            verdict: ValidityVerdict::Valid,
            failed_criteria: Vec::new(),
        };
        let records = vec![record("no-duplicate-roles", AssertionStatus::Fail)];
        let validity = enforce_r5(base, &records);
        assert_eq!(validity.verdict, ValidityVerdict::Partial);
        assert_eq!(
            validity.failed_criteria,
            vec!["assertion:no-duplicate-roles:fail".to_string()]
        );
    }

    #[test]
    fn r5_never_upgrades_invalid_or_partial() {
        let base = TestValidity {
            verdict: ValidityVerdict::Invalid,
            failed_criteria: vec!["throughput:magnitude".into()],
        };
        let records = vec![record("dupes", AssertionStatus::Fail)];
        let validity = enforce_r5(base, &records);
        assert_eq!(validity.verdict, ValidityVerdict::Invalid);
        assert_eq!(validity.failed_criteria.len(), 2);
        assert_eq!(validity.failed_criteria[0], "throughput:magnitude");
        assert_eq!(validity.failed_criteria[1], "assertion:dupes:fail");

        let validity = enforce_r5(
            TestValidity {
                verdict: ValidityVerdict::Partial,
                failed_criteria: Vec::new(),
            },
            &records,
        );
        assert_eq!(validity.verdict, ValidityVerdict::Partial);
    }

    #[test]
    fn inconclusive_never_triggers_r5() {
        let base = TestValidity {
            verdict: ValidityVerdict::Valid,
            failed_criteria: Vec::new(),
        };
        let records = vec![record("probe", AssertionStatus::Inconclusive)];
        let validity = enforce_r5(base, &records);
        assert_eq!(validity.verdict, ValidityVerdict::Valid);
        assert!(validity.failed_criteria.is_empty());
    }

    #[test]
    fn each_failing_assertion_gets_its_own_criterion() {
        let base = TestValidity {
            verdict: ValidityVerdict::Valid,
            failed_criteria: Vec::new(),
        };
        let records = vec![
            record("a", AssertionStatus::Fail),
            record("b", AssertionStatus::Pass),
            record("c", AssertionStatus::Fail),
            record("a", AssertionStatus::Fail), // duplicate name impossible from the scheduler, but the criteria list stays deduped
        ];
        let validity = enforce_r5(base, &records);
        assert_eq!(
            validity.failed_criteria,
            vec![
                "assertion:a:fail".to_string(),
                "assertion:c:fail".to_string(),
            ]
        );
    }

    #[test]
    fn section_quality_follows_the_53_table() {
        // All pass → Complete.
        let all_pass = vec![
            record("a", AssertionStatus::Pass),
            record("b", AssertionStatus::Pass),
        ];
        let section = assemble_assertions_section(&all_pass);
        assert!(section.quality.contains(DataQualityLabel::Complete));
        assert!(section.quality.is_complete());

        // Any fail → validity degrades elsewhere; data quality stays
        // honest (the evidence arrived).
        let with_fail = vec![
            record("a", AssertionStatus::Pass),
            record("b", AssertionStatus::Fail),
        ];
        let section = assemble_assertions_section(&with_fail);
        assert!(section.quality.is_complete());

        // Any inconclusive → Missing + Partial.
        let with_inconclusive = vec![
            record("a", AssertionStatus::Pass),
            record("b", AssertionStatus::Inconclusive),
        ];
        let section = assemble_assertions_section(&with_inconclusive);
        assert!(section.quality.contains(DataQualityLabel::Missing));
        assert!(section.quality.contains(DataQualityLabel::Partial));
        assert!(!section.quality.is_complete());

        // Empty result set is a named gap, never Complete.
        let section = assemble_assertions_section(&[]);
        assert!(section.quality.contains(DataQualityLabel::Missing));
        assert_eq!(section.notes.as_deref(), Some("empty result set"));
    }

    #[test]
    fn sections_are_always_tagged_assertion_provider() {
        let section = assemble_assertions_section(&[record("a", AssertionStatus::Pass)]);
        assert_eq!(section.sources, vec![DataSource::AssertionProvider]);
    }

    #[test]
    fn raw_artifacts_use_the_schema_path_and_bundle_format() {
        let mut artifact_record = record("no-duplicate-roles", AssertionStatus::Fail);
        artifact_record.evidence.push(EvidenceRef {
            kind: EvidenceKind::Artifact,
            reference: "evidence/no-duplicate-roles/20261110T000000Z/roles-diff.json".into(),
        });
        let artifacts = raw_data_artifacts(&[artifact_record]);
        assert_eq!(
            artifacts,
            vec![RawDataArtifact {
                path: "evidence/no-duplicate-roles/20261110T000000Z/roles-diff.json".into(),
                format: "evidence-bundle".into(),
            }]
        );
    }

    #[test]
    fn types_serialise_with_schema_spellings() {
        let validity = TestValidity {
            verdict: ValidityVerdict::Partial,
            failed_criteria: vec!["assertion:dupes:fail".into()],
        };
        let json = serde_json::to_string(&validity).expect("json");
        assert!(json.contains("\"Partial\""));

        let section = assemble_assertions_section(&[record("dupes", AssertionStatus::Fail)]);
        let text = serde_json::to_string(&section).expect("json");
        assert!(text.contains("\"AssertionProvider\""));
        let back: AssertionsSection = serde_json::from_str(&text).expect("reparse");
        assert_eq!(back, section);
    }

    #[test]
    fn scheduler_outcomes_flow_end_to_end_into_section_and_validity() {
        // Records as the WM-0702 scheduler produces them (verbatim
        // provider output + no-shows), through §15 assembly, R5, and §14
        // registration.
        let records = vec![
            record("reward-consistency", AssertionStatus::Pass),
            record("no-duplicate-roles", AssertionStatus::Fail),
            record("provider-hung", AssertionStatus::Inconclusive),
        ];
        let section = assemble_assertions_section(&records);
        let validity = enforce_r5(
            TestValidity {
                verdict: ValidityVerdict::Valid,
                failed_criteria: Vec::new(),
            },
            &records,
        );
        assert_eq!(validity.verdict, ValidityVerdict::Partial);
        assert_eq!(
            validity.failed_criteria,
            vec!["assertion:no-duplicate-roles:fail".to_string()]
        );
        assert!(section.quality.contains(DataQualityLabel::Missing));
        assert_eq!(section.results.len(), 3);
    }
}
