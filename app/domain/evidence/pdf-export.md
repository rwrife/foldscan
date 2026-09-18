# PDF export writer — evidence (issue #27, part of #5)

**Evidence category: software code + self-generated fixture bytes validated
by an independent PDF parser at generation time. No device evidence, no
optical-bench evidence, no print-output claim.**

## What landed

`app/domain/src/pdf.rs` (std-only, zero new dependencies):

- `export_pdf(&[GrayFrame]) -> Result<Vec<u8>, DomainError>` writes a
  PDF 1.4 document: catalog → page tree → one page per input frame in
  order, each drawing one `/Image` XObject (`/DeviceGray`, 8-bit) whose
  stream is the frame payload wrapped by the PNG module's already-tested
  `zlib_stored` (raw-deflate-stored wrapped in zlib = exactly what PDF
  `/FlateDecode` accepts per ISO 32000-1 §7.4.4 / RFC 1950).
- 1 px = 1 pt; each MediaBox equals its frame size.
- Bounds enforced *before* allocation: `MAX_PAGES_PER_DOCUMENT` = 2 000
  (protocol session bound), per-frame `MAX_FRAME_PIXELS` / payload-length /
  `MAX_CAPTURE_BYTES` re-validation (catches hand-mutated `GrayFrame`s),
  and a 128 MiB conservative document bound (`MAX_DOCUMENT_BYTES`) computed
  from worst-case stored-stream framing + fixed per-object overhead before
  assembly. u32 stream lengths and byte offsets use checked arithmetic.
- Deterministic by construction (frozen object numbering, dictionary text,
  fixed `/ID`), locked by two golden-digest unit tests.
- `check_document` re-parses the finished bytes before returning: header /
  `%%EOF`, trailer `/Size` + `/Root`, `/Pages /Count`, `startxref`
  placement, every 20-byte xref entry (free-entry shape, in-use type flag,
  offset pointing exactly at the `<n> 0 obj` header), each image
  dictionary geometry + `/Length`, and full payload fidelity by
  re-inflating every embedded stream and comparing to the source frame.
  A failure raises `Category::InternalError` and emits no bytes.

## Verification actually performed (local, this machine)

| Gate | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --locked -- -D warnings` | clean |
| `cargo test --locked` | 148 passed; 0 failed (baseline on main was 137; +8 pdf unit, +3 pdf integration) |

New tests: 8 in-crate unit tests (5 rejection paths incl. empty doc,
oversized/mismatched/zero-dimension frames, both document bounds; 2 golden
digests; 1 extended structural check) and 3 integration tests in
`tests/pdf_export.rs` (fixture byte drift pin, writer byte-for-byte
reproduction, PyMuPDF-recorded pixel digests equal recomputed
`GrayFrame::digest()`).

## Independent-parser validation (generation time, not CI)

`tests/fixtures/pdf/generate.py` (`pymupdf` 1.28.2) drives
fixture generation through the crate's `gen_pdf_fixture` example, then
opens each committed fixture with PyMuPDF and asserts: exact page count,
exact per-page MediaBox pixel dimensions, exactly one image XObject per
page, and pixel-exact decode (extracted samples byte-equal to the source
LCG frame, recorded as `GrayFrame::digest()` recipes in
`tests/fixtures/pdf/manifest.json`). All generation-time assertions passed:

```text
OK one_page_5x4.pdf: 1 page(s), sha256=9413e09ab8b77624…
OK three_pages.pdf: 3 page(s), sha256=ca6b12880167c815…
wrote manifest.json
```

The loop closed at test time is: source frame → `export_pdf` → committed
bytes (SHA-256-pinned) → PyMuPDF pixels (generation time) → digest equals
the source frame's digest — so the independent parser actually recovered
our exact pixels from the container we produced.

## Known limitations / honest gaps

- Validated with **one** independent parser (PyMuPDF) on synthetic frames.
  No broad conformance sweep across readers, no print-driver rasterization,
  no ISO 32000-1 validator tool. The crate does not parse PDFs at all.
- Uncompressed image streams (stored deflate) — documents are ~raw size
  plus framing; same stance as the PNG encoder, revisit with the
  compression slice.
- 1 pt = 1 px is a documented convention, not metadata; if a future slice
  wants real physical page sizes it must add unit metadata deliberately.
- The export executor and `foldscan.export/0.1` manifest do not yet know
  how to carry a PDF derivative; that integration is a later slice
  decision (deliberately out of scope here).
- Fixture bytes are self-generated (our writer produced them); independence
  is only on the *parse* side, exactly as with the PNG encoder's pinned
  output. Stated so no one over-reads the evidence.
