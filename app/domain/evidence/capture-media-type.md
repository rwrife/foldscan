# Capture media type preservation (issue #51, part of #5)

Date: 2026-10-02. Evidence category: software fixture tests on synthetic
volumes. No device, optical, or GUI evidence.

## Problem fixed

`SessionManifest::validate` ignored the declared `media_type`, so any string
(`image/tiff`, `application/x-executable`, …) imported without complaint,
and `export_sessions_from_import` hard-coded `image/jpeg`. A PNG-declared
capture therefore planned `originals/<session>/<capture>.jpg` — export
mislabeling user file content — and unknown types only failed later at
`ext_for`, after the whole volume had been read and hashed (violating the
protocol's validate-before-IO principle).

## Contract now enforced

- Closed capture vocabulary `{image/jpeg, image/png}`
  (`manifest::SUPPORTED_CAPTURE_MEDIA_TYPES`), validated per capture in
  `SessionManifest::validate` *before* any capture file is opened; unknown
  values fail with `InvalidRequest` naming the capture and value, without
  host-path leakage.
- An omitted `media_type` resolves to the documented default
  `manifest::DEFAULT_CAPTURE_MEDIA_TYPE = "image/jpeg"`, preserving existing
  volumes/fixtures byte-for-byte.
- `ImportedCapture` carries the resolved `media_type`;
  `export_sessions_from_import` copies it verbatim into `ExportPage`, so the
  planner's original extension always matches the declared content type.

## Verification (executor host, Linux arm64, headless)

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets --locked -- -D warnings`: clean.
- `cargo test --locked`: **228 passed, 0 failed** (baseline 221; +1 export
  unit test, +6 new `tests/capture_media_type.rs` integration tests).
- Mutation canary: reintroducing the hard-coded `image/jpeg` in
  `export_sessions_from_import` fails exactly 3 tests
  (`png_declared_capture_exports_with_png_extension`,
  `mixed_declared_types_layout_each_extension_correctly`,
  `declared_type_change_changes_the_export_manifest_digest`), proving the
  carry-through assertions are load-bearing.
- Companion shell lane (Ubuntu 24.04 Docker image with WebKitGTK
  prerequisites, host UID): `cargo fmt --check`, clippy `-D warnings`,
  `cargo test --locked` — **14 passed, 0 failed**. No shell code change was
  needed; the path dependency picks the corrected synthesis up automatically.

## Limits

- Validation is against the declared manifest string only; there is no
  magic-byte sniffing, so a device could still declare `image/png` for JPEG
  bytes (content sniffing is separate, deliberate work).
- Only the two capture types in the vocabulary exist; GIF/WebP/JPEG-XR and
  motion formats remain out of scope.
- Not device evidence: no physical ESP32-S3 volume was imported.
