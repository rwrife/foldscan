#!/usr/bin/env python3
"""Fail-closed validator for FoldScan optical-spike bench manifests."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from collections import Counter
from pathlib import Path
from typing import Any

PAGE_SIZES = {"A4", "LETTER"}
CONDITION_MINIMUMS = {
    "symmetric-matte": 30,
    "ambient-only": 3,
    "left-light-only": 3,
    "right-light-only": 3,
    "symmetric-glossy": 3,
}
PLACEHOLDER_WORDS = {"", "unknown", "tbd", "not measured", "not-measured", "n/a", "none"}


class ValidationError(ValueError):
    """Raised when a manifest cannot support the claimed bench evidence."""


def _require(mapping: dict[str, Any], key: str, location: str) -> Any:
    if key not in mapping:
        raise ValidationError(f"{location}: missing required field {key!r}")
    return mapping[key]


def _require_text(mapping: dict[str, Any], key: str, location: str) -> str:
    value = _require(mapping, key, location)
    if not isinstance(value, str) or value.strip().lower() in PLACEHOLDER_WORDS:
        raise ValidationError(f"{location}.{key}: must be measured/identified non-placeholder text")
    return value.strip()


def _require_number(mapping: dict[str, Any], key: str, location: str, *, minimum: float = 0.0) -> float:
    value = _require(mapping, key, location)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValidationError(f"{location}.{key}: must be a finite number")
    if value < minimum:
        raise ValidationError(f"{location}.{key}: must be >= {minimum}")
    return float(value)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_manifest(manifest_path: Path, *, allow_template: bool = False) -> dict[str, Any]:
    try:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValidationError(f"cannot read JSON manifest: {exc}") from exc

    if not isinstance(data, dict):
        raise ValidationError("manifest root must be an object")
    if data.get("schema_version") != "1.0":
        raise ValidationError("schema_version must equal '1.0'")

    if data.get("template") is True:
        if not allow_template:
            raise ValidationError("template is not bench evidence; pass --allow-template only to check its shape")
        if data.get("evidence_category") != "template-not-run":
            raise ValidationError("template evidence_category must be 'template-not-run'")
        if data.get("captures") != []:
            raise ValidationError("template captures must be empty")
        return {"kind": "template", "captures": 0, "files_verified": 0}

    if allow_template:
        raise ValidationError("--allow-template is only valid when manifest.template is true")
    if data.get("evidence_category") != "bench-test":
        raise ValidationError("real runs must use evidence_category 'bench-test'")

    _require_text(data, "run_id", "manifest")
    _require_text(data, "captured_at_utc", "manifest")
    _require_text(data, "operator", "manifest")
    _require_text(data, "license_or_consent", "manifest")

    device = _require(data, "device", "manifest")
    if not isinstance(device, dict):
        raise ValidationError("manifest.device must be an object")
    for field in (
        "manufacturer",
        "sku",
        "main_board_revision",
        "expansion_board_revision",
        "camera_sensor_marking",
        "lens_marking",
        "firmware_commit",
        "micro_sd_mpn",
    ):
        _require_text(device, field, "device")
    if device["sku"] != "113991115":
        raise ValidationError("device.sku must identify the current candidate as '113991115'")

    fixture = _require(data, "fixture", "manifest")
    if not isinstance(fixture, dict):
        raise ValidationError("manifest.fixture must be an object")
    for field in ("stand_material", "joint_type", "support_mode", "supply", "usb_cable"):
        _require_text(fixture, field, "fixture")
    for field in (
        "camera_height_mm",
        "arm_span_mm",
        "base_width_mm",
        "base_depth_mm",
        "camera_head_mass_g",
        "ambient_temperature_c",
        "relative_humidity_percent",
    ):
        _require_number(fixture, field, "fixture", minimum=0.0)

    captures = _require(data, "captures", "manifest")
    if not isinstance(captures, list) or not captures:
        raise ValidationError("manifest.captures must be a non-empty array")

    base = manifest_path.parent.resolve()
    seen_ids: set[str] = set()
    counts: Counter[tuple[str, str]] = Counter()
    files_verified = 0
    for index, capture in enumerate(captures):
        location = f"captures[{index}]"
        if not isinstance(capture, dict):
            raise ValidationError(f"{location}: must be an object")
        capture_id = _require_text(capture, "id", location)
        if capture_id in seen_ids:
            raise ValidationError(f"{location}.id: duplicate {capture_id!r}")
        seen_ids.add(capture_id)

        page_size = _require_text(capture, "page_size", location).upper()
        condition = _require_text(capture, "condition", location)
        if page_size not in PAGE_SIZES:
            raise ValidationError(f"{location}.page_size: expected A4 or LETTER")
        if condition not in CONDITION_MINIMUMS:
            raise ValidationError(f"{location}.condition: unsupported {condition!r}")
        counts[(page_size, condition)] += 1

        relative = Path(_require_text(capture, "path", location))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValidationError(f"{location}.path: must stay beneath the manifest directory")
        path = (base / relative).resolve()
        if base not in path.parents:
            raise ValidationError(f"{location}.path: escapes manifest directory")
        if not path.is_file():
            raise ValidationError(f"{location}.path: file does not exist: {relative}")

        expected_bytes = int(_require_number(capture, "bytes", location, minimum=1))
        if path.stat().st_size != expected_bytes:
            raise ValidationError(
                f"{location}.bytes: expected {expected_bytes}, found {path.stat().st_size} for {relative}"
            )
        expected_sha = _require_text(capture, "sha256", location).lower()
        if len(expected_sha) != 64 or any(char not in "0123456789abcdef" for char in expected_sha):
            raise ValidationError(f"{location}.sha256: must be 64 lowercase hexadecimal characters")
        actual_sha = _sha256(path)
        if actual_sha != expected_sha:
            raise ValidationError(f"{location}.sha256: mismatch for {relative}")

        _require_number(capture, "width_px", location, minimum=1)
        _require_number(capture, "height_px", location, minimum=1)
        _require_number(capture, "latency_to_durable_file_ms", location, minimum=0)
        _require_number(capture, "usable_width_px", location, minimum=1)
        _require_number(capture, "usable_height_px", location, minimum=1)
        for field in (
            "center_px_per_mm",
            "top_left_px_per_mm",
            "top_right_px_per_mm",
            "bottom_left_px_per_mm",
            "bottom_right_px_per_mm",
            "max_distortion_px",
            "saturated_pixel_fraction",
        ):
            _require_number(capture, field, location, minimum=0)
        _require_text(capture, "smallest_readable_text_pt", location)
        _require_text(capture, "exposure_settings", location)
        files_verified += 1

    for page_size in sorted(PAGE_SIZES):
        for condition, minimum in CONDITION_MINIMUMS.items():
            actual = counts[(page_size, condition)]
            if actual < minimum:
                raise ValidationError(
                    f"captures: {page_size}/{condition} requires >= {minimum}, found {actual}"
                )

    mechanical = _require(data, "mechanical", "manifest")
    if not isinstance(mechanical, dict):
        raise ValidationError("manifest.mechanical must be an object")
    cycles = _require(mechanical, "refold_cycles", "mechanical")
    if not isinstance(cycles, list) or len(cycles) != 10:
        raise ValidationError("mechanical.refold_cycles must contain exactly ten observations")
    for index, cycle in enumerate(cycles):
        if not isinstance(cycle, dict):
            raise ValidationError(f"mechanical.refold_cycles[{index}] must be an object")
        if cycle.get("cycle") != index + 1:
            raise ValidationError(f"mechanical.refold_cycles[{index}].cycle must equal {index + 1}")
        _require_number(cycle, "x_mm", f"mechanical.refold_cycles[{index}]", minimum=-1000)
        _require_number(cycle, "y_mm", f"mechanical.refold_cycles[{index}]", minimum=-1000)
    _require_number(mechanical, "vertical_deflection_10min_mm", "mechanical", minimum=0)

    power = _require(data, "power_and_thermal", "manifest")
    if not isinstance(power, dict):
        raise ValidationError("manifest.power_and_thermal must be an object")
    for field in (
        "idle_current_ma",
        "capture_peak_current_ma",
        "sd_write_peak_current_ma",
        "usb_transfer_peak_current_ma",
        "illumination_current_ma",
        "combined_peak_current_ma",
        "module_temperature_c",
        "diffuser_temperature_c",
        "max_touchable_temperature_c",
        "test_duration_minutes",
        "current_sample_rate_hz",
    ):
        _require_number(power, field, "power_and_thermal", minimum=0)
    _require_text(power, "instrument", "power_and_thermal")

    return {
        "kind": "bench-test",
        "captures": len(captures),
        "files_verified": files_verified,
        "counts": {f"{page}/{condition}": counts[(page, condition)] for page in sorted(PAGE_SIZES) for condition in CONDITION_MINIMUMS},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--allow-template", action="store_true")
    args = parser.parse_args()
    try:
        summary = validate_manifest(args.manifest, allow_template=args.allow_template)
    except ValidationError as exc:
        print(f"INVALID: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"status": "valid", **summary}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
