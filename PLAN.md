# FoldScan implementation plan

## Scope

Deliver an incrementally buildable, low-voltage overhead document-capture system:

- a foldable mechanical stand and replaceable page guides;
- an ESP32-S3 camera/controller module on a custom carrier;
- controlled visible-light illumination and physical capture/status controls;
- offline microSD capture with USB-first transfer;
- a local desktop companion that produces reviewed images and searchable PDFs;
- editable source, validated BOM data, verification evidence, assembly guidance, and release artifacts.

The MVP proves a repeatable single-page and multi-page workflow for A4/US-Letter documents. Optical limits are measured rather than hidden; a module change is allowed before PCB freeze.

## Architecture

```text
Certified USB 5 V SELV
        |
        v
custom carrier -- button / status / PWM light control / test points
        |
        +-- XIAO ESP32S3 Sense candidate -- camera
        |              |
        |              +-- microSD capture queue + manifest
        |
        +-- USB baseline transfer
        +-- opt-in authenticated local Wi-Fi transfer (post-baseline)

Desktop companion
  import -> integrity check -> page review -> crop/perspective/light correction
         -> optional offline OCR/index -> reorder -> PDF/images/session export
```

### Boundaries

- Firmware owns capture timing, storage integrity, device state, illumination control, and transport.
- The companion owns CPU-heavy image correction, OCR, indexing, review, and export.
- The protocol is versioned and transport-independent.
- KiCad symbol properties own final Manufacturer/MPN/supplier metadata; exported CSVs are derived.
- Mechanical CAD owns arm geometry, hinge stops, page plane, camera height, and diffuser mounts.

## Technology choices

- **ESP32-S3 / ESP-IDF:** inexpensive, documented camera ecosystem, USB and Wi-Fi options, and a repeatable native build. Arduino may be used only for a disposable optical spike, not the production baseline.
- **XIAO ESP32S3 Sense candidate:** compact module combining controller, camera expansion, and removable storage; exact revision, pin availability, image quality, and power must be validated before schematic freeze.
- **KiCad 9+:** editable, inspectable open PCB sources and command-line ERC/DRC/export support.
- **Tauri 2 + Rust + TypeScript:** cross-platform desktop shell with a testable native processing core and a narrow web UI boundary.
- **OpenCV-compatible processing:** deterministic crop, homography, dewarp, and illumination correction; backend/license must be pinned.
- **Tesseract-class offline OCR:** mature no-cloud baseline with language packs under user control; OCR is advisory and the original image remains available.
- **Versioned JSON/CBOR-style contract:** JSON manifests for portability and a framed binary/file transfer path for captures; protocol details remain transport-neutral.

Alternatives to evaluate rather than assume: OV2640-class versus higher-resolution camera modules, fixed USB storage versus explicit file transfer, printed hinge versus off-the-shelf friction hinge, and desktop-only versus later mobile companion support.

## Milestones and dependency order

### M0 — requirements and risk closure

**2026-08-24 status:** documentation/calculation gate complete; physical gate blocked. Architecture may proceed only as the disposable spike in ADR 0001. Camera, mechanical geometry, illumination, and custom-carrier freeze are **STOP/HOLD** until a real run passes `hardware/optical-spike/validate_manifest.py` without the template override.

- Fix page sizes, camera height/field of view, sharpness/readability metrics, capture timing, storage behavior, stand stability, light/thermal limits, and target budget.
- Build a disposable module-and-stand optical spike before committing to a PCB.
- Record accepted limitations for glossy pages, curled pages, handwriting, and fine print.

### M1 — selected parts, schematic, and BOM

- Verify manufacturer datasheets, pinouts, voltage/current limits, packages, lifecycle, and sourcing.
- Create the real KiCad project/schematic with protection, connectors, programming/debug, named nets, and test points.
- Populate Manufacturer and MPN properties in symbols; export and validate `bom/bom.csv`.
- Run ERC and document only justified exceptions.

### M2 — carrier PCB and firmware baseline

- Lay out and route the carrier with mounting geometry, LED-current/thermal constraints, USB/antenna keepouts, test access, and readable silkscreen.
- Run DRC and project analyzers; export assembly drawings for review.
- Add reproducible ESP-IDF build, host-side state-machine/storage tests, flashing, recovery, and a simulated camera/storage backend.

### M3 — companion vertical slice

- Implement USB/removable import, manifest validation, page review, deterministic correction, reorder, and image/PDF export.
- Add optional offline OCR after the image-only workflow is reliable.
- Meet keyboard, screen-reader, scaling, contrast, and reduced-motion expectations.

