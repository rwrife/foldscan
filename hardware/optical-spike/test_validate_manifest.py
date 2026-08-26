#!/usr/bin/env python3

import hashlib
import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

from validate_manifest import ValidationError, validate_manifest


JPEG_FIXTURE = (Path(__file__).with_name("test-fixtures") / "white-16x16.jpg").read_bytes()


class ManifestValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    @staticmethod
    def _jpeg_bytes(width: int, height: int, capture_id: str) -> bytes:
        """Add a unique JPEG comment to a genuinely decodable 16x16 fixture."""
        if (width, height) != (16, 16):
            raise ValueError("test JPEG fixture is 16x16")
        comment = capture_id.encode("ascii")
        return (
            JPEG_FIXTURE[:-2]
            + b"\xff\xfe"
            + (len(comment) + 2).to_bytes(2, "big")
            + comment
            + JPEG_FIXTURE[-2:]
        )

    def _valid_manifest(self) -> dict:
        raw = self.root / "raw"
        raw.mkdir(exist_ok=True)
        captures = []
        minimums = {
            "symmetric-matte": 30,
            "ambient-only": 3,
            "left-light-only": 3,
            "right-light-only": 3,
            "symmetric-glossy": 3,
        }
        sequence = 0
        for page_size in ("A4", "LETTER"):
            for condition, count in minimums.items():
                for _ in range(count):
                    sequence += 1
                    capture_id = f"capture-{sequence:03d}"
                    payload = self._jpeg_bytes(16, 16, capture_id)
                    path = raw / f"{capture_id}.jpg"
                    path.write_bytes(payload)
                    captures.append(
                        {
                            "id": capture_id,
                            "page_size": page_size,
                            "condition": condition,
                            "path": f"raw/{capture_id}.jpg",
                            "bytes": len(payload),
                            "sha256": hashlib.sha256(payload).hexdigest(),
                            "width_px": 16,
                            "height_px": 16,
                            "latency_to_durable_file_ms": 1250,
                            "usable_width_px": 15,
                            "usable_height_px": 14,
                            "center_px_per_mm": 6.4,
                            "top_left_px_per_mm": 6.2,
                            "top_right_px_per_mm": 6.3,
                            "bottom_left_px_per_mm": 6.2,
                            "bottom_right_px_per_mm": 6.3,
                            "max_distortion_px": 12,
                            "saturated_pixel_fraction": 0.01,
                            "smallest_readable_text_pt": "8 pt",
                            "exposure_settings": "gain=recorded; exposure=recorded; jpeg_quality=recorded",
                        }
                    )
        return {
            "schema_version": "1.0",
            "template": False,
            "evidence_category": "bench-test",
            "run_id": "20260824T120000Z-ov3660-spike",
            "captured_at_utc": "2026-08-24T12:00:00Z",
            "operator": "fixture operator",
            "license_or_consent": "CC BY-SA 4.0 project-owned test target",
            "device": {
                "manufacturer": "Seeed Studio",
                "sku": "113991115",
                "main_board_revision": "V1.3 physical silkscreen",
                "expansion_board_revision": "V1.3 physical silkscreen",
                "camera_sensor_marking": "OV3660 physical marking",
                "lens_marking": "no marking visible after photographed inspection",
                "firmware_commit": "0123456789abcdef0123456789abcdef01234567",
                "micro_sd_mpn": "Example exact test MPN",
            },
            "fixture": {
                "stand_material": "12 mm plywood",
                "joint_type": "clamped right-angle bracket",
                "support_mode": "desk-clamped",
                "supply": "example certified 5 V supply",
                "usb_cable": "example 1 m USB-C data cable",
                "camera_height_mm": 420,
                "arm_span_mm": 180,
                "base_width_mm": 330,
                "base_depth_mm": 250,
                "camera_head_mass_g": 45,
                "ambient_temperature_c": 23,
                "relative_humidity_percent": 45,
            },
            "captures": captures,
            "mechanical": {
                "refold_cycles": [
                    {"cycle": cycle, "x_mm": cycle / 10, "y_mm": -cycle / 20}
                    for cycle in range(1, 11)
                ],
                "vertical_deflection_10min_mm": 0.8,
            },
            "power_and_thermal": {
                "idle_current_ma": 50,
                "capture_peak_current_ma": 400,
                "sd_write_peak_current_ma": 180,
                "usb_transfer_peak_current_ma": 220,
                "illumination_current_ma": 900,
                "combined_peak_current_ma": 1250,
                "module_temperature_c": 42,
                "diffuser_temperature_c": 36,
                "max_touchable_temperature_c": 38,
                "test_duration_minutes": 30,
                "current_sample_rate_hz": 1000,
                "instrument": "example logged USB power meter",
            },
        }

    def _write(self, data: dict, name: str = "manifest.json") -> Path:
        path = self.root / name
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def _replace_first_capture_payload(self, data: dict, payload: bytes) -> None:
        capture = data["captures"][0]
        (self.root / capture["path"]).write_bytes(payload)
        capture["bytes"] = len(payload)
        capture["sha256"] = hashlib.sha256(payload).hexdigest()

    def _template_manifest(self) -> dict:
        template_path = Path(__file__).with_name("manifest.example.json")
        return json.loads(template_path.read_text(encoding="utf-8"))

    def test_valid_manifest_verifies_all_files(self) -> None:
        path = self._write(self._valid_manifest())
        result = validate_manifest(path)
        self.assertEqual(result["kind"], "bench-test")
        self.assertEqual(result["captures"], 84)
        self.assertEqual(result["files_verified"], 84)

    def test_checksum_mismatch_fails(self) -> None:
        data = self._valid_manifest()
        data["captures"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValidationError, "mismatch"):
            validate_manifest(self._write(data))

    def test_non_jpeg_payload_cannot_claim_capture_dimensions(self) -> None:
        data = self._valid_manifest()
        payload = b"not a jpeg image"
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "JPEG"):
            validate_manifest(self._write(data))

    def test_claimed_dimensions_must_match_jpeg_header(self) -> None:
        data = self._valid_manifest()
        data["captures"][0]["width_px"] = 2200
        with self.assertRaisesRegex(ValidationError, "encoded JPEG dimensions"):
            validate_manifest(self._write(data))

    def test_capture_file_cannot_be_reused_under_another_id(self) -> None:
        data = self._valid_manifest()
        first = data["captures"][0]
        second = data["captures"][1]
        for field in ("path", "bytes", "sha256", "width_px", "height_px"):
            second[field] = first[field]
        with self.assertRaisesRegex(ValidationError, "reuses capture file"):
            validate_manifest(self._write(data))

    def test_capture_file_cannot_be_reused_through_hard_link(self) -> None:
        data = self._valid_manifest()
        first = data["captures"][0]
        second = data["captures"][1]
        first_path = self.root / first["path"]
        second_path = self.root / second["path"]
        second_path.unlink()
        second_path.hardlink_to(first_path)
        for field in ("bytes", "sha256", "width_px", "height_px"):
            second[field] = first[field]
        with self.assertRaisesRegex(ValidationError, "reuses physical capture file"):
            validate_manifest(self._write(data))

    def test_truncated_sof_component_table_is_not_a_jpeg_header(self) -> None:
        data = self._valid_manifest()
        frame_prefix = b"\x08\x00\x10\x00\x10\x03"
        payload = b"\xff\xd8\xff\xc0\x00\x11" + frame_prefix
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "truncated (?:marker|component table)"):
            validate_manifest(self._write(data))

    def test_complete_sof_without_scan_is_not_a_jpeg_file(self) -> None:
        data = self._valid_manifest()
        components = b"\x01\x11\x00\x02\x11\x00\x03\x11\x00"
        frame = b"\x08\x00\x10\x00\x10\x03" + components
        payload = b"\xff\xd8\xff\xc0\x00\x11" + frame + b"\xff\xd9"
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "image scan"):
            validate_manifest(self._write(data))

    def test_missing_end_of_image_marker_is_not_a_jpeg_file(self) -> None:
        data = self._valid_manifest()
        self._replace_first_capture_payload(data, JPEG_FIXTURE[:-2])
        with self.assertRaisesRegex(ValidationError, "end-of-image"):
            validate_manifest(self._write(data))

    def test_repeated_start_of_image_marker_is_rejected(self) -> None:
        data = self._valid_manifest()
        payload = JPEG_FIXTURE[:2] + b"\xff\xd8" + JPEG_FIXTURE[2:]
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "repeated start-of-image"):
            validate_manifest(self._write(data))

    def test_reserved_marker_is_rejected(self) -> None:
        data = self._valid_manifest()
        payload = JPEG_FIXTURE[:2] + b"\xff\x02\x00\x02" + JPEG_FIXTURE[2:]
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "unsupported marker"):
            validate_manifest(self._write(data))

    def test_scan_selector_must_exist_in_frame_components(self) -> None:
        data = self._valid_manifest()
        payload = bytearray(JPEG_FIXTURE)
        scan_marker = payload.index(b"\xff\xda")
        first_selector = scan_marker + 5
        payload[first_selector] = 9
        self._replace_first_capture_payload(data, bytes(payload))
        with self.assertRaisesRegex(ValidationError, "scan component selector"):
            validate_manifest(self._write(data))

    def test_empty_image_scan_is_rejected(self) -> None:
        data = self._valid_manifest()
        components = b"\x01\x11\x00\x02\x11\x00\x03\x11\x00"
        frame = b"\x08\x00\x10\x00\x10\x03" + components
        scan = b"\x03\x01\x00\x02\x00\x03\x00\x00\x3f\x00"
        payload = b"\xff\xd8\xff\xc0\x00\x11" + frame + b"\xff\xda\x00\x0c" + scan + b"\xff\xd9"
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "empty image scan"):
            validate_manifest(self._write(data))

    def test_symlinked_capture_directory_is_rejected(self) -> None:
        data = self._valid_manifest()
        (self.root / "alias").symlink_to(self.root / "raw", target_is_directory=True)
        data["captures"][0]["path"] = "alias/capture-001.jpg"
        with self.assertRaisesRegex(ValidationError, "securely read capture"):
            validate_manifest(self._write(data))

    def test_declared_capture_size_has_a_safe_upper_bound(self) -> None:
        data = self._valid_manifest()
        data["captures"][0]["bytes"] = 50 * 1024 * 1024 + 1
        with self.assertRaisesRegex(ValidationError, "exceeds maximum"):
            validate_manifest(self._write(data))

    def test_baseline_frame_requires_eight_bit_precision(self) -> None:
        data = self._valid_manifest()
        payload = bytearray(JPEG_FIXTURE)
        frame_marker = payload.index(b"\xff\xc0")
        payload[frame_marker + 4] = 12
        self._replace_first_capture_payload(data, bytes(payload))
        with self.assertRaisesRegex(ValidationError, "sample precision"):
            validate_manifest(self._write(data))

    def test_frame_sampling_factor_cannot_be_zero(self) -> None:
        data = self._valid_manifest()
        payload = bytearray(JPEG_FIXTURE)
        frame_marker = payload.index(b"\xff\xc0")
        payload[frame_marker + 11] = 0
        self._replace_first_capture_payload(data, bytes(payload))
        with self.assertRaisesRegex(ValidationError, "sampling factor"):
            validate_manifest(self._write(data))

    def test_every_frame_component_must_appear_in_a_scan(self) -> None:
        data = self._valid_manifest()
        components = b"\x01\x11\x00\x02\x11\x00\x03\x11\x00"
        frame = b"\x08\x00\x10\x00\x10\x03" + components
        scan = b"\x01\x01\x00\x00\x3f\x00"
        payload = b"\xff\xd8\xff\xc0\x00\x11" + frame + b"\xff\xda\x00\x08" + scan + b"\x01\xff\xd9"
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "missing frame components"):
            validate_manifest(self._write(data))

    def test_restart_interval_marker_requires_two_byte_payload(self) -> None:
        data = self._valid_manifest()
        payload = JPEG_FIXTURE[:2] + b"\xff\xdd\x00\x03\x00" + JPEG_FIXTURE[2:]
        self._replace_first_capture_payload(data, payload)
        with self.assertRaisesRegex(ValidationError, "restart interval"):
            validate_manifest(self._write(data))

    def test_missing_primary_condition_fails(self) -> None:
        data = self._valid_manifest()
        data["captures"] = [
            capture
            for capture in data["captures"]
            if not (capture["page_size"] == "LETTER" and capture["condition"] == "symmetric-glossy")
        ]
        with self.assertRaisesRegex(ValidationError, "LETTER/symmetric-glossy"):
            validate_manifest(self._write(data))

    def test_template_requires_explicit_flag(self) -> None:
        template = self._template_manifest()
        path = self._write(template)
        with self.assertRaisesRegex(ValidationError, "not bench evidence"):
            validate_manifest(path)
        self.assertEqual(validate_manifest(path, allow_template=True)["kind"], "template")

    def test_template_requires_complete_manifest_shape(self) -> None:
        template = self._template_manifest()
        del template["device"]
        with self.assertRaisesRegex(ValidationError, "device"):
            validate_manifest(self._write(template), allow_template=True)

    def test_path_escape_fails_before_file_access(self) -> None:
        data = self._valid_manifest()
        data["captures"][0]["path"] = "../private.jpg"
        with self.assertRaisesRegex(ValidationError, "stay beneath"):
            validate_manifest(self._write(data))

    def test_zero_duration_does_not_count_as_bench_measurement(self) -> None:
        data = self._valid_manifest()
        data["power_and_thermal"]["test_duration_minutes"] = 0
        with self.assertRaisesRegex(ValidationError, "test_duration_minutes"):
            validate_manifest(self._write(data))

    def test_required_measurements_must_be_positive(self) -> None:
        fields = (
            ("fixture", "camera_height_mm"),
            ("fixture", "arm_span_mm"),
            ("fixture", "base_width_mm"),
            ("fixture", "base_depth_mm"),
            ("fixture", "camera_head_mass_g"),
            ("capture", "latency_to_durable_file_ms"),
            ("capture", "center_px_per_mm"),
            ("capture", "top_left_px_per_mm"),
            ("capture", "top_right_px_per_mm"),
            ("capture", "bottom_left_px_per_mm"),
            ("capture", "bottom_right_px_per_mm"),
            ("power_and_thermal", "idle_current_ma"),
            ("power_and_thermal", "capture_peak_current_ma"),
            ("power_and_thermal", "sd_write_peak_current_ma"),
            ("power_and_thermal", "usb_transfer_peak_current_ma"),
            ("power_and_thermal", "illumination_current_ma"),
            ("power_and_thermal", "combined_peak_current_ma"),
            ("power_and_thermal", "current_sample_rate_hz"),
        )
        for section, field in fields:
            with self.subTest(section=section, field=field):
                data = self._valid_manifest()
                target = data["captures"][0] if section == "capture" else data[section]
                target[field] = 0
                with self.assertRaisesRegex(ValidationError, field):
                    validate_manifest(self._write(data))

    def test_bounded_measurements_reject_values_above_their_maximum(self) -> None:
        cases = (
            ("relative_humidity_percent", 100.1),
            ("saturated_pixel_fraction", 1.01),
        )
        for field, value in cases:
            with self.subTest(field=field):
                data = self._valid_manifest()
                target = data["fixture"] if field == "relative_humidity_percent" else data["captures"][0]
                target[field] = value
                with self.assertRaisesRegex(ValidationError, field):
                    validate_manifest(self._write(data))

    def test_usable_crop_cannot_exceed_source_dimensions(self) -> None:
        cases = (
            ("usable_width_px", "width_px"),
            ("usable_height_px", "height_px"),
        )
        for usable_field, source_field in cases:
            with self.subTest(field=usable_field):
                data = self._valid_manifest()
                capture = data["captures"][0]
                capture[usable_field] = capture[source_field] + 1
                with self.assertRaisesRegex(ValidationError, usable_field):
                    validate_manifest(self._write(data))

    def test_provenance_identifiers_require_canonical_formats(self) -> None:
        cases = (
            ("captured_at_utc", "yesterday"),
            ("captured_at_utc", "2026-8-4T1:2:3Z"),
            ("captured_at_utc", "2026-08-24T12:00:00z"),
            ("firmware_commit", "main"),
        )
        for field, value in cases:
            with self.subTest(field=field):
                data = self._valid_manifest()
                target = data if field == "captured_at_utc" else data["device"]
                target[field] = value
                with self.assertRaisesRegex(ValidationError, field):
                    validate_manifest(self._write(data))

    def test_combined_peak_cannot_be_lower_than_constituent_peaks(self) -> None:
        data = self._valid_manifest()
        data["power_and_thermal"]["combined_peak_current_ma"] = 100
        with self.assertRaisesRegex(ValidationError, "combined_peak_current_ma"):
            validate_manifest(self._write(data))

    def test_count_and_pixel_fields_require_integers(self) -> None:
        for field in ("bytes", "width_px", "height_px", "usable_width_px", "usable_height_px"):
            with self.subTest(field=field):
                data = self._valid_manifest()
                data["captures"][0][field] += 0.5
                with self.assertRaisesRegex(ValidationError, field):
                    validate_manifest(self._write(data))


if __name__ == "__main__":
    unittest.main()
