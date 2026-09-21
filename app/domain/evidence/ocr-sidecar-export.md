# OCR sidecar export binding — evidence (issue #33, child of #5)

## What landed

Per-capture, *completed* `foldscan.ocr/0.1` documents now bind into the
three export layers, mirroring the session-document (slice 11) pattern:

- **Model** — `ExportSession.ocr: Vec<OcrResult>` (empty = exactly prior
  behavior). `validate_ocr_bindings` (export.rs) checks the whole list:
  every doc re-validates, only `Completed` status binds (`Failed`/`Skipped`
  are host review state — issue #5's "OCR failure must not block image/PDF
  export" holds *structurally*: failure information cannot reach the layout
  or manifest at all), every sidecar binds to an in-export capture, and at
  most one sidecar per capture.
- **Planner** — new `ContentKind::Ocr`, laid out at
  `ocr/<session>/<capture_id>.json` after page/document files, before
  recipes; `export.json` still last. Iteration follows *session page
  order*, so the host's binding-list order can never change the plan
  (pinned by `binding_order_cannot_change_layout_or_manifest_digest`).
- **Manifest** — `ExportManifestPage.ocr_digest: Option<String>` carries the
  sidecar's canonical-JSON content digest (`OcrResult::digest()`), covered
  by the manifest integrity digest. Present iff the plan lays out a sidecar
  for that capture — `from_plan` cross-checks binding-vs-layout set
  equality per capture so an edited plan can neither claim a digest for an
  unlayouted sidecar nor omit one for a layouted sidecar. Absent =
  `skip_serializing_if`, byte-identical to pre-slice manifests (all 181
  pre-existing tests incl. golden digests unchanged).
- **Executor** — writes sidecars by serializing the *bound* document from
  the plan's session (`ocr_document`), never from the host sources map; a
  source supplied for a sidecar path is rejected pre-disk with a specific
  message. Writes pass the same staged `.foldscan-part` → flush →
  read-back → atomic-rename gates, with a read-back proof: the serialized
  bytes re-parse through `OcrResult::from_json_bytes` to exactly the bound
  document before writing. Cancellation/rollback semantics unchanged.

Serialization-shape note: `OcrBlock`/`OcrStatus`/`OcrResult` gained `Eq`
(no representation change; JSON bytes are unaffected).

## Commands and actual results (Linux aarch64, rustc 1.98.0)

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets --locked -- -D warnings` — clean.
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps` — clean.
- `cargo test --locked` — **197 passed, 0 failed** across 14 suites. The
  16 new tests live in `tests/ocr_sidecar_export.rs`; every pre-existing
  test (including all golden digests from earlier slices) passed unmodified
  except adding the new `ocr` field to struct literals.

Mutation canary: replacing the completed-status guard
(`!matches!(doc.status, Completed {..})`) with `false` fails exactly
`planner_rejects_failed_and_skipped_sidecars`; restoring returns 16/16.

## Evidence category

Software-fixture only. The OCR documents in the tests are authored JSON
bound into an export; **no OCR engine exists** and nothing here recognizes
text or measures recognition quality. Executor tests run on real files in a
temp directory. No device, no network, no bench or physical claims.

## Known gaps / limitations

- The bound document travels as its canonical JSON sidecar
  (`ocr/<session>/<capture>.json`); a plain-`.txt` rendering of recognized
  text is a separate follow-up slice.
- OCR content is not embedded in the session-level PDF.
- `reorder`/`remove_from` strand stale bindings by design; planning rejects
  them (proven by tests) rather than repairing silently.
- One session per executor run, unchanged.