### M4 — integrated bring-up and mechanics

- Publish editable mechanical sources plus printable/exported artifacts.
- Assemble one prototype and record rails, current, illumination temperature, capture latency, storage recovery, and stand stability.
- Build a non-sensitive optical corpus with printed targets and representative pages; report metrics and known failures.
- Complete assembly, calibration, troubleshooting, and repair documentation.

### M5 — fabrication/release

- Re-run ERC, DRC, BOM validation, firmware tests/build, app tests/build, protocol checks, and fabrication output inspection.
- Export Gerbers, drill, pick-and-place/CPL when applicable, schematic PDF, BOM, board renders, firmware/app artifacts, checksums, and licenses.
- Tag a release only after reproducible outputs and limitations are documented.

## Testing strategy

### Static analysis

- KiCad ERC/DRC, schematic/PCB analyzers, BOM property coverage, and fabrication file inspection.
- Rust formatting/linting/tests, TypeScript checks, dependency/license audit, and ESP-IDF warnings-as-errors where practical.
- Protocol schema fixtures and malformed-input tests.

### Simulation and hardware abstraction

- Firmware tests with fake camera, storage, button, light, and transport interfaces.
- App processing tests against synthetic perspective, rotation, shadow, noise, and corrupt-manifest fixtures.
- Power/thermal calculations or SPICE simulation where the selected illumination/power circuitry warrants it.

### Bench testing

- Only after hardware exists: rail voltages, boot/inrush/steady current, illumination PWM/current, surface temperature, button behavior, USB transfer, microSD power-loss recovery, ESD-aware handling, and repeated capture sessions.
- Record instruments, firmware/app revisions, fixtures, conditions, raw logs, and expected ranges.

### Optical and field testing

- Printed resolution/distortion target, text-size readability, corner focus, glare/shadow, color neutrality, crop error, OCR character/word error on consented non-sensitive samples, and multi-page throughput.
- Field testing is a separate later category; it cannot be inferred from static checks, simulation, or one bench session.

No issue may call unbuilt hardware “tested.”

## Packaging and distribution

- Firmware: reproducible ESP-IDF build plus versioned binaries/checksums and flashing/recovery instructions.
- Desktop: unsigned development bundles first; later Windows installer/portable package, macOS notarization guidance, and Linux AppImage/deb where CI supports them.
- Hardware: KiCad source, fabrication ZIP, schematic PDF, BOM/CPL, board renders, and release checklist.
- Mechanics: editable CAD source plus reviewed STL/STEP exports and printer/material assumptions.
- Data: documented portable session manifest, original captures, processed images, OCR text, and PDF export.

## Risks and mitigations

| Risk | Status (2026-08-24) | Mitigation / evidence gate |
|---|---|---|
| Candidate camera cannot resolve fine print across A4 | **UNKNOWN — freeze stopped** | Seeed SKU 113991115 may ship with different sensors and no reviewed source specifies the supplied lens/FOV. Run the checksum-validated A4/Letter matrix before PCB freeze; allow a documented higher-resolution module path. |
| Folded stand tips, sags, or loses alignment | **UNKNOWN — no stand** | Measurable center-of-mass/deflection requirements, hinge stops, optional desk clamp, replaceable calibration target, and ten-cycle measurements. |
| LED glare, heat, or rolling-band artifacts | **UNKNOWN — no lights** | Diffusion, angled dual lighting, one-sided/glossy controls, bounded PWM/frequency, thermal/current measurements, exposure-lock experiments. |
| Power loss corrupts storage | **UNKNOWN — no firmware/device test** | Atomic manifest updates, temp-file rename, checksums, recovery tests, explicit safe-eject state. |
| Wi-Fi expands attack surface | **CONTROL ACCEPTED; implementation deferred** | USB-first baseline, disabled by default, local-only pairing, no inbound internet dependency, protocol limits. |
| OCR errors are trusted | **CONTROL ACCEPTED; app unbuilt** | Keep originals, show confidence/uncertain text, require review, label OCR as convenience rather than authoritative transcription. |
| BOM price/availability drifts | **OPEN** | Manufacturer/MPN source of truth in KiCad, dated validation, alternatives only after pin/package review. The candidate module has one dated manufacturer price; no total quote exists. |
| Scope expands into archival/book automation | **CONTROL ACCEPTED** | Enforce single-page/manual-turn MVP and explicit non-goals. |

## Explicit non-goals

Cloud storage, account systems, automatic page turning, destructive book scanning, calibrated archival imaging, identity verification, legal evidence capture, A2 scanning, battery charging, mains power, mobile apps in the MVP, and on-device OCR.
