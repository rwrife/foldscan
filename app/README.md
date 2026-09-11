# FoldScan local companion plan

## Responsibilities

The companion is a local-first desktop application targeting Windows 10/11, macOS, and Linux. It will:

1. discover/import captures through USB/removable media first and optional paired LAN transfer later;
2. validate protocol/session versions, paths, sizes, checksums, and duplicate capture IDs;
3. preserve originals and build reversible processed derivatives;
4. show an accessible thumbnail/page-review workflow;
5. rotate, crop, detect corners, correct perspective/illumination, and optionally dewarp;
6. run optional offline OCR with user-selected language packs and visible uncertainty;
7. reorder pages and export images, OCR text, PDF, and a portable session manifest;
8. provide explicit local retention, deletion, export, and diagnostic controls.

## Proposed stack

- Tauri 2 desktop shell.
- Rust domain/processing core with narrow interfaces for image processing, OCR, storage, PDF export, and device transport.
- TypeScript UI with automated accessibility checks.
- OpenCV-compatible deterministic processing evaluated for licensing and cross-platform packaging.
- Tesseract-class offline OCR as a candidate, not a completed dependency choice.

## Setup flow

- First run explains local storage, original preservation, and optional OCR language downloads/imports.
- User selects a library/export directory; the app does not crawl the whole disk.
- USB/removable import requires no account or network.
- Optional Wi-Fi pairing displays device identity and a short-lived code; it remains disableable and local-network only.
- Processing profiles are reviewable and exportable JSON, with conservative defaults.

## Data ownership

- Original captures and manifests remain in ordinary user-selected folders.
- Processed images, OCR text, indexes, and settings remain local.
- No telemetry, ad SDK, analytics, cloud OCR, or account is part of the MVP.
- Export supports original and processed images, plain OCR text, PDF, and versioned JSON manifest.
- Deletion never silently crosses the device/host boundary; cleanup consequences are shown before confirmation.

## Protocol boundary

The app treats every device file/message as untrusted input. It validates versions, canonicalizes paths, caps allocations, verifies checksums, never executes device-supplied content, and does not render unescaped metadata as HTML. See [`docs/protocol.md`](../docs/protocol.md).

## Accessibility

Acceptance includes keyboard-only workflows, logical focus order, visible focus, screen-reader names/status, scalable text, 200% zoom resilience, high contrast, non-color-only state, reduced motion, and accessible error recovery. OCR output must remain editable and uncertainty must not be communicated only by color.

## Test strategy

- Rust unit/property tests for manifests, path safety, deduplication, export, and processing transforms.
- Golden-image fixtures for rotation, perspective, crop, shadow, noise, and deterministic output; originals contain no personal documents.
- OCR fixtures with published expected text and tolerance, while preserving raw output for inspection.
- UI component/end-to-end tests for import → review → process → reorder → export, keyboard navigation, and failure recovery.
- Build/package smoke tests on supported CI hosts.
- Real-device integration only after hardware/firmware exists and is reported separately from mocked tests.

## Current status

No Tauri shell, UI, image codec integration, OCR model/language data, or
supported installer exists yet.

Implemented so far (issue #5, four domain slices in `app/domain`):

1. Import core: protocol version negotiation, bounded input validation
   before allocation, canonical relative-path safety, SHA-256 capture
   verification, duplicate-ID rejection, read-only `FOLDSCAN/` import.
2. Export-side domain core: versioned `foldscan.recipe/0.1` processing-recipe
   documents with a closed operation vocabulary (rotate/crop/perspective/
   illumination/dewarp) and per-recipe content digests; the originals/
   derivatives page model with reorder and remove-from-export-without-delete;
   a collision-checked export layout planner (writes nothing); and a
   versioned portable export manifest (`foldscan.export/0.1`) with
   order-sensitive integrity digests.
3. Filesystem export executor: materializes a validated plan under
   staged-write/flush/checksum-verify/atomic-rename semantics, writes the
   manifest last, never overwrites an existing export, and rolls back the
   tree it created on any failure.
4. Deterministic processing core: a `GrayFrame` model with bounds-checked
   allocation, a `Processor` interface, and a reference implementation whose
   rotate/crop/quadrilateral-rectification/illumination ops use only integer
   fixed-point and correctly-rounded IEEE-754 primitives — no transcendental
   calls — so processed output is bit-stable across IEEE-754 hosts. Output
   bytes are locked by golden-digest fixtures
   (`app/domain/tests/golden_processing.rs`), and malformed recipes or
   degenerate quadrilaterals are rejected with stable error categories
   rather than garbled output.

The crate is verified by 95 unit/fixture tests plus a clean
`clippy -D warnings` pass in CI (`.github/workflows/app-domain.yml`). All
evidence is software fixture evidence; no image encode/decode, no physical
device integration, and no optical-bench measurement has occurred. JPEG/PNG
codec integration, corner detection, dewarp, PDF export, OCR, and the UI
remain open work under this issue.

## Verification

```bash
cd app/domain
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
