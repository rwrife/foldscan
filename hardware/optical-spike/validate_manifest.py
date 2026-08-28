#!/usr/bin/env python3
"""Fail-closed validator for FoldScan optical-spike bench manifests."""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import math
import os
import re
import stat
import statistics
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
SUPPORTED_SOF_MARKERS = {0xC0}
ALLOWED_SEGMENT_MARKERS = {0xC4, 0xDB, 0xDD, 0xFE, *range(0xE0, 0xF0)}
MAX_CAPTURE_BYTES = 50 * 1024 * 1024
MAX_MEASUREMENT_LOG_BYTES = 10 * 1024 * 1024
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
    "measurement_logs": {
        "mechanical": {"path": str, "bytes": int, "sha256": str},
        "current": {"path": str, "bytes": int, "sha256": str},
        "temperature": {"path": str, "bytes": int, "sha256": str},
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


def _read_confined_file(
    base: Path, relative: Path, location: str, expected_bytes: int, *, kind: str
) -> tuple[bytes, os.stat_result]:
    """Read a regular file through a no-symlink descriptor walk rooted at / on POSIX."""
    if (
        os.name != "posix"
        or os.open not in os.supports_dir_fd
        or not hasattr(os, "O_DIRECTORY")
        or not hasattr(os, "O_NOFOLLOW")
    ):
        raise ValidationError(
            f"{location}.path: platform cannot securely read {kind} without symlink traversal"
        )

    directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0)
    file_flags = os.O_RDONLY | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0)
    directory_fd = -1
    try:
        directory_fd = os.open(base.anchor, directory_flags)
        directory_parts = [*base.parts[1:], *relative.parts[:-1]]
        for part in directory_parts:
            next_fd = os.open(part, directory_flags, dir_fd=directory_fd)
            os.close(directory_fd)
            directory_fd = next_fd

        descriptor = os.open(relative.name, file_flags, dir_fd=directory_fd)
        with os.fdopen(descriptor, "rb") as stream:
            before_stat = os.fstat(stream.fileno())
            if not stat.S_ISREG(before_stat.st_mode):
                raise ValidationError(f"{location}.path: {kind} must be a regular file")
            if before_stat.st_size != expected_bytes:
                raise ValidationError(
                    f"{location}.bytes: expected {expected_bytes}, found {before_stat.st_size} "
                    f"for {relative}"
                )
            payload = stream.read(expected_bytes)
            after_stat = os.fstat(stream.fileno())
    except OSError as exc:
        raise ValidationError(f"{location}.path: cannot securely read {kind} {relative}: {exc}") from exc
    finally:
        if directory_fd >= 0:
            os.close(directory_fd)

    before_fingerprint = (
        before_stat.st_dev,
        before_stat.st_ino,
        before_stat.st_size,
        before_stat.st_mtime_ns,
        before_stat.st_ctime_ns,
    )
    after_fingerprint = (
        after_stat.st_dev,
        after_stat.st_ino,
        after_stat.st_size,
        after_stat.st_mtime_ns,
        after_stat.st_ctime_ns,
    )
    if before_fingerprint != after_fingerprint or len(payload) != after_stat.st_size:
        raise ValidationError(f"{location}.path: {kind} changed while it was being read")
    return payload, after_stat


def _require_sha256(mapping: dict[str, Any], key: str, location: str) -> str:
    value = _require_text(mapping, key, location).lower()
    if len(value) != 64 or any(char not in "0123456789abcdef" for char in value):
        raise ValidationError(f"{location}.{key}: must be 64 lowercase hexadecimal characters")
    return value


def _parse_csv_number(value: str, location: str, *, minimum: float | None = None) -> float:
    try:
        number = float(value)
    except ValueError as exc:
        raise ValidationError(f"{location}: must be a finite number") from exc
    if not math.isfinite(number):
        raise ValidationError(f"{location}: must be a finite number")
    if minimum is not None and number < minimum:
        raise ValidationError(f"{location}: must be >= {minimum}")
    return number


