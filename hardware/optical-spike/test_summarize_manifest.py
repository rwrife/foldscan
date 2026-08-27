#!/usr/bin/env python3

import unittest
from pathlib import Path

from summarize_manifest import summarize_manifest, summarize_manifest_file
from validate_manifest import ValidationError


class ManifestSummaryTests(unittest.TestCase):
    @staticmethod
    def _manifest() -> dict:
        captures = []
        for page_size, latencies in (
            ("A4", [1000, 2000, 3000, 4000]),
            ("LETTER", [900, 1100, 1300, 1500]),
        ):
            for index, latency in enumerate(latencies, start=1):
                captures.append(
                    {
                        "id": f"{page_size.lower()}-{index}",
                        "page_size": page_size,
                        "condition": "symmetric-matte",
                        "bytes": 1000 * index,
                        "latency_to_durable_file_ms": latency,
                        "center_px_per_mm": 6.4,
                        "top_left_px_per_mm": 6.1,
                        "top_right_px_per_mm": 6.2,
                        "bottom_left_px_per_mm": 6.0,
                        "bottom_right_px_per_mm": 6.3,
                        "max_distortion_px": index,
                        "saturated_pixel_fraction": index / 100,
                    }
                )
        return {
            "schema_version": "1.0",
            "template": False,
            "evidence_category": "bench-test",
            "run_id": "20260827T120000Z-test",
            "captured_at_utc": "2026-08-27T12:00:00Z",
            "captures": captures,
            "mechanical": {
                "refold_cycles": [
                    {"cycle": cycle, "x_mm": cycle / 10, "y_mm": 0}
                    for cycle in range(1, 11)
                ],
                "vertical_deflection_10min_mm": 1.5,
            },
            "fixture": {"ambient_temperature_c": 35},
            "power_and_thermal": {
                "combined_peak_current_ma": 2500,
                "max_touchable_temperature_c": 42,
                "module_temperature_c": 55,
                "diffuser_temperature_c": 40,
                "test_duration_minutes": 30,
            },
        }

    def test_reports_page_latency_statistics_with_nearest_rank_p95(self) -> None:
        summary = summarize_manifest(self._manifest())

        a4 = summary["measurements"]["page_sizes"]["A4"]["latency_ms"]
        self.assertEqual(a4["samples"], 4)
        self.assertEqual(a4["minimum"], 1000)
        self.assertEqual(a4["median"], 2500)
        self.assertEqual(a4["p95"], 4000)
        self.assertEqual(a4["maximum"], 4000)

    def test_reports_optical_mechanical_power_and_thermal_measurements(self) -> None:
        summary = summarize_manifest(self._manifest())

        a4 = summary["measurements"]["page_sizes"]["A4"]
        self.assertEqual(a4["jpeg_bytes"]["median"], 2500)
        self.assertEqual(a4["density_px_per_mm"]["minimum"], 6.0)
        self.assertEqual(a4["density_px_per_mm"]["maximum"], 6.4)
        self.assertEqual(a4["maximum_distortion_px"], 4)
        self.assertEqual(a4["maximum_saturated_pixel_fraction"], 0.04)

        mechanical = summary["measurements"]["mechanical"]
        self.assertEqual(mechanical["max_refold_radial_displacement_mm"], 1.0)
        self.assertEqual(mechanical["vertical_deflection_10min_mm"], 1.5)

        power = summary["measurements"]["power_and_thermal"]
        self.assertEqual(power["combined_peak_current_ma"], 2500)
        self.assertEqual(power["max_touchable_temperature_c"], 42)

    def test_evaluates_only_requirements_supported_by_manifest_measurements(self) -> None:
        summary = summarize_manifest(self._manifest())
        checks = summary["acceptance_checks"]

        self.assertEqual(checks["OPT-03"]["status"], "pass")
        self.assertEqual(checks["MECH-03"]["status"], "pass")
        self.assertEqual(checks["MECH-04"]["status"], "pass")
        self.assertEqual(checks["ELEC-02-source-envelope"]["status"], "pass")
        self.assertEqual(checks["ENV-02-touchable-temperature"]["status"], "pass")
        self.assertEqual(checks["OPT-01"]["status"], "not-evaluable")
        self.assertEqual(checks["OPT-02"]["status"], "observed-no-threshold")
        self.assertIn("component temperature versus datasheet limits", summary["unresolved_evidence"])

    def test_manifest_file_must_pass_the_fail_closed_validator(self) -> None:
        template = Path(__file__).with_name("manifest.example.json")
        with self.assertRaisesRegex(ValidationError, "not bench evidence"):
            summarize_manifest_file(template)


if __name__ == "__main__":
    unittest.main()
