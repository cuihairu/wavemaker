//! WM-0007 cross-implementation HDR conformance tests.
//!
//! The fixture is generated once by HdrHistogram Java 2.2.2 and
//! hdrhistogram-go v1.4.0, then verified against an independent Python
//! decoder (`tools/hdr_conformance/compare.py`) and committed. These tests
//! assert that this Rust implementation produces the identical geometry,
//! statistics and decoded counts — making Java + Go + Python + Rust a
//! four-implementation agreement on the frozen V2 format.

use serde::Deserialize;
use wm_core::hdr::{HdrGeometry, HdrParameters, Histogram};

#[derive(Deserialize)]
struct Fixture {
    meta: FixtureMeta,
    param_matrix: Vec<GeometryRow>,
    frozen_v1: FrozenSection,
    value_cases: Vec<ValueCase>,
}

#[derive(Deserialize)]
struct FixtureMeta {
    java: MetaEntry,
    go: MetaEntry,
    percentile_rounding: String,
}

#[derive(Deserialize)]
struct MetaEntry {
    implementation: String,
    date: String,
}

#[derive(Deserialize)]
struct GeometryRow {
    lowest: i64,
    sigfigs: u32,
    highest: i64,
    largest_value_single_unit_resolution: i64,
    sub_bucket_count: i64,
    sub_bucket_half_count: i64,
    sub_bucket_half_count_magnitude: i64,
    unit_magnitude: i64,
    bucket_count: i64,
    counts_len: i64,
}

#[derive(Deserialize)]
struct FrozenSection {
    #[allow(dead_code)]
    lowest: i64,
    #[allow(dead_code)]
    sigfigs: u32,
    #[allow(dead_code)]
    highest: i64,
    geometry: GeometryRow,
    value_from_index: Vec<i64>,
    counts_index_probes: Vec<IndexProbe>,
}

#[derive(Deserialize)]
struct IndexProbe {
    value: i64,
    counts_index: i64,
}

#[derive(Deserialize)]
struct ValueCase {
    name: String,
    #[allow(dead_code)]
    lowest: i64,
    #[allow(dead_code)]
    sigfigs: u32,
    #[allow(dead_code)]
    highest: i64,
    values: Vec<i64>,
    expected: ExpectedStats,
    go_expected: serde_json::Value,
    java_blob_hex: String,
    go_blob_hex: String,
}

#[derive(Deserialize)]
struct ExpectedStats {
    total_count: i64,
    min: i64,
    max: i64,
    mean: f64,
    p50: i64,
    p95: i64,
    p99: i64,
    p99_9: i64,
    p100: i64,
}

fn fixture() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/hdr_conformance.json"
    );
    let raw = std::fs::read_to_string(path).expect("fixture file readable");
    serde_json::from_str(&raw).expect("fixture parses")
}

fn assert_geometry_matches(actual: &HdrGeometry, expected: &GeometryRow) {
    assert_eq!(
        actual.largest_value_with_single_unit_resolution,
        expected.largest_value_single_unit_resolution
    );
    assert_eq!(actual.sub_bucket_count, expected.sub_bucket_count);
    assert_eq!(actual.sub_bucket_half_count, expected.sub_bucket_half_count);
    assert_eq!(
        actual.sub_bucket_half_count_magnitude as i64,
        expected.sub_bucket_half_count_magnitude
    );
    assert_eq!(actual.unit_magnitude as i64, expected.unit_magnitude);
    assert_eq!(actual.bucket_count as i64, expected.bucket_count);
    assert_eq!(actual.counts_len as i64, expected.counts_len);
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex digit"))
        .collect()
}

fn assert_stats_match(histogram: &Histogram, expected: &ExpectedStats, case: &str) {
    assert_eq!(
        histogram.total_count(),
        expected.total_count,
        "{case} total"
    );
    assert_eq!(histogram.min_value(), expected.min, "{case} min");
    assert_eq!(histogram.max_value(), expected.max, "{case} max");
    let mean_error = (histogram.mean() - expected.mean).abs();
    let mean_scale = expected.mean.abs().max(1.0);
    assert!(
        mean_error <= mean_scale * 1e-12,
        "{case} mean: {} vs {}",
        histogram.mean(),
        expected.mean
    );
    assert_eq!(
        histogram.value_at_percentile(50.0),
        expected.p50,
        "{case} p50"
    );
    assert_eq!(
        histogram.value_at_percentile(95.0),
        expected.p95,
        "{case} p95"
    );
    assert_eq!(
        histogram.value_at_percentile(99.0),
        expected.p99,
        "{case} p99"
    );
    assert_eq!(
        histogram.value_at_percentile(99.9),
        expected.p99_9,
        "{case} p99.9"
    );
    assert_eq!(
        histogram.value_at_percentile(100.0),
        expected.p100,
        "{case} p100"
    );
}

