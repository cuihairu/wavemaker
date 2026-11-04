#!/usr/bin/env python3
"""Validate a Wavemaker Swell report JSON against schemas/report/v1.json.

Usage:
    python3 tools/validate_report.py <report.json>          # exit 0 on valid
    python3 -m pytest tools/test_validate_report.py         # schema self-tests
"""

from __future__ import annotations

import argparse
import json
import os
import sys

from jsonschema import Draft202012Validator, FormatChecker

SCHEMA_PATH = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..", "schemas", "report", "v1.json",
)


def load_schema() -> dict:
    with open(SCHEMA_PATH, encoding="utf-8") as f:
        schema = json.load(f)
    Draft202012Validator.check_schema(schema)
    return schema


def validate_report(report: dict) -> list[str]:
    """Return a list of validation error messages (empty when valid)."""
    validator = Draft202012Validator(load_schema(), format_checker=FormatChecker())
    errors = sorted(validator.iter_errors(report), key=lambda e: list(e.absolute_path))
    return [
        f"{'/'.join(map(str, e.absolute_path)) or '<root>'}: {e.message}"
        for e in errors
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", help="path to the report JSON file")
    args = parser.parse_args()
    with open(args.report, encoding="utf-8") as f:
        report = json.load(f)
    errors = validate_report(report)
    if errors:
        for msg in errors:
            print(f"INVALID: {msg}", file=sys.stderr)
        return 1
    print(f"OK: {args.report} conforms to wavemaker.report/v1")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
