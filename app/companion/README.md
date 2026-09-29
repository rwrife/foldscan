# FoldScan companion — application scaffold (issue #37)

The desktop companion shell for issue #5's first acceptance criterion: a
Tauri 2 + Rust + TypeScript application with exact pinned dependencies,
formatting/lint gates, tests, and clean CI builds on the Linux host this
project can build.

This shell additionally exposes **one bounded import probe, in-memory review plan,
and read-only export preview** (issues #39, #41, #43, and #47): the user can
choose a mounted-volume directory through the native system picker or enter its
path manually. The UI sends that path to `import_volume_summary`, the shell runs
the already-tested `foldscan_domain::import_volume` pipeline, and the verified
captures are presented as an accessible, manifest-ordered capture list. Users
can reorder captures, exclude individual items from export, and restore them
without modifying or deleting any source files. Each reviewed session carries a
`Preview export` button that sends its session ID and active capture order to
`preview_export_plan`; the shell re-runs the bounded import, validates the
selection, and returns the canonical planned export files (originals in
reviewed order plus `export.json`) and the manifest integrity digest — writing
nothing. Selecting a directory, manipulating the review plan, or previewing an
export does not run export or mutate the volume. The shell still contains
**no image thumbnail decoding, image processing, or export execution views**.

## Layout

```
app/companion/
  ui/               TypeScript + Vite frontend (pinned in package.json,
                    package-lock.json committed)
  src-tauri/        Tauri 2 Rust shell crate (`=` pins, Cargo.lock committed),
                    path-dependency on app/domain (foldscan-domain)
```

## Pinned stack

| Piece        | Pin (exact)  | Notes                              |
|--------------|--------------|------------------------------------|
| `tauri`      | `=2.11.6`    | Rust shell backend                 |
| `tauri-build`| `=2.6.3`     | shell build script                 |
| `serde`      | `=1.0.219`   | matches `app/domain`               |
| `serde_json` | `=1.0.141`   | matches `app/domain`               |
| `@tauri-apps/api` | `=2.11.1` | UI → shell `invoke` boundary    |
| `tauri-plugin-dialog` | `=2.7.0` | Rust native dialog plugin    |
| `tauri-plugin-fs` | `=2.5.0` | Compatibility pin for Serde 1.0.219 |
| `@tauri-apps/plugin-dialog` | `=2.7.0` | UI folder picker API   |
| `typescript` | `=5.9.3`     | `tsc --noEmit` typecheck gate      |
| `vite`       | `=8.3.0`     | dev server + production build      |

`foldscan-domain` is a path dependency, so the shell always compiles against
the domain crate in this repository — the companion status command reports
`foldscan_domain::version::{SUPPORTED_MAJOR, KNOWN_MINOR}` directly.

## Commands

Frontend (from `app/companion/ui`, Node 22):

```bash
npm ci
npm test            # review-plan state + axe WCAG semantic regression tests
npm run typecheck   # tsc --noEmit
npm run build       # tsc --noEmit && vite build -> dist/
```

The review plan is intentionally ephemeral. A new import check replaces it,
and closing the app discards it. `Remove` means remove from the prospective
export order only; the verified source capture remains on the mounted volume.
Reorder/remove/restore actions are native buttons and remain keyboard-operable.

Shell (from `app/companion/src-tauri`, requires the Linux WebKitGTK
prerequisite set — `libwebkit2gtk-4.1-dev` and friends, see the CI workflow):

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Full app development run (`npm run dev` + `cargo tauri dev`) additionally
needs a desktop session; CI does not launch a GUI.

## What CI verifies (`.github/workflows/app-companion.yml`)

- `ui` job: `npm ci` from the committed lockfile, six deterministic
  review-plan and request tests, three accessibility-regression tests, then
  `tsc --noEmit` + a production Vite build on `ubuntu-latest` (Node 22). The
  review tests exercise initial manifest order, export preview request
  construction, movement and boundary behavior, remove/restore, and
  source-summary immutability. The exact-pinned `axe-core`/`jsdom` audit checks
  the real initial `index.html` and a representative populated import, review,
  and export-preview DOM against applicable WCAG 2.0/2.1 A/AA rules. A canary
  proves an unlabeled button fails with rule and selector evidence.

- `backend` job: apt-install of the Tauri Linux prerequisites, then
  `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  and `cargo test --locked` against the shell crate (which compiles
  `foldscan-domain` via path and exercises the 10 shell unit tests).

## Verified / not verified (honesty box)

Verified locally on the executor host (Linux, headless, arm64) and in CI
(`ubuntu-latest`, x86-64):

- `npm ci`, `npm test`, `tsc --noEmit`, and the production Vite build.
- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked` for the shell crate (including its link against
  `foldscan-domain`), run inside an Ubuntu 24.04 Docker container with the
  WebKitGTK prerequisites installed.

The UI tests cover deterministic in-memory review-state transitions, reviewed
request construction, and headless semantic accessibility regressions. Rust
fixture tests prove that the preview re-imports source metadata, rejects stale,
duplicate, empty, and unknown selections, preserves reviewed order, and returns
the domain planner's canonical files and manifest digest without creating a
destination. They are not export execution, GUI, assistive-technology, mounted
media, or device tests. `color-contrast` is deliberately excluded from the
jsdom axe run because jsdom has no rendered pixels; contrast requires a real
browser/desktop acceptance pass rather than a false static pass.

Not verified — explicitly out of scope for this scaffold:

- The GUI window was never launched (no desktop session on the executor
  host); no screenshot, input, or screen-reader evidence exists yet.
- No installers/bundles were produced (`bundle.active = false`). The
  committed `icons/icon.png` is a generated placeholder: Tauri 2 requires a
  real icon file to compile `tauri::generate_context!`, so the scaffold
  carries a plain rounded square until branding assets are chosen. Platform
  icon sets (.ico/.icns) and packaging/signing policy are later #5 slices.
- Windows and macOS builds are not exercised by this CI workflow; only the
  Linux lane exists.
- The native folder picker was not opened in the headless test environment;
  mounted-volume, picker cancellation, and focus-return behavior need GUI
  acceptance testing on supported desktops.
- The review-plan and export-preview buttons were not exercised in a launched
  GUI; keyboard-only, screen-reader, and focus-restore behavior of the rendered
  list and preview status need desktop acceptance testing.
- No destination was selected and no export was executed. The preview proves
  planning only; filesystem materialization, cancellation, rollback, and output
  inspection remain separate integration/GUI work.
- Automated accessibility evidence is limited to axe-detectable semantic
  structure in the initial and representative populated states. Native picker
  focus return, keyboard journeys in a launched WebView, screen-reader output,
  rendered contrast, high contrast/forced colors, reduced motion, and 200%
  zoom/reflow still need GUI acceptance testing; #5's accessibility acceptance
  criterion is not complete.

## Roadmap back to #5

Next slices plug real functionality into this shell: import browsing, page
review, processing, and export views, each as domain-first slices reusing
the already-tested `app/domain` core.
