# Woosh — Implementation Plan

Native macOS developer-storage reclaimer. Tauri v2 shell, pure-Rust engine.
Ground truth for domain logic: `/Users/forthtech/Desktop/storage-checker` (Python prototype).

---

## 0. Two findings that shape everything

### 0.1 Docker cannot produce the macOS `.app`. (Hard blocker on Deliverable 1 as literally worded.)

A macOS Tauri bundle needs three things that do not exist inside a Linux container:

| Requirement | Why Linux Docker can't |
|---|---|
| `MacOSX.sdk` + `WebKit.framework` | Tauri's webview layer (`wry`) links `WKWebView` on macOS. Not redistributable, not present. |
| `codesign` / `lipo` / `actool` | Apple-only Mach-O + bundle tooling. |
| `.app` bundle assembly | `cargo tauri build` shells out to the above. |

Docker Desktop here runs `linux/aarch64`. A Tauri build in that container links `webkit2gtk`
and produces a Linux ELF binary that cannot run on macOS.

**Resolution — the containment boundary moves, the rule survives.** The rule's intent is
*don't install toolchains on the host Mac*. Nothing needs installing: the host already has
`rustc 1.94.1` / `cargo 1.94.1`, `node v22.22.2`, `pnpm 10.27.0`, `bun 1.3.14` (verified).
So:

- **Docker is the authoritative build + verification environment.** Dependency resolution,
  `fmt`, `clippy -D warnings`, the whole unit/integration test suite, full-workspace
  `cargo check` (incl. the Tauri shell, via `libwebkit2gtk-4.1-dev` in the image), the
  frontend bundle, and the benchmark harness. All of it. CI-ready.
- **The host runs exactly one command**: `cargo tauri build` — the final link + bundle step
  that is physically impossible elsewhere. Zero new installs.

This is the only split that satisfies both "no host pollution" and "ships a working macOS app."

**✅ DECIDED (confirmed by owner).** Docker environments exist *to build and verify*. The shipped
artifact is a real macOS `.app` that runs on this Mac and on other people's Macs when shared —
which promotes distribution from an afterthought to a hard requirement. See §8.

### 0.2 `getattrlistbulk(2)` is the whole performance story.

The 500GB/1.5s target is not reachable with `walkdir`, `jwalk`, or `du`. It is reachable with
the macOS bulk-metadata syscall, which returns name + type + **allocated size** + inode for a
whole batch of directory entries in one call:

- ~1.9× faster than `readdir + lstat` warm; the reference `dumac` implementation clocks
  **409,500 files in 521 ms** (≈785k entries/sec) with 128KB attrbufs and 64-way concurrency.
