# OCR document core — evidence note (issue #29, child of #5)

## What was added

- `app/domain/src/ocr.rs`: the `foldscan.ocr/0.1` result-document model
  (capture binding, requested language-pack list, closed
  completed/failed/skipped terminal status with closed failure-code and
  skip-reason vocabularies), bounds-before-parse validation, control-free
  recognized text, per-mille integer confidence, overflow-checked block
  bounds against the declared frame, canonical-JSON integrity digest, and
  the narrow `OcrProvider` seam.
- `app/domain/tests/ocr_document.rs`: untrusted-document fixture matrix
  (schema handling, language shape, confidence bounds, frame-escape and
  zero-area boxes, control characters, byte bound before allocation, text
  size bound after parse, digest stability/sensitivity), a stub-provider
  seam exercise, and export-equivalence tests.
- Public re-exports in `app/domain/src/lib.rs`.

## What was verified (software-fixture evidence only)

Commands and results run on `feat/ocr-domain-core` (host: Linux aarch64,
rustc 1.98.0):

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --locked -- -D warnings` — clean.
- `cargo test --locked` — 165 passed; 0 failed across all suites
  (11 tests in `tests/ocr_document.rs`, 6 unit tests inside `src/ocr.rs`,
  remaining 148 pre-existing tests unchanged).

## Honesty boundaries

- **No OCR engine exists.** Nothing here recognizes text; the crate only
  validates the *shape* of OCR result documents and defines the interface
  an offline engine will later implement. No recognition-accuracy claim is
  made or possible from these tests.
- Export-equivalence tests prove the *structural* property that OCR
  outcomes are invisible to the export planner and manifest digest (OCR is
  sidecar metadata). They do not exercise a real engine's failure modes —
  they exercise the domain contract that a future engine must satisfy.
- The language-pack id validator is a hostile-input shape guard
  (ASCII-alnum plus `-`, uniqueness, bounds); real pack availability and
  BCP 47 conformance are the engine adapter's responsibility.
- No network use, no new dependencies, no UI. Confidence is integer
  per-mille so UI status can be announced non-visually.
- No physical device or bench measurement is involved in any claim here.
