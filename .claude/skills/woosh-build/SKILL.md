---
name: woosh-build
description: Build, test, verify or bundle the Woosh app. Use when asked to build the project, run the tests, check CI, produce the .app or DMG, regenerate the test fixture, or diagnose a failing build. Covers the Docker/host split and the known failure modes.
---

# Building and verifying Woosh

## The one rule that shapes everything

**Docker does all building and verification. The host does exactly one thing:
bundle the macOS `.app`.**

This is not a preference. A macOS Tauri bundle needs `MacOSX.sdk`,
`WebKit.framework` (`wry` links `WKWebView`) and `codesign` — none of which exist
in a Linux container. Docker Desktop here runs `linux/aarch64`; a Tauri build
there links `webkit2gtk` and produces an ELF binary that cannot run on macOS.

The host already has `rustc`/`cargo` and `pnpm`. **Never install anything on the
host** — not via `cargo install`, `brew`, or `rustup component add` without asking.

## Normal workflow

```bash
make doctor     # read-only: toolchain + signing readiness. Run this first when something is off.
make ci         # fmt, clippy, tests, workspace typecheck, frontend build, bench — all Docker
make app        # HOST: bundles target/aarch64-apple-darwin/release/bundle/macos/Woosh.app
make dmg        # HOST: ad-hoc signs and reports the shareable DMG
```

Faster inner loop:

```bash
make check             # fmt + clippy + test (Docker)
make test              # engine suite only
make check-workspace   # typechecks the Tauri shell too, on Linux — catches broken
                       # #[tauri::command] signatures without a Mac
```

## Verifying the macOS-only code path

`getattrlistbulk` cannot run in Docker, so the Linux suite exercises
`fastwalk/portable.rs` instead. To test the real macOS path:

```bash
make fixture        # generates .fixture/ with known answers
make verify-macos   # runs the suite natively against getattrlistbulk
```

Both backends are asserted against the **same fixture with the same expected
values**. If they disagree, one of them is wrong — that differential is the point.

Always run both when touching `engine/src/fastwalk/`.

## Seeing it work on real data

```bash
cargo run --release --example headless            # real sweep, no GUI, prints findings + phase timings
cargo run --release --example headless -- --deep
cargo run --release --example headless -- --root ~/dev
```

This is the fastest way to confirm end-to-end behaviour, including the Docker
socket, the Ollama API and interpreter auditing. It deletes nothing.

## Known failure modes

| Symptom | Cause and fix |
|---|---|
| `failed to load manifest for workspace member app/src-tauri` | Ran before the shell existed, or from the wrong directory. Run `make` from the repo root. |
| `Couldn't recognize the current folder as a Tauri project` | The Tauri CLI searches *subfolders* of the CWD. It must run from `app/`, where `src-tauri/` lives. The Makefile already does this. |
| `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY` | Host and container fought over `app/node_modules`. The compose `ui` service masks it with an anonymous volume; if it recurs, `rm -rf app/node_modules && (cd app && CI=true pnpm install)`. |
| `failed to open icon .../32x32.png` | Icons missing. `python3 scripts/make-icon.py app/src-tauri/icons` regenerates the whole set including `.icns`. |
| `fixture not found` | `make fixture`. Note `cargo bench` runs with CWD at the package root, so lookup checks `.fixture` and `../.fixture`. |
| Tests fail only in Docker | The container runs as **uid 0**, which ignores permission bits, and has no `/System` or `/Applications`. Do not write tests that depend on either. |

## Inspecting the built bundle

Read plist keys **non-destructively** — `plutil -extract KEY json file.plist`
without `-o -` **rewrites the file**:

```bash
P=target/aarch64-apple-darwin/release/bundle/macos/Woosh.app/Contents/Info.plist
plutil -extract LSMinimumSystemVersion raw -o - "$P"
plutil -lint "$P"
codesign -dv --entitlements - target/.../Woosh.app
```

Expected: `com.woosh.cleaner`, `LSArchitecturePriority = ["arm64"]`,
`LSMinimumSystemVersion = 11.0`, the three `NS*FolderUsageDescription` strings,
and `flags=0x10002(adhoc,runtime)`.
