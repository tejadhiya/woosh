# Woosh — agent context

Developer storage reclaimer for Apple Silicon. Tauri v2 shell + pure-Rust engine.
Ported from a Python prototype at `/Users/forthtech/Desktop/storage-checker`
(still the reference for domain logic; read it before changing detection rules).

Design decisions and measured results: **`docs/IMPLEMENTATION_PLAN.md`**.
Human workflows and troubleshooting: **`docs/DEVELOPMENT.md`**.

---

## Hard rules

**1. Never try to build the macOS `.app` in Docker.** It is physically impossible:
no Linux container has `MacOSX.sdk`, `WebKit.framework` (`wry` links `WKWebView`),
or `codesign`. Docker does *everything else* — deps, fmt, clippy, the whole test
suite, full-workspace `cargo check`, the frontend bundle, benchmarks. Only
`make app` / `make dmg` / `make release` run on the host, using the Rust and pnpm
already installed there. **Install nothing on the host.**

**2. Route work through `make`.** `make check`, `make test`, `make ci`. Running
`cargo` directly on the host is for the macOS-only differential (`make verify-macos`)
and the headless example. Never `cargo install` anything.

**3. Do not weaken `engine/src/safety.rs`.** Every rule there closes a specific
hole, and each has a test asserting a *refusal*. If a path is being wrongly
refused, fix the caller, not the boundary.

**4. Deletion invariants** (`engine/src/reaper.rs`):
- A venv is never removed until `req-inplace.txt` is written to the project root.
  A failed write **aborts** the deletion.
- A failed move-to-Trash is a failed receipt. It must **never** fall through to
  permanent deletion — the Python prototype does this and it is the bug we fixed.
- Only `safety::vet()` produces a `Vetted`, and the reaper accepts nothing else.
  Paths are re-vetted immediately before deletion (TOCTOU).

**5. `engine/src/fastwalk/macos.rs` offsets were derived empirically**, by dumping
real `getattrlistbulk` buffers — *not* from the headers. The non-obvious part:
`ATTR_DIR_ALLOCSIZE` and `ATTR_FILE_ALLOCSIZE` share **one** 8-byte slot. Assuming
two slots puts the read on top of the filename and yields exabyte-scale garbage.
Do not "correct" these from `attr.h` alone. Re-derive with the C probe in
`docs/DEVELOPMENT.md` if you must touch them.

**6. Keep the byte-level differential.** `fast_walker_agrees_with_std_on_counts_and_bytes`
compares the fast walker against an independent `std` walk on **both counts and
bytes**, and asserts `exclusive` and `exclusive + shared` separately. The offset
bug above produced *perfect file counts* throughout — only bytes caught it.

**7. The engine must not depend on Tauri.** That dependency is the only reason
the suite runs in Linux Docker. Anything needing `tauri::` belongs in
`app/src-tauri/src/lib.rs`, which stays thin.

---

## Layout

```
engine/              pure Rust, no Tauri. The whole suite runs in Linux Docker.
  fastwalk/          macos.rs = getattrlistbulk(2); portable.rs = std::fs. One trait.
  sweep.rs           discover() depth-bounded + measure() pure-sum, both rayon
  dedup.rs           (dev, ino) set -> exclusive vs shared bytes
  safety.rs          deletion boundary (see rule 3)
  reaper.rs          trash-backed deletion, receipts, JSONL audit log
  telemetry.rs       60 Hz frame coalescer + log voice
  http.rs            tiny sync HTTP/1.1 over UDS + TCP (Docker socket, Ollama API)
  domains/           venv node global_python models docker caches artifacts
  tests/fixture.rs   integration tests against scripts/make-fixture.sh
app/                 Svelte 5 frontend + thin Tauri shell (src-tauri/)
docker/              build + verification images
scripts/             fixture, icon, doctor, dmg, release
```

## Commands

```bash
make help            # all targets
make doctor          # host toolchain + signing readiness (read-only)
make ci              # everything CI does, all in Docker
make fixture         # known-answer test tree at .fixture/
make app             # bundle Woosh.app          (HOST ONLY)
make dmg             # ad-hoc sign + report DMG  (HOST ONLY)
cargo run --release --example headless   # real sweep, no GUI
```

## Traps that have already cost time

- **`plutil -extract KEY json file.plist` rewrites the file in place.** Always
  pass `-o -` to read. This silently destroyed a built `Info.plist` once.
- **Host and container `node_modules` must stay separate.** The compose `ui`
  service masks `app/node_modules` with an anonymous volume. Without it, the
  container installs Linux binaries where the host pnpm expects darwin ones, and
  the host build then dies with `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY`.
- **`cargo bench` runs with CWD at the package root**, not the workspace root, so
  fixture lookup checks both `.fixture` and `../.fixture`.
- **Tests run in parallel.** Never write scratch files into `.fixture/` — it races
  the differential's file count. Use `tempfile::tempdir()`.
- **Sign during the build, never after.** `tauri build` emits the `.app` and the
  `.dmg` in one pass, so signing `bundle/macos/Woosh.app` afterwards leaves the DMG
  holding an unsigned copy — which macOS reports to recipients as *"Woosh is
  damaged and can't be opened"*, and `xattr` does not fix it. `make app` exports
  `APPLE_SIGNING_IDENTITY` (default `-`) into the build. Always verify the app
  **inside** the DMG, not the sibling copy; `make dmg` does this and fails loudly.
- **Micro-benchmarks have lied here.** Thread-pool oversubscription measured +7%
  in isolation and −18% in the full pipeline. Measure the pipeline, not the part.
