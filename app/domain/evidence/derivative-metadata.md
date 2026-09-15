# Derivative metadata preflight evidence — 2026-09-15

Issue: https://github.com/rwrife/foldscan/issues/21 (bounded child of #5).
Base revision: `99d05d3b6da3c100644b841399c39858fdcada41`.

## Contract and compatibility

- An original-only `ExportPage` has no processed checksum, byte count, recipe
  digest, or processed media type. A derivative requires the first three;
  its recipe digest must reference a supplied recipe. A missing processed
  media type still defaults to PNG for a complete derivative.
- An `ExportManifestPage` carries processed path, checksum, byte count, and
  recipe digest together, or none. Checksums and recipe digests are
  64-character lowercase hexadecimal SHA-256 values.
- The processed byte limit is the existing `MAX_CAPTURE_BYTES` (64 MiB).
  Zero remains an accepted metadata declaration for compatibility; this
  validator does not promise a valid image or decode any payload.
- Malformed previously accepted partial records now return `InvalidRequest`.
  Valid original-only and complete derivative metadata retain schema 0.1.
  No dependencies, lockfile, or serialization field names changed.
- `execute_export` already invokes both validators before creating parent
  directories. Public plan/manifest mutations cannot bypass the new checks.
  Manifest validation alone cannot establish that recipe files exist or that
  encoded image dimensions match; planner/executor checks are separate.

## Reproductions (actual failing tests)

Tests were added and exercised before each corresponding production change.
All four focused commands used `cargo test --locked --test derivative_metadata
<test_name>` and exited 101 before their fix:

| Regression function | Observed failure before fix |
|---|---|
| `planner_requires_complete_derivative_metadata` | Accepted partial masks `[2, 4, 5, 6, 8, 10, 12, 13, 14]` |
| `manifest_requires_complete_derivative_metadata` | Both `validate` and JSON parsing accepted every partial mask 1–14 |
| `derivative_byte_limit_is_checked_by_planner_and_manifest` | Planner, validator, and parser accepted 67,108,865 and `u64::MAX` |
| `derivative_digests_must_be_lowercase_sha256` | Validator and parser accepted empty, uppercase, nonhex, short, and long recipe digests |

The fifth regression, filesystem preflight, was replayed against the unchanged
base in a separate detached worktree, with only the new test file copied in:

```sh
CARGO_TARGET_DIR=/tmp/foldscan-21-red-target cargo test --locked \
  --manifest-path /home/rwrife/repos/foldscan-wt-derivative-red/app/domain/Cargo.toml \
  --test derivative_metadata \
  executor_rejects_invalid_derivatives_without_creating_parent_or_touching_original
```

Actual result: **FAILED**, exit 101, `case 2: parent created:
Ok(ExportExecution { ... files_written: 2, ... })`. The old code successfully
exported a page with an orphan processed byte count. The fixed code rejects
all 15 invalid executor cases before creating the parent directory, and the
fixture asserts the original source bytes remain unchanged in each case.

## Local verification

Host: Linux/aarch64. `rustc 1.98.0 (88d9e12ae 2026-08-18)`;
`cargo 1.98.0 (797e8a9bc 2026-08-05)`.

Commands run in `app/domain` (test/log pipelines used `set -euo pipefail`):

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
git diff --check
```

All exited 0. Format/diff checks emitted no errors; clippy completed with no
warnings. Test output summaries:

| Target | Passed | Failed |
|---|---:|---:|
| Library unit tests | 51 | 0 |
| Derivative metadata (new) | 5 | 0 |
| Export execution | 16 | 0 |
| Export round trip | 1 | 0 |
| Golden processing | 7 | 0 |
| Import fixtures | 15 | 0 |
| Path safety | 6 | 0 |
| Version negotiation | 7 | 0 |
| **Total** | **108** | **0** |

Doc-tests: 0. Baseline before edits: 103 passed, 0 failed; fmt/clippy clean.
The five new functions cover all 16 planner presence masks, all 16 manifest
presence masks through direct validation and JSON, four size boundaries,
ten malformed digest cases across three entry points, and 15 filesystem
preflight cases. Oversize tests inspect declarations without large allocations.

## Review and CI provenance

The implementing PR records the independent review result and exact GitHub
Actions run URLs/head SHA after execution. This document records local output,
not a claim that future CI or release acceptance has already occurred.

## Evidence category and limits

Software metadata/filesystem fixtures only. Fixture bytes are synthetic, not
valid JPEG/PNG captures. No network access is added; originals stay local and
unchanged. No desktop shell, accessible UI, image/PDF codecs, OCR, installer,
real device, physical durability, or optical measurements are validated here.
KiCad ERC/DRC, BOM/datasheet, EMC/thermal, firmware, and mechanical checks are
not applicable to this Rust-only change and were not run. #5 and the hardware
acceptance/dependency gates remain open.
