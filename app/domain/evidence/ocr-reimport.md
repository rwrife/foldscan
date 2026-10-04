# OCR sidecar re-import (issue #53)

Evidence category: **local software fixture**. The new `foldscan_domain::load_exported_ocr(root, expected_manifest_digest)` is a read-only domain API for an existing local export, not a companion UI command and not an OCR engine. The digest must come from a previously reviewed preview/execution receipt *outside* the export directory; a digest fetched from the same directory does not establish trust. The caller gets completed OCR documents in manifest page order (pages without OCR omitted). It does not read or verify originals, derivatives, or PDFs.

The loader caps `export.json`, each JSON sidecar, and optional `.txt` rendition before parsing/returning; derives paths from checked session/capture IDs; rejects symlinks at the export root and each traversed component; checks semantic JSON digests and exact text-rendering bytes. Failures have stable categories with no OCR text or full host paths in diagnostics. The filesystem snapshot must remain stable while reading: symlink checks followed by open are not race-proof under concurrent mutation. SHA-256 digests are integrity pins, not authentication signatures. There is no GUI, recognized text, physical capture, bench measurement, or edit persistence in this slice. OCR source documents in tests are synthetic and originals are synthetic bytes.

Local verification (Linux, `app/domain`):

- `cargo fmt && cargo fmt --check` — pass.
- `cargo clippy --all-targets --locked -- -D warnings` — pass.
- `cargo test --locked --test ocr_reimport` — 10 passed, 0 failed (including real domain executor export fixture, ordered multiple sidecars, digest/tamper/size/missing-file/symlink gates).
- `cargo test --locked` — 238 passed, 0 failed across unit/integration suites; doc tests 0.

Not applicable: KiCad ERC/DRC, BOM/datasheet checks, firmware, companion build, mechanical checks: no hardware, firmware, companion, or mechanical source changed in this domain-only slice. CI result is reported separately in the PR; no physical claims are made.