def _csv_rows(payload: bytes, role: str, expected_header: tuple[str, ...]) -> list[dict[str, str]]:
    location = f"measurement_logs.{role}"
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ValidationError(f"{location}: must be UTF-8 CSV") from exc
    if "\x00" in text:
        raise ValidationError(f"{location}: CSV must not contain NUL bytes")
    try:
        reader = csv.DictReader(io.StringIO(text, newline=""))
        if tuple(reader.fieldnames or ()) != expected_header:
            raise ValidationError(
                f"{location}: {role} CSV header must equal {','.join(expected_header)}"
            )
        rows = list(reader)
    except csv.Error as exc:
        raise ValidationError(f"{location}: invalid CSV: {exc}") from exc
    if not rows:
        raise ValidationError(f"{location}: CSV must contain measurement rows")
    if any(None in row or any(value is None for value in row.values()) for row in rows):
        raise ValidationError(f"{location}: CSV rows must match the declared header")
    return [{key: value.strip() for key, value in row.items()} for row in rows]


def _assert_logged_number(claimed: float, logged: float, location: str, source: str) -> None:
    if not math.isclose(claimed, logged, rel_tol=1e-9, abs_tol=1e-9):
        raise ValidationError(f"{location}: {claimed} does not match {source} {logged}")


def _validate_measurement_logs(
    base: Path,
    logs: Any,
    mechanical: dict[str, Any],
    power: dict[str, Any],
    seen_paths: dict[Path, str],
    seen_file_ids: dict[tuple[int, int], str],
) -> int:
    required_roles = {"mechanical", "current", "temperature"}
    if not isinstance(logs, dict) or set(logs) != required_roles:
        raise ValidationError(
            "manifest.measurement_logs must contain exactly mechanical, current, and temperature"
        )

    payloads: dict[str, bytes] = {}
    for role in ("mechanical", "current", "temperature"):
        location = f"measurement_logs.{role}"
        spec = logs[role]
        if not isinstance(spec, dict):
            raise ValidationError(f"{location}: must be an object")
        relative = Path(_require_text(spec, "path", location))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValidationError(f"{location}.path: must stay beneath the manifest directory")
        if relative in seen_paths:
            raise ValidationError(
                f"{location}.path: reuses evidence file from {seen_paths[relative]!r}: {relative}"
            )
        expected_bytes = _require_integer(spec, "bytes", location, minimum=1)
        if expected_bytes > MAX_MEASUREMENT_LOG_BYTES:
            raise ValidationError(
                f"{location}.bytes: {expected_bytes} exceeds maximum {MAX_MEASUREMENT_LOG_BYTES}"
            )
        payload, file_stat = _read_confined_file(
            base, relative, location, expected_bytes, kind="measurement log"
        )
        file_id = (file_stat.st_dev, file_stat.st_ino)
        if file_stat.st_ino and file_id in seen_file_ids:
            raise ValidationError(
                f"{location}.path: reuses physical evidence file from "
                f"{seen_file_ids[file_id]!r}: {relative}"
            )
        expected_sha = _require_sha256(spec, "sha256", location)
        if hashlib.sha256(payload).hexdigest() != expected_sha:
            raise ValidationError(f"{location}.sha256: mismatch for {relative}")
        seen_paths[relative] = location
        if file_stat.st_ino:
            seen_file_ids[file_id] = location
        payloads[role] = payload

    _validate_mechanical_log(payloads["mechanical"], mechanical)
    _validate_current_log(payloads["current"], power)
    _validate_temperature_log(payloads["temperature"], power)
    return len(payloads)


