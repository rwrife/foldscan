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

Implemented so far (issue #5, domain work in `app/domain`):

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

5. In-memory derivative export: processed bytes use the same checksum,
   read-back, and finalization gates as file-backed content; originals remain
   file-backed. Executor preflight also binds the public file layout and
   manifest to the canonical content plan, rejecting stale or edited
   combinations before creating directories. See the
   [export-integrity evidence](domain/evidence/export-integrity.md).

6. Derivative metadata preflight: original-only pages cannot carry orphan
   processed fields. Derivatives require checksum, byte count (at most 64 MiB),
   and a digest referencing a supplied recipe; omitted processed media type
   continues to mean PNG. Portable manifests require processed path, checksum,
   byte count, and recipe digest together or all absent, with lowercase SHA-256
   shapes. Invalid records fail before export parent/root creation, even if
   the public plan and manifest are edited together. See
   [derivative-metadata evidence](domain/evidence/derivative-metadata.md).

7. Cooperative export cancellation: `execute_export_cancellable` accepts a
   host cancellation callback, checked after validation before directory
   creation and between planned files, including before the final manifest.
   Cancellation uses the existing rollback path and permits retry when the
   new export root has been removed. The original API remains available.
   Cancellation cannot interrupt a single file's I/O/verification, and requests
   arriving after manifest writing starts are not observed. Rollback remains
   best-effort on storage failure; parent directories may remain. See
   [cancellation evidence](domain/evidence/export-cancellation.md).

8. PNG codec boundary: `png::encode_png` writes deterministic 8-bit
   grayscale PNG (filter 0, deflate stored blocks) and `png::decode_png`
   reads the baseline 8-bit grayscale subset — all five scanline filters,
   stored/fixed/dynamic deflate blocks, and multi-IDAT streams — from
   untrusted bytes with allocation bounds fixed by IHDR before any
   decompression, and stable `DomainError` categories for malformed input.
   The decoder is cross-checked against fixtures generated by independent
   encoders (libpng via Pillow, Python zlib, pypng) and validated with an
   independent decoder; the encoder's byte-exact output is pinned and
   Pillow-validated. JPEG remains out of scope; interlaced/palette/16-bit/
   alpha PNGs are rejected. See [PNG evidence](domain/evidence/png-codec.md).

9. PDF export writer: `pdf::export_pdf` serializes an ordered page set as
   a deterministic PDF 1.4 document — one `/Image` XObject per page
   (`/DeviceGray`, 8-bit, zlib stored-block stream under `/FlateDecode`,
   1 px = 1 pt) with correct xref/trailer. Bounds (page count, per-frame
   pixels/bytes, total document size) are enforced before allocation; a
   post-assembly self-check re-parses the output and verifies structure and
   payload fidelity (every embedded stream re-inflates to the exact source
   frame) before returning. Byte-exact output is pinned by golden digests;
   committed fixtures were cross-parsed with an independent PDF parser
   (PyMuPDF) at generation time. PDF *parsing*, text/OCR layers, and
   executor/manifest integration of PDF derivatives are out of scope.
   See [PDF evidence](domain/evidence/pdf-export.md).

10. OCR document core: `ocr::OcrResult` models a versioned
    `foldscan.ocr/0.1` result document per capture — capture binding,
    explicit requested language-pack list, and a closed terminal status
    (completed blocks with per-mille integer confidence and pixel boxes
    inside a declared frame, `failed` with a closed code vocabulary, or
    `skipped` with a closed reason vocabulary; no document means "not
    run"). Untrusted documents are bounded before parse and re-validated
    after (language shape/uniqueness, confidence range, block and text
    bounds, overflow-checked box-in-frame, control-character-free text),
    carry a canonical-JSON integrity digest, and `ocr::OcrProvider` is the
    narrow seam a future offline engine implements. Tests pin that export
    layout and manifest digests are identical whether OCR completed,
    failed, or never ran — an OCR failure cannot change or block export
    planning. **No OCR engine exists**; nothing here recognizes text. See
    [OCR evidence](domain/evidence/ocr-document.md).

The crate is verified by 165 unit/fixture tests plus a clean
`clippy -D warnings` pass locally; CI runs the same gates
(`.github/workflows/app-domain.yml`). All
evidence is software fixture evidence; no physical
device integration, and no optical-bench measurement has occurred. JPEG
codec integration, corner detection, dewarp, an actual OCR engine, and the UI
remain open work under this issue (PNG landed in slice 8; the PDF writer
landed in slice 9; the OCR document core landed in slice 10).

## Verification

```bash
cd app/domain
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
