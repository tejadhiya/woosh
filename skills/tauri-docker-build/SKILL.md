---
name: tauri-docker-build
description: Set up or fix a containerized build and verification pipeline for a Tauri v2 + Rust desktop app that ships on macOS. Use when asked to build a Tauri app in Docker, keep a host machine clean of toolchains, set up CI for a Rust desktop app, or diagnose why a Dockerized Tauri build fails. Covers what can and cannot be containerized, the crate split that makes it work, and the known failure modes.
---

# Containerized builds for a macOS Tauri app

## The constraint that shapes everything

**A macOS `.app` cannot be produced inside a Linux container.** Do not attempt it,
and do not let a request for "Docker only" push you into shipping something broken.

| Needed to bundle a macOS app | Why Linux can't |
|---|---|
| `MacOSX.sdk` + `WebKit.framework` | Tauri's webview layer (`wry`) links `WKWebView`. Not redistributable, not present. |
| `codesign`, `lipo`, `actool` | Apple-only Mach-O and bundle tooling. |
| `.app` bundle assembly | `cargo tauri build` shells out to the above. |

A Tauri build in a Linux container links `webkit2gtk` and emits an **ELF binary
that cannot run on macOS**.

## The split that does work

> Docker is the authoritative build and verification environment.
> The host runs exactly one command: the final bundle.

Docker handles dependency resolution, `fmt`, `clippy -D warnings`, the entire test
suite, **full-workspace `cargo check` including the Tauri shell**, the frontend
bundle, and benchmarks. The host runs `tauri build`, using a toolchain it already
has. Nothing new is installed on the host.

When a user asks for "Docker only", surface this constraint explicitly and let
them choose — do not silently pick one. The usual answer is the hybrid.

## Crate split (do this first — everything depends on it)

```
workspace/
  core/        <- ALL logic. NO tauri dependency. Compiles and tests on Linux.
  app/
    src-tauri/ <- thin shell: commands, event bridge, window chrome
    src/       <- frontend
```

**The core crate must not depend on `tauri`.** That single rule is what lets the
test suite run in a Linux container. Anything needing `tauri::` goes in the shell,
which stays thin enough that "untestable on Linux" costs nothing.

Platform-specific code gets a trait with two implementations behind `#[cfg]`, and
**both are tested against the same fixtures with the same expected values**. That
differential is what catches a platform backend silently disagreeing.

## Dockerfile

Two stages. The slim one is the fast inner loop; the fat one exists only so the
Tauri shell's dependency graph resolves for `cargo check`.

```dockerfile
FROM rust:1.94-slim-bookworm AS engine
ENV CARGO_TERM_COLOR=always CARGO_INCREMENTAL=0 \
    CARGO_TARGET_DIR=/build/target RUSTFLAGS="-D warnings"
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev ca-certificates git && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy
WORKDIR /src

FROM engine AS workspace
# Tauri v2 Linux prerequisites. Present ONLY so `cargo check` can resolve the
# shell crate's graph — never used to produce a shipped artifact.
RUN apt-get update && apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
      librsvg2-dev libxdo-dev build-essential curl wget file \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
```

Checking the Tauri shell on Linux is worth the image size: it catches broken
`#[tauri::command]` signatures and event plumbing without a Mac in the loop.

## docker-compose

Use YAML anchors so every service shares the same mounts and cache volumes.

```yaml
x-rust: &rust
  build: { context: .., dockerfile: docker/Dockerfile.engine, target: engine }
  volumes:
    - ../:/src
    - cargo-registry:/usr/local/cargo/registry
    - cargo-git:/usr/local/cargo/git
    - build-target:/build/target      # keep target/ out of the bind mount: much faster
  working_dir: /src

services:
  fmt:    { <<: *rust, command: cargo fmt --all -- --check }
  clippy: { <<: *rust, command: cargo clippy -p core --all-targets -- -D warnings }
  test:   { <<: *rust, command: cargo test -p core --all-targets }
  ui:
    build: { context: .., dockerfile: docker/Dockerfile.ui }
    volumes:
      - ../:/src
      - pnpm-store:/pnpm/store
      - /src/app/node_modules    # anonymous volume — see failure modes below
    working_dir: /src/app

volumes: { cargo-registry:, cargo-git:, build-target:, pnpm-store: }
```

## Makefile

Make the Docker/host boundary visible in the file itself.

```make
DC := docker compose -f docker/docker-compose.yml
TARGET := aarch64-apple-darwin

# ---- containerized ----
test:            ; $(DC) run --rm test
clippy:          ; $(DC) run --rm clippy
check-workspace: ; $(DC) run --rm check-workspace
ci: check check-workspace ui bench

# ---- host only: bundling needs WKWebView + codesign ----
# APPLE_SIGNING_IDENTITY must be exported INTO the build (see macos-app-bundle).
app:
	cd app && pnpm install && \
	  APPLE_SIGNING_IDENTITY="$${APPLE_SIGNING_IDENTITY:--}" \
	  pnpm exec tauri build --target $(TARGET)
```

**Get the Tauri CLI from the frontend's `devDependencies`** (`@tauri-apps/cli`),
invoked as `pnpm exec tauri`. It is then project-local — no `cargo install`, no
host pollution.

## Failure modes

| Symptom | Cause / fix |
|---|---|
| `Couldn't recognize the current folder as a Tauri project` | The CLI searches *subfolders* of the CWD for `tauri.conf.json`. It must run from the directory that **contains** `src-tauri/`. Use the conventional layout: frontend at `app/`, `app/src-tauri/` inside it. |
| `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY` | Host and container fought over `node_modules`. The container installed Linux binaries where host pnpm expects darwin ones. Mask it with an anonymous volume (above). Recover: `rm -rf app/node_modules && (cd app && CI=true pnpm install)`. |
| `failed to load manifest for workspace member` | A workspace member listed in `Cargo.toml` has no `Cargo.toml` yet. Scaffold the shell before the first `cargo` run. |
| `failed to open icon .../32x32.png` | `tauri::generate_context!()` is a proc macro and resolves icons at **compile** time. Generate icons before any build. |
| Tests pass on host, fail in Docker | The container runs as **uid 0**, which ignores permission bits, and has no `/System` or `/Applications`. Never write tests depending on either. Block a write structurally (e.g. put a directory where a file must go), not with `chmod`. |
| `cargo bench` can't find fixtures | Benches run with CWD at the **package** root, not the workspace root. Check both `./fixture` and `../fixture`. |

## Verifying both platform backends

The containerized suite exercises only the portable implementation. Add a host
target that runs the same suite natively against the macOS one:

```make
verify-macos: ; cargo test -p core --all-targets
```

Run both whenever platform-specific code changes. If they disagree, one is wrong —
that is the entire point of keeping two.