def _validate_mechanical_log(payload: bytes, mechanical: dict[str, Any]) -> None:
    rows = _csv_rows(
        payload,
        "mechanical",
        ("measurement", "cycle", "x_mm", "y_mm", "elapsed_minutes", "vertical_deflection_mm"),
    )
    logged_cycles: dict[int, tuple[float, float]] = {}
    logged_deflection: float | None = None
    for row_index, row in enumerate(rows, start=2):
        location = f"measurement_logs.mechanical row {row_index}"
        if row["measurement"] == "refold":
            if row["elapsed_minutes"] or row["vertical_deflection_mm"]:
                raise ValidationError(f"{location}: refold rows cannot contain deflection fields")
            try:
                cycle = int(row["cycle"])
            except ValueError as exc:
                raise ValidationError(f"{location}.cycle: must be an integer") from exc
            if str(cycle) != row["cycle"] or cycle not in range(1, 11) or cycle in logged_cycles:
                raise ValidationError(f"{location}.cycle: must be a unique integer from 1 through 10")
            logged_cycles[cycle] = (
                _parse_csv_number(row["x_mm"], f"{location}.x_mm"),
                _parse_csv_number(row["y_mm"], f"{location}.y_mm"),
            )
        elif row["measurement"] == "deflection":
            if any(row[field] for field in ("cycle", "x_mm", "y_mm")):
                raise ValidationError(f"{location}: deflection row cannot contain refold fields")
            if logged_deflection is not None:
                raise ValidationError("measurement_logs.mechanical: requires exactly one deflection row")
            elapsed = _parse_csv_number(row["elapsed_minutes"], f"{location}.elapsed_minutes", minimum=0)
            _assert_logged_number(elapsed, 10.0, f"{location}.elapsed_minutes", "required interval")
            logged_deflection = _parse_csv_number(
                row["vertical_deflection_mm"], f"{location}.vertical_deflection_mm", minimum=0
            )
        else:
            raise ValidationError(f"{location}.measurement: expected refold or deflection")
    if set(logged_cycles) != set(range(1, 11)) or logged_deflection is None:
        raise ValidationError(
            "measurement_logs.mechanical: requires refold cycles 1 through 10 and one deflection row"
        )
    for index, claim in enumerate(mechanical["refold_cycles"]):
        logged_x, logged_y = logged_cycles[index + 1]
        _assert_logged_number(
            float(claim["x_mm"]), logged_x, f"mechanical.refold_cycles[{index}].x_mm", "mechanical log"
        )
        _assert_logged_number(
            float(claim["y_mm"]), logged_y, f"mechanical.refold_cycles[{index}].y_mm", "mechanical log"
        )
    _assert_logged_number(
        float(mechanical["vertical_deflection_10min_mm"]),
        logged_deflection,
        "mechanical.vertical_deflection_10min_mm",
        "mechanical log",
    )


def _validate_current_log(payload: bytes, power: dict[str, Any]) -> None:
    rows = _csv_rows(payload, "current", ("sample_index", "elapsed_ms", "state", "current_ma"))
    state_to_field = {
        "idle": "idle_current_ma",
        "capture": "capture_peak_current_ma",
        "sd-write": "sd_write_peak_current_ma",
        "usb-transfer": "usb_transfer_peak_current_ma",
        "illumination": "illumination_current_ma",
        "combined": "combined_peak_current_ma",
    }
    samples: dict[str, list[float]] = {state: [] for state in state_to_field}
    elapsed_values: list[float] = []
    for row_index, row in enumerate(rows, start=2):
        location = f"measurement_logs.current row {row_index}"
        try:
            sample_index = int(row["sample_index"])
        except ValueError as exc:
            raise ValidationError(f"{location}.sample_index: must be an integer") from exc
        if str(sample_index) != row["sample_index"] or sample_index != row_index - 1:
            raise ValidationError(f"{location}.sample_index: must be sequential from 1")
        elapsed = _parse_csv_number(row["elapsed_ms"], f"{location}.elapsed_ms", minimum=0)
        if elapsed_values and elapsed <= elapsed_values[-1]:
            raise ValidationError(f"{location}.elapsed_ms: must increase strictly")
        elapsed_values.append(elapsed)
        state = row["state"]
        if state not in samples:
            raise ValidationError(f"{location}.state: unsupported {state!r}")
        samples[state].append(
            _parse_csv_number(row["current_ma"], f"{location}.current_ma", minimum=0)
        )
    for state, field in state_to_field.items():
        if not samples[state]:
            raise ValidationError(f"measurement_logs.current: missing {state!r} samples")
        _assert_logged_number(
            float(power[field]), max(samples[state]), f"power_and_thermal.{field}", "logged maximum"
        )
    intervals = [right - left for left, right in zip(elapsed_values, elapsed_values[1:])]
    if not intervals:
        raise ValidationError("measurement_logs.current: requires at least two timed samples")
    logged_sample_rate_hz = 1000.0 / statistics.median(intervals)
    if not math.isclose(
        float(power["current_sample_rate_hz"]),
        logged_sample_rate_hz,
        rel_tol=0.01,
        abs_tol=0.01,
    ):
        raise ValidationError(
            "power_and_thermal.current_sample_rate_hz: does not match logged median interval "
            f"({logged_sample_rate_hz} Hz) within 1%"
        )


