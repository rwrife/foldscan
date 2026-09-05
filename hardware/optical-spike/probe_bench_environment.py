#!/usr/bin/env python3
"""Collect deterministic host-side bench-readiness evidence for FoldScan issue #1.

This probe is intentionally conservative: it reports what the host can see
(camera/serial nodes, USB inventory, key tooling) and explains why bench
captures are blocked when required surfaces are missing.
"""

from __future__ import annotations

import argparse
import glob
import json
import platform
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

DEVICE_GLOBS: dict[str, tuple[str, ...]] = {
    "video": ("/dev/video*", "/dev/media*"),
    "serial": ("/dev/ttyACM*", "/dev/ttyUSB*"),
    "instrument": ("/dev/usbtmc*", "/dev/hidraw*"),
}

TOOL_COMMANDS: dict[str, tuple[str, ...]] = {
    "lsusb": ("lsusb",),
    "v4l2-ctl": ("v4l2-ctl", "--list-devices"),
    "libcamera-hello": ("libcamera-hello", "--version"),
    "rpicam-hello": ("rpicam-hello", "--version"),
    "esptool.py": ("esptool.py", "version"),
    "esptool": ("esptool", "version"),
    "idf.py": ("idf.py", "--version"),
}

USB_MODULE_KEYWORDS = ("espressif", "seeed", "xiao")
USB_LINE_PATTERN = re.compile(
    r"^Bus\s+(?P<bus>\d+)\s+Device\s+(?P<device>\d+):\s+ID\s+"
    r"(?P<vendor>[0-9a-fA-F]{4}):(?P<product>[0-9a-fA-F]{4})\s*(?P<description>.*)$"
)


@dataclass
class CommandResult:
    available: bool
    command: tuple[str, ...]
    exit_code: int | None
    stdout_lines: list[str]
    stderr_lines: list[str]


def _run_command(command: tuple[str, ...]) -> CommandResult:
    executable = command[0]
    if shutil.which(executable) is None:
        return CommandResult(
            available=False,
            command=command,
            exit_code=None,
            stdout_lines=[],
            stderr_lines=[f"{executable} not found on PATH"],
        )

    completed = subprocess.run(command, capture_output=True, text=True, check=False)
    return CommandResult(
        available=True,
        command=command,
        exit_code=completed.returncode,
        stdout_lines=[line for line in completed.stdout.splitlines() if line.strip()],
        stderr_lines=[line for line in completed.stderr.splitlines() if line.strip()],
    )


def _glob_devices(glob_pattern: str) -> list[str]:
    return sorted(glob.glob(glob_pattern))


def _parse_usb_devices(lines: list[str]) -> list[dict[str, str]]:
    parsed: list[dict[str, str]] = []
    for line in lines:
        match = USB_LINE_PATTERN.match(line.strip())
        if not match:
            continue
        parsed.append(
            {
                "bus": match.group("bus"),
                "device": match.group("device"),
                "vendor_id": match.group("vendor").lower(),
                "product_id": match.group("product").lower(),
                "description": match.group("description").strip(),
            }
        )
    return parsed


def _candidate_modules(usb_devices: list[dict[str, str]]) -> list[dict[str, str]]:
    candidates: list[dict[str, str]] = []
    for entry in usb_devices:
        text = f"{entry['vendor_id']}:{entry['product_id']} {entry['description']}".lower()
        if any(keyword in text for keyword in USB_MODULE_KEYWORDS):
            candidates.append(entry)
    return candidates


def _build_blockers(
    command_results: dict[str, CommandResult],
    device_nodes: dict[str, list[str]],
    candidate_modules: list[dict[str, str]],
) -> list[str]:
    blockers: list[str] = []
    if not command_results["lsusb"].available:
        blockers.append("lsusb is unavailable; cannot record USB inventory evidence")
    elif command_results["lsusb"].exit_code != 0:
        blockers.append("lsusb failed; USB inventory evidence is incomplete")

    if not device_nodes["video"]:
        blockers.append("no camera/video device nodes detected (/dev/video* or /dev/media*)")
    if not device_nodes["serial"]:
        blockers.append("no serial device nodes detected (/dev/ttyACM* or /dev/ttyUSB*)")
    if not candidate_modules:
        blockers.append("no likely Seeed/Espressif USB device detected in lsusb output")

    return blockers


