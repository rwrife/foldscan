# FoldScan companion — application scaffold (issue #37)

The desktop companion shell for issue #5's first acceptance criterion: a
Tauri 2 + Rust + TypeScript application with exact pinned dependencies,
formatting/lint gates, tests, and clean CI builds on the Linux host this
project can build.

This scaffold intentionally contains **no UI workflows yet** — no import,
review, processing, or export views. It contains one minimal accessible page
that asks the Rust shell for a versioned status document and renders it as
text, plus the CI wiring that keeps every later slice honest.

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
| `typescript` | `=5.9.3`     | `tsc --noEmit` typecheck gate      |
| `vite`       | `=8.3.0`     | dev server + production build      |

`foldscan-domain` is a path dependency, so the shell always compiles against
the domain crate in this repository — the companion status command reports
`foldscan_domain::version::{SUPPORTED_MAJOR, KNOWN_MINOR}` directly.

## Commands

Frontend (from `app/companion/ui`, Node 22):

```bash
npm ci
npm run typecheck   # tsc --noEmit
npm run build       # tsc --noEmit && vite build -> dist/
```

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

- `ui` job: `npm ci` from the committed lockfile, then `tsc --noEmit` + a
  production Vite build on `ubuntu-latest` (Node 22).
- `backend` job: apt-install of the Tauri Linux prerequisites, then
  `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  and `cargo test --locked` against the shell crate (which compiles
  `foldscan-domain` via path).

## Verified / not verified (honesty box)

Verified locally on the executor host (Linux, headless, arm64) and in CI
(`ubuntu-latest`, x86-64):

- `npm ci`, `tsc --noEmit`, and the production Vite build.
- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked` for the shell crate (including its link against
  `foldscan-domain`), run inside an Ubuntu 24.04 Docker container with the
  WebKitGTK prerequisites installed.

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
- Accessibility is limited to structural hygiene in the scaffold (landmarks,
  `aria-live` status, visible focus, skip link, reduced-motion and
  forced-colors CSS); #5's accessibility acceptance testing is not done.

## Roadmap back to #5

Next slices plug real functionality into this shell: import browsing, page
review, processing, and export views, each as domain-first slices reusing
the already-tested `app/domain` core.
