# OCR plain-text rendition export (issue #35, child of #5)

Date: 2026-09-22. Evidence category: **software fixture evidence** on
synthetic OCR documents — no OCR engine exists, so no recognition accuracy,
physical device, or bench claim is made or implied.

## What was added

- `ocr::OcrResult::render_plain_text()` — deterministic plain-text
  rendering of a *completed* `foldscan.ocr/0.1` document: block texts in
  document order, one `\n` after each block; a completed document with zero
  blocks renders to exactly `"\n"` (empty-but-present page text).
  Non-completed (failed/skipped) documents return `InvalidRequest` —
  consistent with the export rule that only completed results bind.
- `ocr::OcrResult::plain_text_digest()` — SHA-256 over the exact rendered
  bytes, bound into the manifest next to the JSON document digest.
- `MAX_OCR_TEXT_BYTES` — derived byte bound on a rendering
  (`MAX_OCR_TEXT_CHARS * 4 + MAX_OCR_BLOCKS`), from the validated
  character/block bounds.
- `ExportSession::ocr_text: bool` — opt-in flag. When on, the planner lays
  out `ocr/<session>/<capture>.txt` immediately after each bound capture's
  `.json` sidecar (page order preserved; host binding-list order still
  cannot change the plan).
- `ContentKind::OcrText` and `ExportManifestPage::ocr_text_digest`
  (additive `Option`, `#[serde(default, skip_serializing_if)]` — pre-text
  manifests serialize and parse byte-identically).
- Manifest validation: `ocr_text_digest` must be lowercase-hex SHA-256 and
  may only exist alongside `ocr_digest` (no text without its reviewed
  document). `ExportManifest::from_plan` cross-checks the planned
  `OcrText` layout against the session's bindings and opt-in per capture.
- Executor: `ContentKind::OcrText` files are rendered *from the bound
  document* (never from a host-supplied source — supplying a source for a
  planned `.txt` is an explicit `InvalidRequest`), cross-checked against
  the manifest's `ocr_text_digest` before writing, and finalized through
  the same staged-write/verify/atomic-rename/rollback path.

## Commands and results (all on the PR head, Linux aarch64, cargo 1.98.0)

| Gate | Result |
|------|--------|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --locked -- -D warnings` | clean |
| `cargo test --locked` | 220 passed / 0 failed (baseline before edits: 197; +23 new in `tests/ocr_text_export.rs`, no pre-existing test modified) |
| `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps` | clean |

## Golden digests (independent computation)

Rendering digests were computed with Python `hashlib` before being pinned
in `tests/ocr_text_export.rs`, so the Rust code cannot have generated its
own expectation:

- `"alpha\n"` → `b6a98d9c…b51060`
- `"alpha\nbeta\n"` → `e49c81e2…0d78ee`
- `"beta\nalpha\n"` → `3588d4ce…f07af9`
- `"\n"` (no blocks) → `01ba4719…ca546b`
- `"παν ἔixe 你好\n"` → `f6d292c3…684628`

## Mutation canary

Guards were individually neutered in a scratch copy and only
`tests/ocr_text_export.rs` re-run:

1. Manifest `validate()` text-without-document rule removed →
   `manifest_text_digest_without_document_digest_is_rejected` FAILED.
2. `from_plan` layout/opt-in cross-check removed →
   `edited_plan_layout_diverging_from_text_opt_in_is_rejected` FAILED.
3. Executor text-digest cross-check neutered → **no test changed outcome**:
   the executor's preflight already rebuilds the canonical layout and
   rejects any plan/manifest divergence before file writing, so the
   cross-check is defense-in-depth for future refactors rather than the
   sole active barrier. Stated honestly rather than claimed as proven by
   the canary.

All guards restored; full suite re-verified green (220/0).

## Honesty boundaries

- No OCR engine exists; every test document is authored by the tests.
  Nothing here proves text recognition, only that a *given* document
  renders, binds, and round-trips deterministically.
- Layout follows document block order only; no line/column reconstruction
  (geometric re-flow stays a non-goal of this slice).
- The rendering cannot block image/PDF export any more than the JSON
  sidecar could: it exists only where a completed OCR document already
  binds, and absence is byte-identical to pre-slice exports (pinned by
  `opt_out_and_empty_bindings_keep_layout_byte_identical_to_pre_text`,
  `text_opt_in_json_sidecar_bytes_are_unchanged_by_the_rendition`, and
  `text_free_session_export_is_file_free_of_ocr_dir_entries_like_before`).
- No Tauri shell/UI change; `.txt` in the desktop UI is a later slice.