def _validate_temperature_log(payload: bytes, power: dict[str, Any]) -> None:
    rows = _csv_rows(payload, "temperature", ("elapsed_minutes", "location", "temperature_c"))
    location_to_field = {
        "module": "module_temperature_c",
        "diffuser": "diffuser_temperature_c",
        "touchable": "max_touchable_temperature_c",
    }
    temperatures: dict[str, list[float]] = {location: [] for location in location_to_field}
    elapsed_minutes: list[float] = []
    for row_index, row in enumerate(rows, start=2):
        location = f"measurement_logs.temperature row {row_index}"
        elapsed_minutes.append(
            _parse_csv_number(row["elapsed_minutes"], f"{location}.elapsed_minutes", minimum=0)
        )
        location_name = row["location"]
        if location_name not in temperatures:
            raise ValidationError(f"{location}.location: unsupported {location_name!r}")
        temperatures[location_name].append(
            _parse_csv_number(row["temperature_c"], f"{location}.temperature_c")
        )
    for location_name, field in location_to_field.items():
        if not temperatures[location_name]:
            raise ValidationError(f"measurement_logs.temperature: missing {location_name!r} samples")
        _assert_logged_number(
            float(power[field]),
            max(temperatures[location_name]),
            f"power_and_thermal.{field}",
            "logged maximum",
        )
    _assert_logged_number(
        float(power["test_duration_minutes"]),
        max(elapsed_minutes),
        "power_and_thermal.test_duration_minutes",
        "logged duration",
    )


