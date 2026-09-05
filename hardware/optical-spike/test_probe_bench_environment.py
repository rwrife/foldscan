#!/usr/bin/env python3

import unittest
from copy import deepcopy

import probe_bench_environment as probe


class BenchProbeTests(unittest.TestCase):
    @staticmethod
    def _fake_run(results: dict[str, probe.CommandResult]):
        def _runner(command: tuple[str, ...]) -> probe.CommandResult:
            key = command[0]
            return deepcopy(results[key])

        return _runner

    @staticmethod
    def _fake_glob(mapping: dict[str, list[str]]):
        def _glob(pattern: str) -> list[str]:
            return list(mapping.get(pattern, []))

        return _glob

    def test_bench_ready_when_camera_serial_and_candidate_module_exist(self) -> None:
        run = self._fake_run(
            {
                "lsusb": probe.CommandResult(
                    available=True,
                    command=("lsusb",),
                    exit_code=0,
                    stdout_lines=[
                        "Bus 001 Device 002: ID 303a:1001 Espressif USB JTAG/serial debug unit"
                    ],
                    stderr_lines=[],
                ),
                "v4l2-ctl": probe.CommandResult(
                    available=True,
                    command=("v4l2-ctl", "--list-devices"),
                    exit_code=0,
                    stdout_lines=["USB Camera"],
                    stderr_lines=[],
                ),
                "libcamera-hello": probe.CommandResult(
                    available=False,
                    command=("libcamera-hello", "--version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "rpicam-hello": probe.CommandResult(
                    available=False,
                    command=("rpicam-hello", "--version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "esptool.py": probe.CommandResult(
                    available=True,
                    command=("esptool.py", "version"),
                    exit_code=0,
                    stdout_lines=["esptool.py v4.8"],
                    stderr_lines=[],
                ),
                "esptool": probe.CommandResult(
                    available=False,
                    command=("esptool", "version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "idf.py": probe.CommandResult(
                    available=True,
                    command=("idf.py", "--version"),
                    exit_code=0,
                    stdout_lines=["ESP-IDF v5.4"],
                    stderr_lines=[],
                ),
            }
        )
        globber = self._fake_glob(
            {
                "/dev/video*": ["/dev/video0"],
                "/dev/media*": [],
                "/dev/ttyACM*": ["/dev/ttyACM0"],
                "/dev/ttyUSB*": [],
                "/dev/usbtmc*": [],
                "/dev/hidraw*": ["/dev/hidraw0"],
            }
        )

        report = probe.collect_bench_environment(run_command=run, glob_devices=globber)

        self.assertTrue(report["bench_ready"])
        self.assertEqual(report["blockers"], [])
        self.assertEqual(len(report["candidate_module_devices"]), 1)

    def test_missing_video_and_serial_report_blockers(self) -> None:
        run = self._fake_run(
            {
                "lsusb": probe.CommandResult(
                    available=True,
                    command=("lsusb",),
                    exit_code=0,
                    stdout_lines=["Bus 011 Device 002: ID 13d3:3630 IMC Networks Wireless_Device"],
                    stderr_lines=[],
                ),
                "v4l2-ctl": probe.CommandResult(
                    available=False,
                    command=("v4l2-ctl", "--list-devices"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "libcamera-hello": probe.CommandResult(
                    available=False,
                    command=("libcamera-hello", "--version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "rpicam-hello": probe.CommandResult(
                    available=False,
                    command=("rpicam-hello", "--version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "esptool.py": probe.CommandResult(
                    available=False,
                    command=("esptool.py", "version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "esptool": probe.CommandResult(
                    available=False,
                    command=("esptool", "version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
                "idf.py": probe.CommandResult(
                    available=False,
                    command=("idf.py", "--version"),
                    exit_code=None,
                    stdout_lines=[],
                    stderr_lines=["missing"],
                ),
            }
        )
        globber = self._fake_glob(
            {
                "/dev/video*": [],
                "/dev/media*": [],
                "/dev/ttyACM*": [],
                "/dev/ttyUSB*": [],
                "/dev/usbtmc*": [],
                "/dev/hidraw*": [],
            }
        )

        report = probe.collect_bench_environment(run_command=run, glob_devices=globber)

        self.assertFalse(report["bench_ready"])
        blockers = "\n".join(report["blockers"])
        self.assertIn("no camera/video device nodes", blockers)
        self.assertIn("no serial device nodes", blockers)
        self.assertIn("no likely Seeed/Espressif USB device", blockers)

    def test_markdown_render_mentions_blockers(self) -> None:
        report = {
            "captured_at_utc": "2026-09-05T00:00:00Z",
            "bench_ready": False,
            "host": {"platform": "Linux-test", "python": "3.11.0"},
            "device_nodes": {"video": [], "serial": [], "instrument": []},
            "tools": {
                name: {"available": False, "exit_code": None}
                for name in (
                    "lsusb",
                    "v4l2-ctl",
                    "libcamera-hello",
                    "rpicam-hello",
                    "esptool.py",
                    "esptool",
                    "idf.py",
                )
            },
            "candidate_module_devices": [],
            "blockers": ["example blocker"],
        }

        rendered = probe.to_markdown(report)
        self.assertIn("Bench ready: **no**", rendered)
        self.assertIn("lsusb: unavailable", rendered)
        self.assertIn("example blocker", rendered)


if __name__ == "__main__":
    unittest.main()
