#!/usr/bin/env python3
"""Summarize validated FoldScan optical-spike bench measurements."""

from __future__ import annotations

import argparse
import json
import math
import statistics
import sys
from pathlib import Path
from typing import Any

from validate_manifest import ValidationError, validate_manifest

DENSITY_FIELDS = (
    "center_px_per_mm",
    "top_left_px_per_mm",
    "top_right_px_per_mm",
    "bottom_left_px_per_mm",
    "bottom_right_px_per_mm",
)


def _distribution(values: list[float]) -> dict[str, float | int]:
    ordered = sorted(values)
    p95_index = math.ceil(0.95 * len(ordered)) - 1
    return {
        "samples": len(ordered),
        "minimum": ordered[0],
        "median": statistics.median(ordered),
        "p95": ordered[p95_index],
        "maximum": ordered[-1],
    }


def _status(passed: bool, measured: str, target: str) -> dict[str, str]:
    return {
        "status": "pass" if passed else "fail",
        "measured": measured,
        "target": target,
    }


def summarize_manifest(data: dict[str, Any]) -> dict[str, Any]:
    """Compute deterministic statistics and narrow acceptance checks.

    The caller must first pass the manifest through ``validate_manifest``. This
    function deliberately does not turn measurements into claims that the
    manifest cannot support.
    """
    page_sizes: dict[str, Any] = {}
    for page_size in ("A4", "LETTER"):
        captures = [capture for capture in data["captures"] if capture["page_size"].upper() == page_size]
        page_sizes[page_size] = {
            "capture_count": len(captures),
            "latency_ms": _distribution(
                [capture["latency_to_durable_file_ms"] for capture in captures]
            ),
            "jpeg_bytes": _distribution([capture["bytes"] for capture in captures]),
            "density_px_per_mm": _distribution(
                [capture[field] for capture in captures for field in DENSITY_FIELDS]
            ),
            "maximum_distortion_px": max(capture["max_distortion_px"] for capture in captures),
            "maximum_saturated_pixel_fraction": max(
                capture["saturated_pixel_fraction"] for capture in captures
            ),
        }

    mechanical_data = data["mechanical"]
    max_refold = max(
        math.hypot(cycle["x_mm"], cycle["y_mm"])
        for cycle in mechanical_data["refold_cycles"]
    )
    mechanical = {
        "max_refold_radial_displacement_mm": max_refold,
        "vertical_deflection_10min_mm": mechanical_data["vertical_deflection_10min_mm"],
    }
    power = dict(data["power_and_thermal"])

    latency_passed = all(
        values["median"] <= 3000 and values["p95"] <= 5000
        for values in (page_sizes["A4"]["latency_ms"], page_sizes["LETTER"]["latency_ms"])
    )
    ambient = data["fixture"]["ambient_temperature_c"]
    thermal_duration = power["test_duration_minutes"]
    touch_temperature = power["max_touchable_temperature_c"]
    touch_conditions_met = ambient >= 35 and thermal_duration >= 30

    acceptance_checks: dict[str, dict[str, str]] = {
        "OPT-01": {
            "status": "not-evaluable",
            "measured": "manifest does not record four page-edge margin measurements",
            "target": ">= 8 mm visible margin on every side for A4 and Letter",
        },
        "OPT-02": {
            "status": "observed-no-threshold",
            "measured": "density, readable-text labels, and distortion observations are recorded",
            "target": "no numeric acceptance threshold is defined in hardware/requirements.md",
        },
        "OPT-03": _status(
            latency_passed,
            "; ".join(
                f"{page}: median={page_sizes[page]['latency_ms']['median']} ms, "
                f"p95={page_sizes[page]['latency_ms']['p95']} ms"
                for page in ("A4", "LETTER")
            ),
            "each page size: median <= 3000 ms and p95 <= 5000 ms",
        ),
        "MECH-03": _status(
            max_refold <= 2,
            f"maximum radial displacement={max_refold:.3f} mm",
            "<= 2 mm after ten refold cycles",
        ),
        "MECH-04": _status(
            mechanical["vertical_deflection_10min_mm"] <= 2,
            f"vertical deflection={mechanical['vertical_deflection_10min_mm']} mm",
            "<= 2 mm after ten minutes",
        ),
        "ELEC-02-source-envelope": _status(
            power["combined_peak_current_ma"] <= 3000,
            f"combined measured peak={power['combined_peak_current_ma']} mA",
            "<= 3000 mA from the 5 V source; design margin requires separate review",
        ),
        "ENV-02-touchable-temperature": (
            _status(
                touch_temperature < 45,
                f"maximum touchable={touch_temperature} C at {ambient} C ambient for "
                f"{thermal_duration} min",
                "< 45 C at >= 35 C ambient after >= 30 min",
            )
            if touch_conditions_met
            else {
                "status": "not-evaluable",
                "measured": f"ambient={ambient} C, duration={thermal_duration} min",
                "target": "test at >= 35 C ambient for >= 30 min",
            }
        ),
    }

    return {
        "schema_version": "1.1",
        "evidence_category": data["evidence_category"],
        "run_id": data["run_id"],
        "captured_at_utc": data["captured_at_utc"],
        "measurement_log_provenance": {
            role: dict(spec) for role, spec in data["measurement_logs"].items()
        },
        "measurements": {
            "page_sizes": page_sizes,
            "mechanical": mechanical,
            "power_and_thermal": power,
        },
        "acceptance_checks": acceptance_checks,
        "unresolved_evidence": [
            "page-edge margin measurements for OPT-01",
            "component temperature versus datasheet limits",
            "illumination uniformity, color shift, and rolling-band review",
            "200-page storage-capacity and recovery testing",
            "raw-image and measurement-log reviewer inspection",
        ],
    }


def summarize_manifest_file(manifest_path: Path) -> dict[str, Any]:
    """Validate and summarize a stable real bench manifest."""
    try:
        before = manifest_path.read_bytes()
    except OSError as exc:
        raise ValidationError(f"cannot read manifest for summary: {exc}") from exc
    validate_manifest(manifest_path)
    try:
        after = manifest_path.read_bytes()
    except OSError as exc:
        raise ValidationError(f"cannot reread manifest for summary: {exc}") from exc
    if before != after:
        raise ValidationError("manifest changed while it was being validated")
    data = json.loads(before)
    return summarize_manifest(data)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path, help="write JSON summary to this path")
    args = parser.parse_args()
    try:
        summary = summarize_manifest_file(args.manifest)
    except ValidationError as exc:
        print(f"INVALID: {exc}", file=sys.stderr)
        return 1
    rendered = json.dumps(summary, indent=2, sort_keys=True) + "\n"
    if args.output is None:
        sys.stdout.write(rendered)
    else:
        args.output.write_text(rendered, encoding="utf-8")
        print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