def _jpeg_dimensions(payload: bytes, display_name: str) -> tuple[int, int]:
    """Validate JPEG marker structure and return dimensions from its frame header."""
    if not payload.startswith(b"\xff\xd8"):
        raise ValidationError(f"JPEG file {display_name!r}: missing start-of-image marker")

    position = 2
    dimensions: tuple[int, int] | None = None
    frame_components: set[int] | None = None
    scanned_components: set[int] = set()
    saw_scan = False
    pending_marker: int | None = None

    def read_segment(marker: int) -> tuple[int, int]:
        nonlocal position
        if position + 2 > len(payload):
            raise ValidationError(f"JPEG file {display_name!r}: truncated marker length")
        segment_length = int.from_bytes(payload[position : position + 2], "big")
        if segment_length < 2:
            raise ValidationError(f"JPEG file {display_name!r}: invalid marker length")
        start = position + 2
        end = position + segment_length
        if end > len(payload):
            raise ValidationError(f"JPEG file {display_name!r}: truncated marker 0x{marker:02x}")
        position = end
        return start, end

    while True:
        if pending_marker is None:
            if position >= len(payload):
                suffix = "end-of-image marker" if saw_scan else "image scan and end-of-image marker"
                raise ValidationError(f"JPEG file {display_name!r}: missing {suffix}")
            if payload[position] != 0xFF:
                raise ValidationError(f"JPEG file {display_name!r}: invalid marker stream")
            while position < len(payload) and payload[position] == 0xFF:
                position += 1
            if position >= len(payload) or payload[position] == 0x00:
                raise ValidationError(f"JPEG file {display_name!r}: invalid marker")
            marker = payload[position]
            position += 1
        else:
            marker = pending_marker
            pending_marker = None

        if marker == 0xD8:
            raise ValidationError(f"JPEG file {display_name!r}: repeated start-of-image marker")
        if marker == 0xD9:
            if dimensions is None:
                raise ValidationError(f"JPEG file {display_name!r}: ended before frame header")
            if not saw_scan:
                raise ValidationError(f"JPEG file {display_name!r}: ended before image scan")
            if scanned_components != frame_components:
                raise ValidationError(f"JPEG file {display_name!r}: image scans are missing frame components")
            if position != len(payload):
                raise ValidationError(f"JPEG file {display_name!r}: trailing data after end-of-image")
            return dimensions
        if marker == 0x01 or 0xD0 <= marker <= 0xD7:
            raise ValidationError(f"JPEG file {display_name!r}: standalone marker outside image scan")

        if marker == 0xDA:
            if dimensions is None:
                raise ValidationError(f"JPEG file {display_name!r}: image scan begins before frame header")
            start, end = read_segment(marker)
            scan_components = payload[start] if start < end else 0
            expected_length = 1 + (2 * scan_components) + 3
            if scan_components == 0 or end - start != expected_length:
                raise ValidationError(f"JPEG file {display_name!r}: invalid image scan header")
            selectors = [payload[start + 1 + (2 * index)] for index in range(scan_components)]
            if len(set(selectors)) != len(selectors) or not set(selectors).issubset(frame_components or set()):
                raise ValidationError(f"JPEG file {display_name!r}: invalid scan component selector")
            scanned_components.update(selectors)
            spectral_start, spectral_end, approximation = payload[end - 3 : end]
            if (spectral_start, spectral_end, approximation) != (0, 63, 0):
                raise ValidationError(f"JPEG file {display_name!r}: invalid baseline scan parameters")
            saw_scan = True

            saw_entropy_data = False
            while position < len(payload):
                marker_start = payload.find(b"\xff", position)
                if marker_start < 0:
                    if position < len(payload):
                        saw_entropy_data = True
                    break
                if marker_start > position:
                    saw_entropy_data = True
                position = marker_start + 1
                while position < len(payload) and payload[position] == 0xFF:
                    position += 1
                if position >= len(payload):
                    break
                scan_marker = payload[position]
                position += 1
                if scan_marker == 0x00:
                    saw_entropy_data = True
                    continue
                if 0xD0 <= scan_marker <= 0xD7:
                    continue
                pending_marker = scan_marker
                break
            if not saw_entropy_data:
                raise ValidationError(f"JPEG file {display_name!r}: empty image scan")
            if pending_marker is None:
                raise ValidationError(f"JPEG file {display_name!r}: missing end-of-image marker")
            continue

        if marker not in SUPPORTED_SOF_MARKERS and marker not in ALLOWED_SEGMENT_MARKERS:
            raise ValidationError(f"JPEG file {display_name!r}: unsupported marker 0x{marker:02x}")
        start, end = read_segment(marker)
        if marker not in SUPPORTED_SOF_MARKERS:
            if marker == 0xDD and end - start != 2:
                raise ValidationError(f"JPEG file {display_name!r}: invalid restart interval header")
            continue
        if dimensions is not None:
            raise ValidationError(f"JPEG file {display_name!r}: duplicate frame header")
        frame = payload[start:end]
        if len(frame) < 6:
            raise ValidationError(f"JPEG file {display_name!r}: truncated frame header")
        if frame[0] != 8:
            raise ValidationError(f"JPEG file {display_name!r}: baseline sample precision must be 8 bits")
        height = int.from_bytes(frame[1:3], "big")
        width = int.from_bytes(frame[3:5], "big")
        component_count = frame[5]
        if width == 0 or height == 0 or component_count == 0:
            raise ValidationError(f"JPEG file {display_name!r}: invalid encoded dimensions")
        if len(frame) != 6 + (3 * component_count):
            raise ValidationError(f"JPEG file {display_name!r}: invalid component table length")
        component_ids = {frame[6 + (3 * index)] for index in range(component_count)}
        if len(component_ids) != component_count:
            raise ValidationError(f"JPEG file {display_name!r}: duplicate frame component identifier")
        block_count = 0
        for index in range(component_count):
            sampling = frame[7 + (3 * index)]
            horizontal = sampling >> 4
            vertical = sampling & 0x0F
            quantization_table = frame[8 + (3 * index)]
            if not (1 <= horizontal <= 4 and 1 <= vertical <= 4):
                raise ValidationError(f"JPEG file {display_name!r}: invalid component sampling factor")
            if quantization_table > 3:
                raise ValidationError(f"JPEG file {display_name!r}: invalid quantization table selector")
            block_count += horizontal * vertical
        if block_count > 10:
            raise ValidationError(f"JPEG file {display_name!r}: frame sampling factors exceed MCU limit")
        dimensions = (width, height)
        frame_components = component_ids