- `ATTR_FILE_ALLOCSIZE` means **sizing is free** — no second pass, no `du` subprocess.
  It also reports what you *actually reclaim*, which silently fixes the sparse-file problem
  (`Docker.raw` reports 64GB apparent / ~12GB real; the prototype needed a special case, we don't).

Requested attribute set: `ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_NAME | ATTR_CMN_ERROR |
ATTR_CMN_OBJTYPE | ATTR_CMN_FILEID`, plus `ATTR_FILE_ALLOCSIZE` in the file attrgroup.

---

## 1. Performance — measured, and the 1.5s target assessed

**Measured on this machine (M-series, 4P+4E, 460 GB APFS, warm cache):**

| | Result |
|---|---|
| Full sweep | **6.7 s** · 1,719,607 entries · **81.6 GB reclaimable across 95 items** |
| — discovery phase | **433 ms** |
| — measure phase | **5.7 s** (85% of wall clock) |
| — probe phase | 558 ms |
| Walker, normal tree | ~500k entries/s |
| Walker, `node_modules` | ~300k entries/s |
| **vs `du -sk`, identical input** | **4.925 s → 1.085 s · 4.5× faster** |

### The 1.5s target was not met for a full sweep. Here is exactly why.

Discovery — *finding* what is reclaimable — lands at **433 ms**, comfortably inside
budget. The cost is **measuring**, and that cost is structural:

1. **There is no "size of this directory" call.** Recursive size requires visiting
   every entry. No API avoids this.
2. **We are syscall-bound, not CPU-bound**, and the syscall count is set by
   *directory shape*, not by our code. Reading one directory costs
   `open` + `getattrlistbulk` + `getattrlistbulk`(returns 0) + `close` ≈ 4 syscalls,
   **regardless of how few entries it holds**. Real `node_modules` averages ~5
   entries per directory, so ~0.8 syscalls per entry.
3. **That is why the reference 785k entries/s is not reachable here.** That figure
   comes from a synthetic fixture of 4,095 directories holding 409,500 files —
   **100 files per directory**, i.e. 0.04 syscalls/entry. Twenty times fewer
   syscalls per entry than real dependency trees. The number is real; it does not
   transfer to this shape of data.

**Two optimizations were implemented, measured, and judged on the result rather
than on the theory:**

- *Skip per-entry name allocation in the measure pass* (names are never read there).
  1.7M fewer heap allocations. Effect: 250k → 267k entries/s. Kept — free and correct.
- *Inline single-child recursion* to remove one task spawn per level of the long
  single-child chains dependency trees are made of. Effect on wall clock: **none**.
  Kept only because it bounds queued-task memory on deep trees, and the comment in
  the code says so rather than claiming a speedup.
- *Thread-pool oversubscription* (blocked syscalls hold no core): +7% in isolation
  at 32 threads, but **a 18% regression in the full pipeline** — concurrent
  `measure()` scopes contend on the shared inode map. **Reverted.** A microbenchmark
  that did not predict system behaviour.

**What this means in practice.** The honest claim is not "500 GB in 1.5 s"; it is
*4.5× faster than the `du`-based approach it replaces, with hardlink
de-duplication `du` cannot do, and first findings on screen in under half a
second.* The telemetry feed exists precisely so the measure phase reads as
progress rather than as a hang.

### Hardlink / clone de-duplication — a correctness bug we fix

`pnpm` is a content-addressable store that **hardlinks** packages into every
project's `node_modules` from `~/Library/pnpm/store`. APFS also clones files
copy-on-write. Summing allocated size naively therefore reports space that
deleting will not give back — and `du -sk` de-duplicates only *within a single
invocation*, so the prototype, which calls it once per candidate, over-reports.

[`InodeSet`] charges each `(dev, ino)` exactly once per scan. Findings carry
`exclusive` (what deleting really returns) and `shared` (freed only when the last
referrer goes) separately. Confirmed live: **341 MB of the 81.6 GB found on this
machine is shared**, and is reported as such rather than counted twice.

## 2. Repository layout

```
Garabage-Cleaner-Thingy/
├── Makefile                      # single entry point; everything routes through Docker
├── docker/
│   ├── Dockerfile.engine         # rust:1.94 + webkit2gtk-4.1-dev → check/clippy/test/bench
│   ├── Dockerfile.ui             # node:22 + pnpm → frontend bundle
│   └── docker-compose.yml        # engine-check | engine-test | bench | ui-build
├── .dockerignore
├── Cargo.toml                    # workspace: engine + app/src-tauri
├── docs/
│   └── IMPLEMENTATION_PLAN.md    # this file
├── engine/                       # ← pure Rust. NO tauri dep. Compiles & tests on Linux.
│   ├── Cargo.toml
│   ├── benches/sweep.rs
│   └── src/
│       ├── lib.rs                # Engine facade: sweep(), reap(), Config
│       ├── fastwalk/
│       │   ├── mod.rs            #   Walker trait, Entry { name, kind, alloc, ino, dev, mtime }
│       │   ├── macos.rs          #   #[cfg(macos)]  getattrlistbulk + openat + rayon
│       │   └── portable.rs       #   #[cfg(not(macos))] jwalk fallback — same trait
│       ├── paths.rs              # ← paths.py
│       ├── safety.rs             # ← cleaner.py boundaries, hardened (§5)
│       ├── reaper.rs             # ← cleaner.py execution, via `trash` crate
│       ├── telemetry.rs          # frame coalescer + creative log voice (§6)
│       ├── dedup.rs              # sharded inode set
│       ├── sysinfo.rs            # ← system_info.py (statvfs)
│       └── domains/
│           ├── mod.rs            # Domain trait + registry
│           ├── venv.rs           # ← venv_manager.py
│           ├── node.rs           # ← node_modules_manager.py
│           ├── global_python.rs  # ← global_python_manager.py
│           ├── models.rs         # ← models_manager.py
│           ├── docker.rs         # ← docker_manager.py  (UDS, not subprocess)
│           ├── caches.rs         # ← cache_manager.py + §4 expansion
│           └── artifacts.rs      # ← cache_manager.scan_project_artifacts
└── app/
    ├── src-tauri/                # thin shell: commands, event bridge, window chrome
    │   ├── Cargo.toml
    │   ├── tauri.conf.json
    │   └── src/{main.rs, cmds.rs, bridge.rs}
    └── ui/                       # Svelte 5 + Vite + TS, hand-written CSS, no UI lib
        ├── package.json
        └── src/{App.svelte, lib/, views/}
```

**Why the engine is its own crate with no Tauri dependency:** it is the only reason the test
suite can run in Docker at all. The Tauri shell stays thin enough that "untestable on Linux"
costs nothing — it is command signatures and event plumbing, and even *that* gets a
full-workspace `cargo check` in the container.

---

## 3. Domain port map

Fidelity rule: behaviour comes from the prototype. Where I deviate, it is listed as a **Δ** and
justified. All detection is structural, never name-based, matching the prototype's design intent.

### 3.1 `paths.rs` ← `paths.py`
- `HOST_ROOT` env + `/host/Users` auto-detect; `TARGET_USER`; multi-user `/Users/*` discovery
  skipping dotfiles + `Shared`; `~` → discovered host home (not container `/root`).
- `get_default_scan_roots()`: `Desktop, Documents, Downloads, Projects, code, dev, workspace, work`
  + non-dot top-level home dirs, minus the `skip_home_folders` set. Dedup by canonical path.
- **Δ** `resolve_path` canonicalizes and resolves symlinks (feeds §5's hardened check).

### 3.2 `venv.rs` ← `venv_manager.py`
- Name-agnostic detection, all four markers, in prototype priority order:
  `pyvenv.cfg` → `bin/activate` + `bin/python{,3}` → `Scripts/activate` + `Scripts/python.exe` → `conda-meta/`.
- **Zero-subprocess** `site-packages/*.dist-info/METADATA` parse — the prototype's `<2ms` claim is
  the design target. Streamed line scan, early-break once `Name:`+`Version:` seen, with the
  `foo-1.0.0.dist-info` folder-name fallback. Prototype's `pip freeze` fallback is **dropped (Δ)**:
  it is a subprocess, and the folder-name fallback already covers the case it existed for.
- `pyvenv.cfg` version parse (`version` / `version_info`). **Δ** no `python --version` subprocess
  fallback — report `Unknown`, which is honest and free.
- `req-inplace.txt` generated into the **project root** (venv's parent) with the full header block
  and 3-line recreation recipe, byte-identical in shape to the prototype. Written **before**
  deletion; deletion aborts if the write fails. This is the safety contract and it is not optional.

### 3.3 `node.rs` ← `node_modules_manager.py`
- Lockfile precedence exactly as prototype: `pnpm-lock.yaml` → `bun.lockb`/`bun.lock` → `yarn.lock`
  → `package-lock.json` → default `npm`.
- Reinstall command per manager; `>1MB` floor; `days_ago` from mtime.
- **Δ** Yarn Berry split: `.yarnrc.yml` with `nodeLinker: pnp` means there *is* no `node_modules`
  to clean — detect and skip rather than mis-report.
- **Δ** pnpm `node_modules` are hardlink farms → §1 dedup applies, and the finding is tagged
  "restored from local store (fast)" vs npm's "re-downloads from network (slow)". That changes
  the user's decision, so it belongs in the UI.

### 3.4 `global_python.rs` ← `global_python_manager.py`
- Discovery across `PATH`, `/opt/homebrew/bin/python3.{14…8}`, `/Library/Frameworks/Python.framework/Versions/*`,
  `/usr/local/bin/python3*`, `/usr/bin/python3`, `~/.pyenv/versions/*`. Suffix filter
  (`-config`, `-intel64`, `m`, `w`). Dedup by realpath. Source classification
  (Homebrew / Python.org Framework / Apple System (SIP) / pyenv / Other).
- `ESSENTIAL_GLOBAL_PACKAGES` = `{pip, setuptools, wheel, certifi, packaging, virtualenv, pipenv, uv, poetry}` — ported verbatim.
- SIP guard on `/usr/bin/python3` — refuse to modify, ported verbatim.
- `req-global-backup.txt` before any prune, ported.
- **Δ** package listing drops the `pip list --format=json` subprocess in favour of the same
  `dist-info/METADATA` reader used by §3.2, resolving `site-packages` by globbing the binary's
  prefix (`../lib/python3.X/site-packages`). One `sysconfig` subprocess only if the layout is
  unrecognised.
- **Kept as subprocess, deliberately:** `pip uninstall`. It consumes `RECORD` files and removes
  console scripts; hand-deleting `dist-info` corrupts the installation. Correctness beats purity here.

### 3.5 `models.rs` ← `models_manager.py`
- HF Hub `models--<org>--<repo>` → `org/repo`, snapshot count. Ollama blobs. LM Studio
  `<publisher>/<model>` + extensions (>50MB floor). All nine other ML caches with their floors.
- **Δ** Ollama via its **HTTP API** (`GET /api/tags`, `DELETE /api/delete` on `:11434`) instead of
  parsing `ollama list` stdout and shelling `ollama rm`. Removes the fragile column-splitting
  size parser (`parts[2] + parts[3]`) and gives exact byte counts instead of reconstructing them
  from `"2.2 GB"`. Manifest-walk fallback retained for when the daemon is down.
- **Δ** HF `blobs/` are hardlinked from `snapshots/` → §1 dedup prevents double-counting.

### 3.6 `docker.rs` ← `docker_manager.py`
- **Δ** Talks to `/var/run/docker.sock` directly (`GET /v1.44/system/df`) — the prompt asks for
  socket inspection, and it is both faster and immune to the `_parse_docker_size` string-parsing
  path (`"1.5GB (50%)"` → float). Exact `i64` bytes from JSON. Prune via
  `POST /{images,containers,volumes,build}/prune`.
- `Docker.raw` across all three candidate paths, allocated vs apparent → sparse detection
  (now free, per §0.2).
- Daemon-down path returns the VM disk reading alone, as the prototype does.

### 3.7 `caches.rs` ← `cache_manager.py`
All 22 prototype definitions ported with their `safe_to_clean` flags, descriptions, `cleanup_cmd`
hints, and the `>512KB` floor — including `~/.android` staying `safe_to_clean: false`, and Trash.

### 3.8 `artifacts.rs`
`node_modules` (>5MB) and Rust `target/` **gated on a sibling `Cargo.toml`** — ported exactly;
that gate is what stops it eating unrelated `target` folders.

---

## 4. Expansion targets (researched)

New cleaners beyond the prototype. Each is structurally detected, size-gated, and ships a
restore hint.

| Domain | Path | Notes |
|---|---|---|
| Cargo git checkouts | `~/.cargo/git/{checkouts,db}` | Prototype covers `registry/cache` only. |
| Stale `target/` sweep | project-local | Rank by mtime age; `cargo clean` hint. |
| Swift PM | `~/Library/Caches/org.swift.swiftpm`, `~/Library/Developer/Xcode/DerivedData/*/SourcePackages` | |
| CocoaPods | `~/Library/Caches/CocoaPods`, `~/.cocoapods/repos` | Spec repos are multi-GB git clones. |
| Xcode Archives / DeviceSupport | already in prototype | per-iOS-version breakdown added. |
| **CoreSimulator Devices** | `~/Library/Developer/CoreSimulator/Devices` | **Biggest single win: 20–60GB.** Prototype only cleans `Caches`. Parse `device.plist` for runtime + availability; target unavailable/shutdown devices. |
| Simulator runtimes | `~/Library/Developer/CoreSimulator/Profiles/Runtimes` | 3–8GB each. |
| JetBrains | `~/Library/Caches/JetBrains/*` | Split per-IDE; detect orphaned versions. |
| Gradle daemon logs | `~/.gradle/daemon` | Research flagged this reaching ~15GB with 1–5GB single logs. |
| Bun | `~/.bun/install/cache` | ~9GB after a year of use, per research. |
| Deno | `$DENO_DIR` or `~/Library/Caches/deno` | |
| Yarn Berry | `~/.yarn/berry/cache`, `.yarn/cache` | Zip-per-package. |
| TurboRepo | `~/.turbo`, `node_modules/.cache/turbo` | |
| Triton | `~/.triton/cache` | |
| Go | `$GOCACHE` or `~/Library/Caches/go-build`, `~/go/pkg/mod` | |
| Maven / Gradle deps | `~/.m2/repository`, `~/.gradle/caches/modules-2` | |
| Nix | `/nix/store` | **Detect + report only, never delete** — needs `nix-collect-garbage`. |
| Electron / node-gyp | `~/Library/Caches/electron{,-builder}`, `~/.node-gyp` | |
| Playwright / Puppeteer | `~/Library/Caches/{ms-playwright,puppeteer}` | Full browser builds, ~500MB each. |
| Bazel | `~/.cache/bazel` | Can hit tens of GB. |
| Conda / uv | `~/.conda/pkgs`, `~/Library/Caches/uv` | |

---

## 5. Safety model

Ported from `cleaner.py`, then hardened. The prototype's four rules stay:
`PROTECTED_PATHS` exact-match; self-preservation; no user-home root; allowlist prefixes
(`/Users/`, `/Volumes/`, `/tmp/`, + `/host/` variants); minimum component count.

Four additions, each closing a real hole:

1. **Canonicalize before checking.** The prototype uses `os.path.abspath`, which does *not*
   resolve symlinks — so a symlink at `~/evil → /System/Library` passes every check and gets
   deleted. `safety.rs` resolves the full realpath first, and refuses to follow a symlink out of
   the permitted tree.
2. **Re-verify at delete time (TOCTOU).** Scan and reap are separated by however long the user
   stares at the UI. The path is re-canonicalized and re-checked immediately before the
   destructive call, not just when it was discovered.
3. **Mount-boundary check.** Compare `st_dev` against the claimed root; refuse surprise crossings
   into network or external volumes.
4. **Dry-run mode**, wired through `reaper.rs` and surfaced as a UI toggle. Every receipt the
   real run would produce, produced, with nothing touched.

Deletion goes through the **`trash` crate** (`trash = "5.2"`, `NSFileManager trashItemAtURL:`) —
real Finder Trash with working Put Back, no `osascript` subprocess, no AppleScript permission
prompt. Permanent delete is opt-in and never a silent fallback: the prototype's
`move_to_macos_trash` → `_permanent_delete` fallback path means a Trash failure escalates to
irreversible deletion without asking. **That fallback is removed (Δ)** — a Trash failure
surfaces as a failed receipt, and permanent deletion happens only when the user explicitly
selected that mode.

Audit log: append-only JSONL (prototype keeps the last 50 in a rewritten JSON array; JSONL
appends atomically and cannot lose history to a crash mid-write).

---

## 6. Telemetry + IPC

**The throughput problem:** at ~800k entries/sec, one event per file would emit 800k
IPC messages/sec and destroy both the bridge and the renderer. Event volume must be decoupled
from scan volume.

```
rayon workers ──► atomics (entries, bytes, dirs)          ─┐
             └──► bounded crossbeam channel (discoveries)  ─┤
                                                            ├─► coalescer @60Hz ──► emit("telemetry", Frame)
                                                            │     (16.6ms tick)
             └──► SeqCst path sampler (3–5 live paths)     ─┘
```

- **Counters are atomics**, never events — the coalescer reads them each tick. Scan speed is
  therefore completely independent of UI speed.
- **Discoveries** (actual findings) are rare — hundreds, not millions — so they ride a bounded
  channel and appear individually.
- **Live paths** are sampled, not queued: workers store into a small ring, the coalescer reads
  whatever is current. Backpressure is impossible by construction.
- One frame type, `TelemetryFrame { seq, elapsed_us, entries, bytes, rate, paths[], discoveries[], lines[] }`,
  emitted on `"telemetry"`; `"reap"` carries the same shape during deletion.

**Creative log voice.** Templates with real interpolated data — the wit never costs information.
Lines are generated in the engine (it has the facts) and carry a `kind` the UI colour-codes.

```
[00.0412s]  ⟫ hangar doors open · 14 roots · 10 threads warm
[00.0631s]  ◈ FOUND  ~/dev/kraken/node_modules   412.8 MB   pnpm   ~ 94% hardlinked to store
[00.1240s]  ⟳ 318,442 entries · 1.41 GB · 772k/s · descending ~/Library/Developer/CoreSimulator
[00.2888s]  ◈ FOUND  iPhone 14 Pro (iOS 16.4, unavailable)   7.2 GB   ← orphaned runtime
[00.9104s]  ✓ sweep complete · 581,203 entries · 41.3 GB reclaimable · 904ms
[—— reap ——]
[00.0008s]  ⊘ req-inplace.txt written · 61 packages pinned · ~/dev/scraper/
[00.0121s]  ✦ WOOSH  ~/dev/scraper/.venv → Trash   1.8 GB   restorable
```

---

## 7. Verification

| Gate | Where | What |
|---|---|---|
| `cargo fmt --check` | Docker | |
| `cargo clippy -D warnings` | Docker | workspace-wide |
| `cargo test` | Docker | engine unit + integration on fixtures |
| `cargo check` (workspace) | Docker | catches Tauri command-signature breakage on Linux |
| `cargo bench` | Docker | 400k-file synthetic fixture, gates the 1.5s budget |
| `tsc --noEmit` + `vite build` | Docker | |
| `cargo tauri build` | **host** | final link + bundle only (§0.1) |

**Fixture generator** (`scripts/make-fixture.sh`) synthesizes a tree with known-answer sizes:
nested venvs under odd names (`mynenv`, `custom-env`), hardlink farms to prove dedup, sparse
files to prove `allocsize`, symlink escape attempts to prove §5.1, and a `target/` with no
`Cargo.toml` to prove the §3.8 gate. Every safety rule gets a test that asserts a *refusal*.

The macOS `getattrlistbulk` path cannot execute in Linux Docker — so `fastwalk/portable.rs` and
`fastwalk/macos.rs` are tested against **the same fixture with the same asserted answers**, and
a host-side `make verify-macos` runs the differential. Any divergence is a bug in one of them.

---

## 8. Distribution — the app runs on other people's Macs

Sharing a `.app` is where unsigned macOS builds fall apart, so this is designed in, not bolted on.

### 8.1 Gatekeeper reality

| Build | What the recipient sees |
|---|---|
| Ad-hoc signed (`-`) | `"Woosh" is damaged and can't be opened.` — Gatekeeper kills it on download. The binary is fine; the quarantine xattr is the problem. |
| Developer ID signed, not notarized | `"Woosh" can't be opened because Apple cannot check it for malicious software.` Right-click → Open works, but reads as malware. |
| **Developer ID + notarized + stapled** | Opens normally. The only genuinely shareable option. |

Tiering, so progress is never blocked on an Apple account:

- **Tier 0 — local dev (now).** Ad-hoc signature. Runs on this Mac immediately.
- **Tier 1 — informal sharing (no account needed).** Ship a `.dmg` plus a one-line unquarantine
  (`xattr -dr com.apple.quarantine /Applications/Woosh.app`), documented in the README. Honest,
  and it works.
- **Tier 2 — real distribution.** Developer ID Application cert + `notarytool` + `stapler`.
  Wired into the Makefile as `make release`, reading
  `APPLE_SIGNING_IDENTITY` / `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` from the env so no
  secret ever lands in the repo. Inert until those are set — zero friction before then.

### 8.2 Apple Silicon only (owner decision)

Single target: **`aarch64-apple-darwin`**. No `universal-apple-darwin`, no `lipo` fusing, no
x86_64 slice.

This is a real simplification, not just a smaller binary — it deletes the dual-slice fixture
differential that a fat binary would have forced on §0.2, so the `getattrlistbulk` attrbuf reader
only ever has to be correct for one arch's `#[repr(C)]` packing and alignment.

Consequence to be explicit about: an arm64-only `.app` on an Intel Mac does not show a friendly
error — it fails to launch, or Finder reports it as damaged. So the bundle declares
`LSArchitecturePriority: [arm64]` and `LSMinimumSystemVersion: 11.0`, and the DMG + README state
**"Apple Silicon (M1 or later) required"** plainly. Dev and release builds are now the same target,
which also means the thing you test is byte-for-byte the thing you ship.

### 8.3 Permissions — a designed first-run flow, not an error path

The app needs broad filesystem reach, and recipients must grant it. macOS splits this into two
mechanisms that behave *completely differently*, and conflating them is the usual way this gets
shipped broken:

**(a) Desktop / Documents / Downloads — automatic prompts, and they need Info.plist strings.**
These three are the engine's primary scan roots (§3.1), and macOS shows a real consent dialog on
first access. The reason string shown to the user comes from Info.plist, so these are
**mandatory**, not optional polish:

```
NSDesktopFolderUsageDescription   = "Woosh scans your Desktop for project folders holding
                                     reclaimable build artifacts and virtual environments."
NSDocumentsFolderUsageDescription = "…Documents…"
NSDownloadsFolderUsageDescription = "…Downloads…"
```

Omit them and the recipient gets a blank-reason prompt or a silent denial — and then an empty
scan they can't explain.

**(b) Full Disk Access — cannot be requested programmatically.** There is no API. The only route
is the user adding the app themselves in
*System Settings → Privacy & Security → Full Disk Access*. Needed for other users' homes and
`~/Library/Containers/com.docker.docker`. So the app:

1. **Probes** at startup — attempts a cheap read of a known FDA-gated path and records the result.
2. **Deep-links** to the exact pane via
   `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles`
   rather than telling the user to go hunting.
3. **Degrades gracefully.** FDA is an *enhancement*, never a gate. Without it the sweep still
   runs across everything it can reach, and blocked roots surface as
   *"N paths need Full Disk Access · +~12 GB unscanned"* with the fix one click away.

`EPERM`/`EACCES` is a first-class scan outcome throughout the engine. The prototype's bare
`except Exception: pass` swallows exactly this, so a 40GB simulator folder just silently never
appears and the user has no idea why.

**The wrinkle worth knowing about up front:** TCC identifies an app by bundle ID **plus code
signature**. For an ad-hoc or unsigned build, that's the `cdhash` — which changes on *every
rebuild*. So each new build you share is, to macOS, a different app: Full Disk Access grants
reset and recipients must re-add it after every update. A stable signing identity (Tier 2, §8.1)
is what actually fixes this — until then the README says so honestly instead of letting people
think the app broke.

**Entitlements:** App Sandbox **off** (a cleaner reaching `/var/run/docker.sock` and `~/.cargo`
cannot live in a container — this correctly rules out Mac App Store distribution). Hardened
Runtime **on** with `com.apple.security.cs.allow-jit` for WebKit's JS engine.
`NSAppleEventsUsageDescription` is **not** needed: the `trash` crate uses `NSFileManager`, so we
never drive Finder via AppleScript (§5).

### 8.4 Bundle identity

`com.woosh.cleaner`, `productName: Woosh`, versioned from the workspace `Cargo.toml`. DMG with a
styled background and an `/Applications` drop target. Updater is explicitly out of scope for v1 —
a self-updating deletion tool is a trust liability I'd rather not ship on day one.

## 9. Milestones

| | Deliverable | Exit criterion |
|---|---|---|
| **M0** | Docker images, workspace, Makefile, `.dockerignore` | `make check` green in container |
| **M1** | `fastwalk` both impls, `dedup`, `sysinfo`, bench harness | differential passes; bench prints real entries/sec |
| **M2** | `paths` + all 7 domains + §4 expansion | parity diff vs prototype JSON on this Mac |
| **M3** | `safety` + `reaper` + audit log | every hardened rule has a passing refusal test |
| **M4** | Tauri shell, commands, coalescer, telemetry | frames stream at 60Hz under full scan load |
| **M5** | Svelte UI, all views | end-to-end sweep → select → dry-run → reap |
| **M6** | Host bundle, README with measured numbers | `.app` launches, scans, reaps, restores from Trash |
| **M7** | arm64 DMG, first-run permission flow, `make release` signing hooks | DMG installs on a second Apple Silicon Mac; FDA probe + deep-link + graceful degradation all verified |

Parity check at M2 is a real gate: run the Python prototype and the Rust engine on this
machine, diff the findings, and account for every difference as either a listed **Δ** or a bug.

---

## 10. Decisions

**Resolved:** §0.1 — Docker owns all building and verification; the host runs one pre-existing
`cargo tauri build` to produce the shippable macOS `.app`. Distribution requirements in §8.

**Decided, proceeding unless you object:**
- **Svelte 5 + Vite + TypeScript**, hand-written CSS, no component library. Compiles to near-zero
  runtime, and runes handle a 60Hz telemetry stream with a virtualized list cleanly.
- **Sweep is the default mode**, Deep is a toggle (§1).
- **`aarch64-apple-darwin` only**, shipped as a DMG (§8.2).
- **Global Python pruning ships behind a confirm dialog**, not one-click — it is the one
  destructive action that touches a shared interpreter rather than a disposable artifact.
- **`pip uninstall` stays a subprocess** (§3.4); everything else is zero-subprocess.
