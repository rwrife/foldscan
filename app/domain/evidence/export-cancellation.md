# Cooperative export cancellation — 2026-09-16

Issue: https://github.com/rwrife/foldscan/issues/23 (bounded child of #5).
Base: `2c77c660b23e26db96c7554366b6793ffbc91b8d`.

## Scope and behavior

`executor::execute_export_cancellable(plan, root, sources, manifest, callback)`
adds opt-in host cancellation. `execute_export` uses the same implementation
with an always-false callback. No dependencies or device wire schemas change.
`Category::Cancelled` is host-local; consumers matching the public enum
exhaustively must handle the new variant.

Request validation occurs first. Cancellation is polled once before any
parent/root creation and before each planned file. A true response returns
`Cancelled`; if this invocation created the root, its existing rollback path
removes that tree. Source originals, pre-existing destinations, and unrelated
siblings are not cleanup targets. Tests demonstrate retry into the removed root.

The final cancellation poll is immediately before manifest writing begins.
After that point, cancellation is not observed: normal finalization/audit can
return success; I/O/audit failures still roll back. A cancellation request is
not a claim that an operation already committed was undone.

## Executed evidence

Evidence category: real Rust execution with synthetic data and temporary host
filesystem I/O. Not hardware, codec, GUI, or physical durability evidence.

Host: Linux aarch64; `rustc 1.98.0 (88d9e12ae 2026-08-18)`;
`cargo 1.98.0 (797e8a9bc 2026-08-05)`.

Baseline `cargo test --locked`: **108 passed, 0 failed**.

Test-first sequence:

1. Added `pre_cancel_leaves_export_parent_absent`. The first compile reported
   absent API/category (`E0425`, `E0599`). Added a delegating API scaffold,
   leaving cancellation unused to exercise an actual behavioral RED:
   `cargo test --locked --test export_execution pre_cancel_leaves_export_parent_absent`
   failed with `unwrap_err()` on `Ok(ExportExecution { files_written: 5, ... })`.
   Adding the pre-directory poll made it pass.
2. Added `cancellation_at_each_file_boundary_rolls_back_and_allows_retry`.
   Its focused command failed on `Ok(ExportExecution { files_written: 5, ... })`
   before per-file polling existed. Adding per-file polling made all five
   cancellation boundaries and retries pass.
3. Added compatibility/safety regressions using existing behavior: legacy vs
   never-cancel byte equality with omitted media type defaulting to `.png`,
   invalid requests rejected before polling, and existing destination
   preservation for both cancellation states.

Final local commands, each exit 0:

```text
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Actual test groups:

```text
unit:                51 passed; 0 failed
derivative_metadata:  5 passed; 0 failed
export_execution:    21 passed; 0 failed
export_round_trip:     1 passed; 0 failed
golden_processing:    7 passed; 0 failed
import_fixture:      15 passed; 0 failed
path_safety:          6 passed; 0 failed
version_negotiation:  7 passed; 0 failed
doc-tests:            0 passed; 0 failed
TOTAL:              113 passed; 0 failed
```

The boundary matrix observes the exact finalized file prefix and no manifest
before each cancel, then checks root removal, sibling/original preservation,
manifest parse/digest on retry, and no staging leftovers. The equivalence
fixture compares every output byte and both manifest digests. Callback polling
is also checked to stop before manifest creation, not after successful commit.
PR review and CI URLs/results are recorded on the pull request rather than
claiming a not-yet-run hosted build in this document.

## Limits and acceptance still open

- File-boundary polling, not asynchronous I/O interruption. Content files have
  a 64 MiB byte bound, not a time bound; validation, copy/hash/flush/read-back,
  and blocked OS operations cannot be interrupted by this API.
- The callback must be fast, non-panicking, and must not modify sources or the
  destination. Panic/process-death cleanup and hostile concurrent filesystem
  mutation are outside this contract.
- Existing rollback is best-effort if storage fails; failure to remove the
  tree is not separately reported by the existing cleanup path. Check root
  absence before retry; never delete an unknown existing destination. Parent
  directories created by the invocation are retained. No parent-directory
  fsync, power-cut, permission-loss, or failure-to-delete evidence is claimed.
- Single-session exports remain the current executor limit.
- Test derivative bytes are synthetic raw-frame payloads, **not PNG encoding**;
  `.png` checks prove the layout default, not codec validity.
- No Tauri shell/cancel button, accessibility review, PDF/OCR, device firmware,
  physical prototype, or optical bench test. Parent #5 and hardware gates
  remain open. KiCad/ERC/DRC/BOM/datasheet/EMC/thermal/mechanical/firmware tools
  are not applicable to these host-only Rust changes and were not run.
