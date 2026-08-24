#!/usr/bin/env python3
"""Fail-closed validator for FoldScan optical-spike bench manifests."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import sys
from collections import Counter
from datetime import datetime
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
TEMPLATE_SHAPE = {
    "run_id": str,
    "captured_at_utc": str,
    "operator": str,
    "license_or_consent": str,
    "device": {
        "manufacturer": str,
        "sku": str,
        "main_board_revision": str,
        "expansion_board_revision": str,
        "camera_sensor_marking": str,
        "lens_marking": str,
        "firmware_commit": str,
        "micro_sd_mpn": str,
    },
    "fixture": {
        "stand_material": str,
        "joint_type": str,
        "support_mode": str,
        "supply": str,
        "usb_cable": str,
        "camera_height_mm": "number",
        "arm_span_mm": "number",
        "base_width_mm": "number",
        "base_depth_mm": "number",
        "camera_head_mass_g": "number",
        "ambient_temperature_c": "number",
        "relative_humidity_percent": "number",
    },
    "mechanical": {
        "refold_cycles": list,
        "vertical_deflection_10min_mm": "number",
    },
    "power_and_thermal": {
        "idle_current_ma": "number",
        "capture_peak_current_ma": "number",
        "sd_write_peak_current_ma": "number",
        "usb_transfer_peak_current_ma": "number",
        "illumination_current_ma": "number",
        "combined_peak_current_ma": "number",
        "module_temperature_c": "number",
        "diffuser_temperature_c": "number",
        "max_touchable_temperature_c": "number",
        "test_duration_minutes": "number",
        "current_sample_rate_hz": "number",
        "instrument": str,
    },
}


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


def _require_number(
    mapping: dict[str, Any],
    key: str,
    location: str,
    *,
    minimum: float = 0.0,
    maximum: float | None = None,
    exclusive_minimum: bool = False,
) -> float:
    value = _require(mapping, key, location)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValidationError(f"{location}.{key}: must be a finite number")
    if value < minimum or (exclusive_minimum and value == minimum):
        operator = ">" if exclusive_minimum else ">="
        raise ValidationError(f"{location}.{key}: must be {operator} {minimum}")
    if maximum is not None and value > maximum:
        raise ValidationError(f"{location}.{key}: must be <= {maximum}")
    return float(value)


def _require_integer(
    mapping: dict[str, Any], key: str, location: str, *, minimum: int = 0
) -> int:
    value = _require(mapping, key, location)
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValidationError(f"{location}.{key}: must be an integer")
    if value < minimum:
        raise ValidationError(f"{location}.{key}: must be >= {minimum}")
    return value


def _require_utc_timestamp(mapping: dict[str, Any], key: str, location: str) -> str:
    value = _require_text(mapping, key, location)
    if re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", value) is None:
        raise ValidationError(f"{location}.{key}: must use UTC format YYYY-MM-DDTHH:MM:SSZ")
    try:
        datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError as exc:
        raise ValidationError(
            f"{location}.{key}: must use UTC format YYYY-MM-DDTHH:MM:SSZ"
        ) from exc
    return value


def _require_full_commit(mapping: dict[str, Any], key: str, location: str) -> str:
    value = _require_text(mapping, key, location)
    if len(value) not in {40, 64} or any(char not in "0123456789abcdef" for char in value):
        raise ValidationError(f"{location}.{key}: must be a full lowercase hexadecimal commit ID")
    return value


def _require_template_shape(
    mapping: dict[str, Any], schema: dict[str, Any] = TEMPLATE_SHAPE, location: str = "manifest"
) -> None:
    for key, expected in schema.items():
        value = _require(mapping, key, location)
        field_location = f"{location}.{key}"
        if isinstance(expected, dict):
            if not isinstance(value, dict):
                raise ValidationError(f"{field_location}: template value must be an object")
            _require_template_shape(value, expected, field_location)
        elif expected == "number":
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                raise ValidationError(f"{field_location}: template value must be numeric")
        elif not isinstance(value, expected):
            raise ValidationError(f"{field_location}: template value must be {expected.__name__}")


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
        _require_template_shape(data)
        return {"kind": "template", "captures": 0, "files_verified": 0}

    if allow_template:
        raise ValidationError("--allow-template is only valid when manifest.template is true")
    if data.get("evidence_category") != "bench-test":
        raise ValidationError("real runs must use evidence_category 'bench-test'")

    _require_text(data, "run_id", "manifest")
    _require_utc_timestamp(data, "captured_at_utc", "manifest")
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
        "micro_sd_mpn",
    ):
        _require_text(device, field, "device")
    _require_full_commit(device, "firmware_commit", "device")
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
    ):
        _require_number(fixture, field, "fixture", minimum=0.0, exclusive_minimum=True)
    _require_number(fixture, "ambient_temperature_c", "fixture", minimum=0.0)
    _require_number(fixture, "relative_humidity_percent", "fixture", minimum=0.0, maximum=100.0)

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

        expected_bytes = _require_integer(capture, "bytes", location, minimum=1)
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

        width_px = _require_integer(capture, "width_px", location, minimum=1)
        height_px = _require_integer(capture, "height_px", location, minimum=1)
        _require_number(
            capture, "latency_to_durable_file_ms", location, minimum=0, exclusive_minimum=True
        )
        usable_width_px = _require_integer(capture, "usable_width_px", location, minimum=1)
        usable_height_px = _require_integer(capture, "usable_height_px", location, minimum=1)
        if usable_width_px > width_px:
            raise ValidationError(f"{location}.usable_width_px: cannot exceed width_px")
        if usable_height_px > height_px:
            raise ValidationError(f"{location}.usable_height_px: cannot exceed height_px")
        for field in (
            "center_px_per_mm",
            "top_left_px_per_mm",
            "top_right_px_per_mm",
            "bottom_left_px_per_mm",
            "bottom_right_px_per_mm",
        ):
            _require_number(capture, field, location, minimum=0, exclusive_minimum=True)
        _require_number(capture, "max_distortion_px", location, minimum=0)
        _require_number(capture, "saturated_pixel_fraction", location, minimum=0, maximum=1.0)
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
    current_fields = (
        "idle_current_ma",
        "capture_peak_current_ma",
        "sd_write_peak_current_ma",
        "usb_transfer_peak_current_ma",
        "illumination_current_ma",
        "combined_peak_current_ma",
    )
    currents = {
        field: _require_number(
            power, field, "power_and_thermal", minimum=0, exclusive_minimum=True
        )
        for field in current_fields
    }
    constituent_peak = max(
        value for field, value in currents.items() if field != "combined_peak_current_ma"
    )
    if currents["combined_peak_current_ma"] < constituent_peak:
        raise ValidationError(
            "power_and_thermal.combined_peak_current_ma: cannot be lower than a constituent peak"
        )
    _require_number(
        power, "current_sample_rate_hz", "power_and_thermal", minimum=0, exclusive_minimum=True
    )
    for field in (
        "module_temperature_c",
        "diffuser_temperature_c",
        "max_touchable_temperature_c",
    ):
        _require_number(power, field, "power_and_thermal", minimum=0)
    _require_number(power, "test_duration_minutes", "power_and_thermal", minimum=30)
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
