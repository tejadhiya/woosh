# Developing Woosh

Everything you need to build, test, extend and ship this. For design rationale
and measured performance, see [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

---

## 1. The Docker/host split

**Docker does all building and verification. The host bundles the `.app`. That's it.**

A macOS Tauri bundle needs `MacOSX.sdk`, `WebKit.framework` (`wry` links
`WKWebView`) and `codesign`. None exist in a Linux container, and Docker Desktop
here runs `linux/aarch64` — a Tauri build there links `webkit2gtk` and produces an
ELF binary that cannot run on macOS.

So the containment boundary moved rather than the rule: Docker is the
authoritative environment for dependency resolution, `fmt`, `clippy -D warnings`,
the full test suite, **workspace-wide `cargo check` including the Tauri shell**
(the image carries `libwebkit2gtk-4.1-dev` purely so that graph resolves), the
frontend bundle and the benchmark. The host runs one command, with the toolchain
it already had.

The engine is a separate crate with **no Tauri dependency**. That is the only
reason the suite runs on Linux at all, and it is worth protecting.

## 2. First run

```bash
make doctor     # read-only check of toolchain, Docker, signing, optional integrations
make ci         # the whole pipeline, in Docker
make app        # HOST: produces Woosh.app
make dmg        # HOST: signs it and tells you how to share it
```

`make help` lists every target.

## 3. Inner loop

```bash
make test               # engine suite (Docker, ~1s once warm)
make check              # fmt + clippy + test
make check-workspace    # typechecks the Tauri shell on Linux too — catches a broken
                        # #[tauri::command] signature without needing a Mac
make ui                 # svelte-check + vite build
```

Seeing real behaviour without the GUI:

```bash
cargo run --release --example headless
cargo run --release --example headless -- --deep
cargo run --release --example headless -- --root ~/dev
```

It prints findings, per-phase timings and totals, and deletes nothing. This is
the fastest way to confirm the Docker socket, Ollama API and interpreter audit
paths actually work.

Running the GUI in dev mode:

```bash
make dev      # vite dev server + Tauri window, hot reload on the frontend
```

## 4. Testing

78 tests: unit tests beside the code, plus `engine/tests/fixture.rs` against a
known-answer tree.

```bash
make fixture         # build .fixture/ (~90 MB, gitignored)
make test            # Linux container -> fastwalk/portable.rs
make verify-macos    # host -> fastwalk/macos.rs (getattrlistbulk)
```

**Both backends run the same assertions against the same fixture.** That
differential is the core safety net for the syscall parser — run both whenever you
touch `engine/src/fastwalk/`.

Each fixture branch pins a specific behaviour: venvs under odd names, a package
with no `METADATA`, a hardlink farm, a sparse file, symlink escapes and loops, a
`target/` with and without `Cargo.toml`, build output that must *not* be reported.

### Writing tests here

- The container runs as **uid 0**, which ignores permission bits. A test that
  blocks a write with `chmod 0o500` passes on macOS and silently no-ops in CI.
  Block it structurally instead — e.g. put a directory where a file must be written.
- The container has **no `/System` or `/Applications`**. Assert refusals, not the
  existence of macOS-only paths.
- Tests run **in parallel**. Never write scratch files into `.fixture/` — use
  `tempfile::tempdir()`.
- Watch substring collisions in path assertions: `"js-pnp"` matches `"js-pnpm"`.
  Compare exact path components.

## 5. Architecture

```
  paths::default_scan_roots()
        │
        ▼
  sweep::discover()  ── depth-bounded, rayon. detect() claims directories using the
        │               listing already in hand, so marker checks cost no syscalls.
        ▼
  sweep::measure()   ── pure sum of allocated bytes, inode-deduplicated.
        │               No classification: the cheapest traversal that exists.
        ▼
  Finding { bytes, shared_bytes, risk, restore, meta }
        │
        ├── probes run concurrently: caches · models · docker · global python
        ▼
  safety::vet() ──► Vetted ──► reaper::reap() ──► Receipt ──► audit.jsonl
```

**Telemetry** is decoupled from scan speed on purpose. At hundreds of thousands of
entries/sec, one event per entry would mean ~800k IPC messages/sec. Instead
workers touch relaxed atomics and a sampling ring; a coalescer reads them at 60 Hz
and emits one frame. Discoveries are rare enough to ride a bounded channel.
Nothing in the hot loop can block on the UI.

**Sizes are allocated bytes** (`ATTR_FILE_ALLOCSIZE` / `st_blocks * 512`), never
apparent length. That is what you actually reclaim, and it makes sparse files
correct for free — `Docker.raw` reports ~12 GB real rather than 64 GB apparent,
with no special case.

**Hardlinks are counted once.** `pnpm` hardlinks into a shared store and APFS
clones copy-on-write, so the same bytes appear under many paths. Findings carry
`exclusive` (what deleting returns) and `shared` (freed only when the last
referrer goes) separately.

## 6. Touching `fastwalk/macos.rs`

The attribute buffer layout was derived **empirically**, not from headers. The
rule that is easy to get wrong: `ATTR_DIR_ALLOCSIZE` and `ATTR_FILE_ALLOCSIZE`
**share one 8-byte slot** — the kernel packs whichever applies. Assuming two slots
puts the file read 8 bytes late, on top of the filename, and parses filenames as
sizes (exabyte-scale nonsense, with *perfect file counts*).

If you must change it, re-derive with a C probe rather than reasoning from
`attr.h`:

```c
// cc -o probe probe.c && ./probe /some/dir
#include <stdio.h>
#include <string.h>
#include <fcntl.h>
#include <sys/attr.h>
int main(int argc, char **argv) {
    int fd = open(argv[1], O_RDONLY | O_DIRECTORY);
    struct attrlist al; memset(&al, 0, sizeof al);
    al.bitmapcount = ATTR_BIT_MAP_COUNT;
    al.commonattr = ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_ERROR | ATTR_CMN_NAME
                  | ATTR_CMN_DEVID | ATTR_CMN_OBJTYPE | ATTR_CMN_MODTIME
                  | ATTR_CMN_FILEID;
    al.dirattr = ATTR_DIR_ALLOCSIZE; al.fileattr = ATTR_FILE_ALLOCSIZE;
    char buf[131072];
    int n = getattrlistbulk(fd, &al, buf, sizeof buf, FSOPT_PACK_INVAL_ATTRS | FSOPT_NOFOLLOW);
    char *p = buf;
    for (int i = 0; i < n; i++) {
        unsigned len = *(unsigned *)p;
        printf("group %d len=%u\n", i, len);
        for (unsigned j = 0; j < len && j < 96; j++)
            printf("%02x%s", (unsigned char)p[j], j % 16 == 15 ? "\n" : " ");
        printf("\n"); p += len;
    }
}
```

Create a directory with a **known-size** file (`dd if=/dev/zero of=x bs=1024
count=100` → expect 102400 at the alloc offset), a 1-byte file (expect one 4096
block — proof you are reading allocated, not apparent, size), a subdirectory and a
symlink. Verify the layout holds for all four.

Then run `make verify-macos` **and** `make test`.

## 7. Performance notes

Current: full sweep ~6.7 s for 1.72M entries; discovery 433 ms; measure 85% of
wall clock. Against `du -sk` on identical input: **4.925 s → 1.085 s**.

The walk is **syscall-bound, and the syscall count is set by directory shape**.
Reading any directory costs ~4 syscalls regardless of how few entries it holds,
and real `node_modules` averages ~5 entries per directory. This is why published
figures like "785k entries/s" do not transfer — that benchmark used 100 files per
directory, i.e. 20× fewer syscalls per entry.

Three optimizations were tried and judged on measurements:

| Change | Result | Kept? |
|---|---|---|
| Skip per-entry name allocation in the measure pass | 250k → 267k entries/s | yes |
| Inline single-child recursion (one less spawn per chain level) | no wall-clock change | yes, but only for bounded task memory — the comment says so |
| Thread-pool oversubscription (32 threads) | +7% isolated, **−18% in the full pipeline** | **reverted** |

The last one is the lesson: measure the pipeline, not the part. Concurrent
`measure()` scopes contend on the shared inode map in a way the microbenchmark
could not show.

## 8. Traps

- **`plutil -extract KEY json file.plist` rewrites the file in place.** Always
  `-o -` to read. This destroyed a built `Info.plist` once; the symptom was an
  `Info.plist` containing only the extracted value.
- **Host and container `node_modules` must stay separate.** The compose `ui`
  service masks `app/node_modules` with an anonymous volume. Without it the
  container installs Linux binaries where host pnpm expects darwin ones and the
  host build dies with `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY`. Recovery:
  `rm -rf app/node_modules && (cd app && CI=true pnpm install)`.
- **The Tauri CLI searches *subfolders* of the CWD** for `tauri.conf.json`, so it
  must run from `app/`, not the repo root.
- **`cargo bench` runs with CWD at the package root**, not the workspace root.
- **The DMG and the `.app` in `bundle/macos/` are different artifacts.** `tauri
  build` produces both in one pass; anything you do to the sibling afterwards
  never reaches the DMG. Signing after the fact shipped a DMG containing a
  linker-signed binary that recipients saw as *"Woosh is damaged and can't be
  opened"* — and quarantine removal does not fix an invalid signature. Sign via
  `APPLE_SIGNING_IDENTITY` during the build; `make dmg` verifies the shipped copy.
- **Icons are generated, not committed art.** `python3 scripts/make-icon.py
  app/src-tauri/icons` rebuilds every size plus `.icns` via `iconutil`.

## 9. Releasing

See the `woosh-ship` skill, or in short:

```bash
make dmg       # ad-hoc signed, shareable with a one-time xattr step
make release   # Developer ID + notarized + stapled (needs APPLE_* env vars)
```

`make doctor` reports which signing credentials are present.
