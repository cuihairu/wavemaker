"""Self-tests for schemas/report/v1.json via tools/validate_report.py.

Good sample passes; deliberately corrupted samples are rejected — each bad
case targets one plan-document §13.3 conclusion constraint or a structural
requirement.

Run: python3 -m pytest tools/test_validate_report.py -v
"""

import copy
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from validate_report import load_schema, validate_report  # noqa: E402

SECTIONS = (
    "experiment_summary spectrum_load_profile planned_vs_actual_arrival "
    "concurrency_curve client_performance generator_health "
    "engine_bot_backend_details server_performance correlation_analysis "
    "bottleneck_analysis data_quality test_validity environment "
    "raw_data_references"
).split()


def make_section(quality="Complete", sources=("Planned",), **extra):
    sec = {"quality": quality, "sources": list(sources)}
    sec.update(extra)
    return sec


def good_report() -> dict:
    """A minimal but fully conforming report."""
    secs = {name: make_section() for name in SECTIONS}
    secs["experiment_summary"].update(
        swell_name="staging-login-capacity", backend_count=1,
        started_at="2026-11-05T10:00:00Z", ended_at="2026-11-05T10:20:00Z")
    secs["generator_health"]["saturation"] = {"flag": False}
    secs["test_validity"]["verdict"] = "Valid"
    secs["server_performance"] = make_section(
        quality="Complete", sources=("Basin",),
        metrics=[{"metric": "tick_duration_ms", "unit": "ms",
                  "coverage_percent": 100.0}])
    return {
        "schema_version": "wavemaker.report/v1",
        "swell_id": "swell-001",
        "generated_at": "2026-11-05T10:30:00Z",
        "overall_data_quality": "Complete",
        "sections": secs,
        "conclusions": {
            "confidence": "high",
            "capacity_conclusion": {
                "statement": "sustained 5000 concurrent logins with stable tick duration",
                "based_on": ["Basin", "Actual"],
                "concurrency_level": 5000,
            },
            "bottleneck_claims": [],
        },
    }


def broken(mutate) -> dict:
    report = good_report()
    mutate(report)
    return report


# --- schema itself ---------------------------------------------------------------

def test_schema_is_valid_draft_2020_12():
    load_schema()  # raises on invalid schema


def test_good_sample_passes():
    assert validate_report(good_report()) == []


# --- structural rejection ---------------------------------------------------------

def test_missing_section_rejected():
    def mutate(r):
        del r["sections"]["concurrency_curve"]
    errors = validate_report(broken(mutate))
    assert any("concurrency_curve" in e for e in errors)


def test_unknown_section_rejected():
    def mutate(r):
        r["sections"]["extra_section"] = make_section()
    assert validate_report(broken(mutate))


def test_invalid_quality_label_rejected():
    def mutate(r):
        r["sections"]["client_performance"]["quality"] = "Fine"
    assert validate_report(broken(mutate))


def test_missing_sources_rejected():
    def mutate(r):
        del r["sections"]["server_performance"]["sources"]
    assert validate_report(broken(mutate))


def test_wrong_schema_version_rejected():
    def mutate(r):
        r["schema_version"] = "wavemaker.report/v2"
    assert validate_report(broken(mutate))


# --- plan §13.3 conclusion constraints ---------------------------------------------

def test_generator_saturation_forbids_capacity_conclusion():
    def mutate(r):
        r["sections"]["generator_health"]["saturation"] = {
            "flag": True, "reason": "worker cpu pinned at 100%"}
    errors = validate_report(broken(mutate))
    assert any("capacity_conclusion" in e for e in errors)


def test_overall_generator_saturated_forbids_capacity_conclusion():
    def mutate(r):
        r["overall_data_quality"] = "GeneratorSaturated"
    assert any(
        "capacity_conclusion" in e for e in validate_report(broken(mutate)))


def test_capacity_conclusion_requires_basin_server_data():
    def mutate(r):
        r["sections"]["server_performance"]["sources"] = ["Planned"]
        r["sections"]["server_performance"]["quality"] = "Missing"
    errors = validate_report(broken(mutate))
    assert errors, "client/planned-only capacity conclusion must be rejected"


def test_poor_observability_forces_low_confidence():
    def mutate(r):
        r["sections"]["server_performance"]["quality"] = "Partial"
    errors = validate_report(broken(mutate))
    assert any("confidence" in e for e in errors)