fn recorded_histogram(case: &ValueCase) -> Histogram {
    let params =
        HdrParameters::new(case.lowest, case.sigfigs, case.highest).expect("case params valid");
    let mut histogram = Histogram::new(params).expect("histogram allocates");
    for &value in &case.values {
        histogram.record_value(value).expect("case value in range");
    }
    histogram
}

#[test]
fn param_matrix_matches_java_and_go() {
    let fixture = fixture();
    assert_eq!(fixture.param_matrix.len(), 10, "ten parameter rows");
    for row in &fixture.param_matrix {
        let params = HdrParameters::new(row.lowest, row.sigfigs, row.highest)
            .unwrap_or_else(|e| panic!("params {:?}: {e}", (row.lowest, row.sigfigs, row.highest)));
        assert_geometry_matches(&params.geometry().unwrap(), row);
    }
}

#[test]
fn frozen_v1_geometry_and_index_maps_match_java() {
    let fixture = fixture();
    let params = HdrParameters::frozen_v1();
    assert_eq!(params.lowest_discernible_value, fixture.frozen_v1.lowest);
    assert_eq!(params.significant_figures, fixture.frozen_v1.sigfigs);
    assert_eq!(params.highest_trackable_value, fixture.frozen_v1.highest);
    let geometry = params.geometry().unwrap();
    assert_geometry_matches(&geometry, &fixture.frozen_v1.geometry);

    assert_eq!(
        geometry.counts_len,
        fixture.frozen_v1.value_from_index.len(),
        "full index map covers every slot"
    );
    for (index, &expected) in fixture.frozen_v1.value_from_index.iter().enumerate() {
        assert_eq!(geometry.value_from_index(index), expected, "slot {index}");
    }
    for probe in &fixture.frozen_v1.counts_index_probes {
        assert_eq!(
            geometry.counts_index_for(probe.value) as i64,
            probe.counts_index,
            "probe value {}",
            probe.value
        );
    }
}

#[test]
fn recorded_stats_match_java() {
    let fixture = fixture();
    for case in &fixture.value_cases {
        let histogram = recorded_histogram(case);
        assert_stats_match(&histogram, &case.expected, &case.name);
    }
}

#[test]
fn java_and_go_blobs_decode_to_identical_counts() {
    let fixture = fixture();
    for case in &fixture.value_cases {
        let recorded = recorded_histogram(case);

        for (impl_name, hex) in [("java", &case.java_blob_hex), ("go", &case.go_blob_hex)] {
            let decoded = Histogram::decode_v2_blob(&unhex(hex))
                .unwrap_or_else(|e| panic!("{} blob of {}: {e}", impl_name, case.name));
            assert_eq!(
                decoded.counts(),
                recorded.counts(),
                "{} blob counts differ from recorded state for {}",
                impl_name,
                case.name
            );
            assert_stats_match(
                &decoded,
                &case.expected,
                &format!("{} decode {}", impl_name, case.name),
            );
        }
    }
}

#[test]
fn reencoded_blobs_roundtrip() {
    let fixture = fixture();
    for case in &fixture.value_cases {
        let recorded = recorded_histogram(case);
        let blob = recorded.encode_v2_blob().expect("encode succeeds");
        let back = Histogram::decode_v2_blob(&blob)
            .unwrap_or_else(|e| panic!("own blob of {}: {e}", case.name));
        assert_eq!(back.counts(), recorded.counts(), "{} roundtrip", case.name);
        assert_stats_match(&back, &case.expected, &case.name);
    }
}

#[test]
fn go_percentile_divergence_is_documented() {
    // hdrhistogram-go truncates the percentile target count while Java (and
    // this implementation) round up. The fixture keeps Go's numbers under
    // go_expected for reference; they are deliberately NOT asserted equal.
    let fixture = fixture();
    assert!(fixture.meta.percentile_rounding.contains("Java"));
    assert!(fixture.meta.percentile_rounding.contains("truncat"));
    let case = fixture
        .value_cases
        .iter()
        .find(|c| c.name == "small")
        .expect("small case present");
    let go_p95 = case.go_expected["p95"]
        .as_i64()
        .expect("go p95 is an integer");
    assert_ne!(
        go_p95, case.expected.p95,
        "expected the documented Java/Go p95 divergence in the small case"
    );
}

#[test]
fn fixture_reference_versions() {
    // Pin the reference implementations the fixture was generated with.
    let fixture = fixture();
    assert!(fixture
        .meta
        .java
        .implementation
        .contains("HdrHistogram 2.2.2"));
    assert!(fixture
        .meta
        .go
        .implementation
        .contains("hdrhistogram-go v1.4.0"));
    assert_eq!(fixture.meta.java.date, "2026-11-09");
    assert_eq!(fixture.meta.go.date, "2026-11-09");
}