def validate_manifest(manifest_path: Path, *, allow_template: bool = False) -> dict[str, Any]:
    try:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValidationError(f"cannot read JSON manifest: {exc}") from exc

    if not isinstance(data, dict):
        raise ValidationError("manifest root must be an object")
    if data.get("schema_version") != "1.1":
        raise ValidationError("schema_version must equal '1.1'")

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
    seen_paths: dict[Path, str] = {}
    seen_file_ids: dict[tuple[int, int], str] = {}
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
        if relative in seen_paths:
            raise ValidationError(
                f"{location}.path: reuses capture file from {seen_paths[relative]!r}: {relative}"
            )
        seen_paths[relative] = capture_id

        expected_bytes = _require_integer(capture, "bytes", location, minimum=1)
        if expected_bytes > MAX_CAPTURE_BYTES:
            raise ValidationError(
                f"{location}.bytes: {expected_bytes} exceeds maximum {MAX_CAPTURE_BYTES}"
            )
        payload, file_stat = _read_confined_file(
            base, relative, location, expected_bytes, kind="capture"
        )

        file_id = (file_stat.st_dev, file_stat.st_ino)
        if file_stat.st_ino and file_id in seen_file_ids:
            raise ValidationError(
                f"{location}.path: reuses physical capture file from "
                f"{seen_file_ids[file_id]!r}: {relative}"
            )
        if file_stat.st_ino:
            seen_file_ids[file_id] = capture_id

        if len(payload) != expected_bytes:
            raise ValidationError(
                f"{location}.bytes: expected {expected_bytes}, found {len(payload)} for {relative}"
            )
        expected_sha = _require_sha256(capture, "sha256", location)
        actual_sha = hashlib.sha256(payload).hexdigest()
        if actual_sha != expected_sha:
            raise ValidationError(f"{location}.sha256: mismatch for {relative}")

        width_px = _require_integer(capture, "width_px", location, minimum=1)
        height_px = _require_integer(capture, "height_px", location, minimum=1)
        encoded_width_px, encoded_height_px = _jpeg_dimensions(payload, relative.as_posix())
        if (width_px, height_px) != (encoded_width_px, encoded_height_px):
            raise ValidationError(
                f"{location}: claimed dimensions {width_px}x{height_px} do not match "
                f"encoded JPEG dimensions {encoded_width_px}x{encoded_height_px}"
            )
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

    measurement_logs_verified = _validate_measurement_logs(
        base,
        _require(data, "measurement_logs", "manifest"),
        mechanical,
        power,
        seen_paths,
        seen_file_ids,
    )

    return {
        "kind": "bench-test",
        "captures": len(captures),
        "capture_files_verified": files_verified,
        "measurement_logs_verified": measurement_logs_verified,
        "files_verified": files_verified + measurement_logs_verified,
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