def test_correlation_may_not_assert_causality():
    def mutate(r):
        r["sections"]["correlation_analysis"]["claims"] = [{
            "metric_a": "worker_cpu_percent",
            "metric_b": "tick_duration_ms",
            "asserts_causality": True,
        }]
    assert validate_report(broken(mutate))


def test_single_metric_bottleneck_inference_rejected():
    def mutate(r):
        r["sections"]["bottleneck_analysis"]["claims"] = [{
            "resource": "cpu",
            "statement": "CPU is high, therefore CPU is the only bottleneck",
            "single_metric_inference": True,
            "evidence": [{"section": "generator_health",
                          "quality": "Complete"}],
        }]
    assert validate_report(broken(mutate))


def test_bottleneck_claim_requires_evidence():
    def mutate(r):
        r["sections"]["bottleneck_analysis"]["claims"] = [{
            "resource": "cpu",
            "statement": "CPU-bound",
            "single_metric_inference": False,
            "evidence": [],
        }]
    assert validate_report(broken(mutate))


# --- ADR-007 assertions section -------------------------------------------------------

def assertion_result(status="pass", name="no-duplicate-roles"):
    return {
        "provider": "game-admin-api",
        "name": name,
        "status": status,
        "checked_at": "2026-11-05T10:25:00Z",
        "trigger": "after_scenario",
        "message": "",
        "evidence": [{"kind": "artifact",
                      "ref": "swell-001/evidence/no-duplicate-roles-diff.json"}],
    }


def test_report_without_assertions_still_passes():
    # assertions is an optional section (ADR-007); pre-ADR-007 reports stay valid.
    assert validate_report(good_report()) == []


def test_report_with_passing_assertion_passes():
    report = good_report()
    report["sections"]["assertions"] = make_section(
        quality="Complete", sources=("AssertionProvider",),
        results=[assertion_result("pass")])
    assert validate_report(report) == []


def test_failed_assertion_forbids_valid_verdict():
    report = good_report()
    report["sections"]["assertions"] = make_section(
        quality="Complete", sources=("AssertionProvider",),
        results=[assertion_result("fail")])
    errors = validate_report(report)
    assert any("verdict" in e for e in errors), (
        "a failed assertion must downgrade test_validity away from 'Valid'")


def test_failed_assertion_with_partial_verdict_passes():
    report = good_report()
    report["sections"]["assertions"] = make_section(
        quality="Complete", sources=("AssertionProvider",),
        results=[assertion_result("fail")])
    report["sections"]["test_validity"]["verdict"] = "Invalid"
    report["sections"]["test_validity"]["failed_criteria"] = [
        "assertion:no-duplicate-roles:fail"]
    assert validate_report(report) == []


def test_inconclusive_assertion_allows_valid_verdict():
    report = good_report()
    report["sections"]["assertions"] = make_section(
        quality="Missing", sources=("AssertionProvider",),
        results=[assertion_result("inconclusive")])
    assert validate_report(report) == []


def test_invalid_assertion_status_rejected():
    def mutate(r):
        r["sections"]["assertions"] = make_section(
            quality="Complete", sources=("AssertionProvider",),
            results=[assertion_result("ok")])
    assert validate_report(broken(mutate))


def test_assertion_requires_evidence():
    def mutate(r):
        res = assertion_result("pass")
        res["evidence"] = []
        r["sections"]["assertions"] = make_section(
            quality="Complete", sources=("AssertionProvider",), results=[res])
    errors = validate_report(broken(mutate))
    assert any("evidence" in e for e in errors)


def test_assertion_provider_source_label_accepted():
    # the new data-source label must be usable on ordinary sections too
    def mutate(r):
        r["sections"]["data_quality"]["sources"] = [
            "Planned", "AssertionProvider"]
    assert validate_report(broken(mutate)) == []


# --- multi-backend grouping ----------------------------------------------------------

def test_multiple_backends_require_per_backend_grouping():
    def mutate(r):
        r["sections"]["experiment_summary"]["backend_count"] = 2
    errors = validate_report(broken(mutate))
    assert any("per_backend" in e for e in errors)


def test_multi_backend_report_with_grouping_passes():
    report = good_report()
    report["sections"]["experiment_summary"]["backend_count"] = 2
    report["sections"]["client_performance"]["per_backend"] = [
        {"backend_id": "rust-worker-a", "backend_type": "native",
         "quality": "Complete", "latency_p99_ms": 12.5},
        {"backend_id": "kbengine-bots", "backend_type": "engine_bot",
         "quality": "Partial", "latency_p99_ms": None},
    ]
    assert validate_report(report) == []
