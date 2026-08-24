# FoldScan firmware plan

## Responsibilities

- Initialize camera, storage, USB, button, status indicator, and illumination in safe states.
- Run an explicit capture state machine: idle → prepare light/exposure → capture → validate/write → finalize manifest → report result.
- Queue sessions and captures offline on microSD with checksums and atomic finalization.
- Expose diagnostics, capabilities, version, storage health, and recoverable errors.
- Support USB-first import/maintenance and an optional explicitly provisioned local Wi-Fi transport.
- Enforce bounds on paths, file sizes, session counts, request rates, illumination level/duration, and firmware update inputs.

The firmware does not perform OCR or claim final scan quality. Heavy correction, OCR, indexing, and PDF export belong to the companion.

## Planned architecture

- Pinned ESP-IDF toolchain and reproducible build configuration.
- Hardware abstraction interfaces for camera, storage, clock, button, light, status, and transports.
- Event-driven capture controller with no blocking UI assumptions.
- Versioned persistent configuration with safe defaults and migration tests.
- Append/finalize session manifest format shared with the app.
- Structured logs that omit page content and credentials.

## Interfaces and protocol

The transport-neutral contract begins in [`docs/protocol.md`](../docs/protocol.md). USB/removable import is baseline. Wi-Fi remains disabled until explicit pairing and must not be required for firmware build, capture, recovery, or file retrieval.

## Provisioning and updates

- Factory/default state: Wi-Fi disabled, illumination off, no cloud endpoint.
- USB serial or companion-guided local provisioning with user confirmation.
- Firmware update via documented USB recovery first; network update is optional and must verify signed/versioned artifacts if added.
- Always document bootloader/recovery entry, erase/reset consequences, backup steps, and rollback limits.

## Test strategy

- Host tests for capture state transitions, debounce, timeout, retries, manifest atomicity, checksum handling, malformed paths/requests, and power-loss recovery logic.
- Fake camera/storage/light/transport backends in CI.
- ESP-IDF compile and static checks with pinned versions.
- Hardware-in-loop only after a board/module exists; record board revision, instruments, conditions, raw logs, and actual measurements.
- Distinguish static analysis, simulation, bench measurements, and field testing in every report.

## Current status

No firmware project, build, flash result, test suite, or device evidence exists yet. This file defines the boundary for backlog implementation.