def collect_bench_environment(
    *,
    run_command: Callable[[tuple[str, ...]], CommandResult] = _run_command,
    glob_devices: Callable[[str], list[str]] = _glob_devices,
) -> dict[str, Any]:
    command_results = {name: run_command(command) for name, command in TOOL_COMMANDS.items()}

    device_nodes = {
        role: sorted({path for pattern in patterns for path in glob_devices(pattern)})
        for role, patterns in DEVICE_GLOBS.items()
    }

    lsusb_result = command_results["lsusb"]
    usb_devices = _parse_usb_devices(lsusb_result.stdout_lines) if lsusb_result.available else []
    candidates = _candidate_modules(usb_devices)
    blockers = _build_blockers(command_results, device_nodes, candidates)

    host_tools = {
        name: {
            "available": result.available,
            "exit_code": result.exit_code,
            "command": list(result.command),
            "stdout": result.stdout_lines,
            "stderr": result.stderr_lines,
        }
        for name, result in command_results.items()
    }

    return {
        "schema_version": "1.0",
        "captured_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "host": {
            "platform": platform.platform(),
            "python": sys.version.split()[0],
        },
        "tools": host_tools,
        "device_nodes": device_nodes,
        "usb_devices": usb_devices,
        "candidate_module_devices": candidates,
        "bench_ready": not blockers,
        "blockers": blockers,
    }


def to_markdown(report: dict[str, Any]) -> str:
    lines = [
        f"# FoldScan bench readiness probe ({report['captured_at_utc']})",
        "",
        f"- Bench ready: **{'yes' if report['bench_ready'] else 'no'}**",
        f"- Host platform: `{report['host']['platform']}`",
        f"- Python: `{report['host']['python']}`",
        "",
        "## Device nodes",
    ]

    for role in ("video", "serial", "instrument"):
        nodes = report["device_nodes"].get(role, [])
        if nodes:
            lines.append(f"- {role}: {', '.join(f'`{node}`' for node in nodes)}")
        else:
            lines.append(f"- {role}: *(none)*")

    lines.extend(["", "## Tooling", ""])
    for name in ("lsusb", "v4l2-ctl", "libcamera-hello", "rpicam-hello", "esptool.py", "esptool", "idf.py"):
        tool = report["tools"].get(name, {})
        if tool.get("available"):
            exit_code = tool.get("exit_code")
            lines.append(f"- {name}: available (exit={exit_code})")
        else:
            lines.append(f"- {name}: unavailable")

    lines.extend(["", "## USB candidates", ""])
    candidates = report.get("candidate_module_devices", [])
    if not candidates:
        lines.append("- *(none matching Seeed/Espressif keywords)*")
    else:
        for entry in candidates:
            lines.append(
                "- "
                f"Bus {entry['bus']} Device {entry['device']} "
                f"{entry['vendor_id']}:{entry['product_id']} {entry['description']}"
            )

    lines.extend(["", "## Blockers", ""])
    blockers = report.get("blockers", [])
    if not blockers:
        lines.append("- none")
    else:
        for blocker in blockers:
            lines.append(f"- {blocker}")

    return "\n".join(lines) + "\n"


def _render(report: dict[str, Any], markdown: bool) -> str:
    return to_markdown(report) if markdown else json.dumps(report, indent=2, sort_keys=True) + "\n"


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--markdown",
        action="store_true",
        help="render Markdown instead of JSON",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="write output to this file instead of stdout",
    )
    parser.add_argument(
        "--require-ready",
        action="store_true",
        help="exit non-zero when bench requirements are not met",
    )
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    report = collect_bench_environment()
    rendered = _render(report, markdown=args.markdown)

    if args.output is None:
        sys.stdout.write(rendered)
    else:
        args.output.write_text(rendered, encoding="utf-8")
        print(args.output)

    if args.require_ready and not report["bench_ready"]:
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
