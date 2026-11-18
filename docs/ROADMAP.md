# Woosh — Roadmap (written 2026-11-02)

> **Superseded for open work by `docs/PLAN_AIO.md`.** Some items below are marked done but were
> not: the Search UI never calls the file index (Phase 6), Phase 4 P4 (trigram, FSEvents sync,
> memmap) was never built, and `make ci` fails to compile the shell. See PLAN_AIO.md §0.

The next set of work, based on reading the whole engine, the Tauri shell and the UI.
Each item names the file it touches and how to know it's done. Hard rules in
`CLAUDE.md` still apply to all of it, especially `safety.rs`, the reaper invariants
and the `fastwalk/macos.rs` offsets.

Suggested order: **Phase 0 → Phase 1 → P1/P2 from Phase 4 → Phase 3 basics → Phase 2 in batches.**

---

## Phase 0 — Bugs found while reading (all completed 2026-11-02)

| # | Where | Bug | Fix / done when | Status |
|---|---|---|---|---|
| B1 | `app/src-tauri/src/lib.rs` `scan` | `state.scanning` is set to `true` and only reset on the happy path. If `spawn_blocking` errors or the scan panics, the `?` returns early and **every later Sweep fails with "a scan is already running" until the app is relaunched.** | Reset it with an RAII drop guard. Done when an injected error leaves the next scan working. | ✓ Done (`ScanGuard(Arc<Mutex<bool>>)`) |
| B2 | `engine/src/lib.rs` | `scan()` and `scan_paths()` duplicate the measure + finding mapping. The integration tests exercise `scan_paths`, so the code that actually ships (`scan`) is untested and can drift. | Extract one `measure_claims(claims, stats, inodes, on_found)` used by both. | ✓ Done (extracted `measure_claims`) |
| B3 | `App.svelte` | Selection survives filtering. With a sidebar filter on, Woosh acts on hidden items too, and the action bar doesn't say so. | Show "N selected · M hidden by filter", or scope the action to the visible list. Pick one and make it obvious. | ✓ Done (`hiddenSelectedCount` displayed) |
| B4 | UI / honesty | After "Move to Trash" the UI says *reclaimed X GB*, but **macOS frees nothing until the Trash is emptied**. The disk bar doesn't move, which looks like a bug. | After a trash reap, show "X GB is in the Trash — empty it to free the space". Woosh must not empty the Trash itself without an explicit permanent-delete confirmation. | ✓ Done (honest messaging in `App.svelte`) |
| B5 | `App.svelte` telemetry | `log = [...log, ...lines].slice(-600)` copies up to 600 objects at 60 Hz. | Use a fixed ring buffer. Measure frame time before and after. | ✓ Done (`RingBuffer<T>` with reactive version) |
| B6 | `Findings.svelte` | No list virtualization. Fine at about 100 rows. Large-file results (Phase 1) and Deep mode will produce thousands. | Window the rows (fixed row height already ~50 px). | ✓ Done (row windowing with overscan 8) |
| B7 | `engine/src/lib.rs` `detect` | Allocates a `String` for the dir name on every directory visited (`to_string_lossy().to_string()`). | Compare on `OsStr`/`Cow` without allocating. Keep it only if the **full pipeline** gets faster (see the trap in CLAUDE.md). | ✓ Done (`&OsStr == &str` zero heap alloc) |
| B8 | `lib.rs` `reap` | Resolves ids with a linear `find` per id, and removes reaped items with `Vec::contains`. O(n·m). | Use a `HashSet`/`HashMap`. Trivial, but it matters once large files bring hundreds of items. | ✓ Done (`HashMap<&str, &Finding>` & `HashSet`) |

---

## Phase 1 — Large and forgotten files (the headline request)

**Why Woosh misses them today:** `paths::SKIP_HOME_FOLDERS` excludes `Movies`, `Music` and
`Pictures` from the scan roots. Discovery is depth-limited to 5 and **only ever looks at
directories**: `sweep::walk` reads every file's `alloc` from the listing and then throws it away.
So a 12 GB `.mov` in `~/Movies` or a 10 GB `Xcode.xip` in `~/Downloads` is invisible.

### Design
1. **New domain `Domain::LargeFile`** ("Large files"), with a glyph and label in `api.ts`.
2. **Zero-cost capture during discovery.** Extend `sweep::discover` with a file visitor. Any
   `Kind::File` entry with `alloc >= threshold` becomes a candidate. The listing is already in
   hand, so this costs **no extra syscalls**. Count files toward the threshold by `alloc`
   (what you get back), not logical size. Sparse files like `Docker.raw` already show why.
3. **Media roots pass.** Walk `~/Movies`, `~/Downloads`, `~/Desktop`, `~/Documents`, `~/Music`,
   `~/Pictures` and `~/Library/Mobile Documents` (iCloud Drive, but skip evicted placeholders)
   with the same walker, size-gated and without a depth limit, but:
   - **Never descend into packages.** `.photoslibrary`, `.fcpbundle`, `.logicx`, `.band`, `.app`,
     `.musiclibrary`, `.tvlibrary`, `.imovielibrary` and `.utm` are reported as **one Advisory
     unit** with their total size ("Photos Library — 84 GB, manage it in Photos").
   - Respect `is_noise`, plus `node_modules` and the like, already claimed by other domains.
4. **Classify by extension** for facet chips and wording:
   video (`mp4 mov mkv avi m4v webm`), disk images (`dmg iso img sparseimage`),
   installers (`pkg xip ipsw`), archives (`zip tar gz tgz 7z rar xz zst`),
   VM disks (`vmdk qcow2 vdi raw` outside Docker), audio (`wav aif aiff flac`), other.
5. **Risk:** user files are **never Safe** and **never pre-selected**.
   - Installers, disk images and `.ipsw` in `~/Downloads` idle for more than 14 days → Caution,
     with the reason "installer — the app is already installed" when a matching `.app` exists in
     `/Applications`. Match `Xcode_16.xip` → `Xcode.app` with a small name heuristic.
   - Everything else → Caution: "Your file. Woosh moves it to the Trash; Put Back restores it."
6. **Trash only.** The reaper refuses `Mode::Permanent` for `Domain::LargeFile`, and the UI
   disables the option when one is selected. Add a refusal test, matching the style in `reaper.rs`.
7. **Safety:** confirm `safety::vet` accepts regular files. It is path-based and checks
   `symlink_metadata`. Add *refusal* tests for files inside protected roots and inside packages.
   Do not loosen any existing rule (hard rule 3).
8. **Per-row actions:** Reveal in Finder and Quick Look (`qlmanage -p`, or the opener plugin's
   reveal), since nobody deletes a video without seeing it first.
9. **Settings:** threshold (default 500 MB), plus extra roots.

### Done when
- The fixture (`scripts/make-fixture.sh`) gains sparse large files, a fake `.photoslibrary`
  package and a `.dmg`, plus known-answer tests in `engine/tests/fixture.rs`.
- The byte-level differential still passes on both backends (`make test`, `make verify-macos`).
- The whole pipeline is measured on this Mac: report scan time before and after. The media pass
  is likely to be the slowest new cost.

### Status (completed 2026-11-02)
- Added `Domain::LargeFile` ("Large files", `◈`) across engine and UI.
- Zero-cost capture in `sweep::walk` / `sweep::discover` when `alloc >= large_file_threshold` (default 500 MB).
- Media roots pass over `Movies`, `Music`, `Pictures`, `Library/Mobile Documents`, `Downloads`, `Desktop`, `Documents`.
- Package bundle isolation: `.photoslibrary`, `.app`, `.fcpbundle`, `.logicx`, `.band`, `.utm` claimed as one Advisory unit; internal files never traversed or reported individually.
- Classification by extension: Video, DiskImage, Installer, Archive, VmDisk, Audio, Other.
- Safety: `safety::vet` enforces regular files outside packages; reaper strictly refuses `Mode::Permanent` on `Domain::LargeFile` and refuses Advisory items; comprehensive unit and fixture refusal tests added.
- Actions: Reveal in Finder and Quick Look (`qlmanage -p`) available per row.
- Known-answer fixture tests (`large_files_and_packages_are_claimed` and `package_safety_refusal_at_vet_time`) pass in Docker and host macOS.

### Follow-on: duplicates (Phase 1b)
Group by `alloc` size → hash the first and last 64 KB → full BLAKE3 only on collisions. **Be
APFS-clone aware:** clones share blocks, so deleting one frees nothing. The existing
`(dev, ino)` dedup catches hardlinks but not clones. Check `ATTR_CMNEXT_CLONEID` /
`getattrlist` for the clone id, and treat equal clone ids as "not a duplicate". Keep the newest
copy by default, and never auto-select.

---

## Granular Docker Management & Electability (completed 2026-11-02)

Transitioned Docker reporting from coarse lumped totals (`docker:images`, `docker:containers`, `docker:volumes`) to granular, individually inspectable and selectable items with rich safety invariants and sub-category electability:

1. **Individual Resource Classification**:
   - **Containers**: Active (running) containers identified and protected as `Risk::Advisory` (never deleted while running). Stopped/exited containers classified as `Risk::Caution` with `SizeRw` (writable layer) and associated attached volumes reported.
   - **Volumes**: Volumes mapped across all containers. Volumes attached to active containers protected as `Risk::Advisory`. Dangling (unattached) or attached-to-stopped volumes classified as `Risk::Caution`.
   - **Images & Origin Differentiation**:
     - **Pullable Registry Images**: Images with valid remote repository digests (`@sha256:`). Displayed with `☁ PULLABLE` badge and `docker pull <tag>` restore hints. Safe to reap because they can be re-downloaded at any time.
     - **Images Built on Device**: Images built from local Dockerfiles/Compose without a remote digest. Displayed with `⚙ BUILT ON DEVICE` badge; warned that deleting requires local source code to recompile/rebuild.
     - **Untagged Build Stages**: Dangling `<none>:<none>` intermediate layers. Displayed with `◌ UNTAGGED` badge.
     - **In-use vs Unused**: Images referenced by running/existing containers are protected as `Risk::Advisory`.
   - **Build Cache**: Reclaimable vs total layers sized; marked `Risk::Safe` (rebuilt on next build).
   - **VM Disk**: `Docker.raw` virtual disk reported with sparse allocation; protected from direct deletion.
2. **Reaper Actions**:
   - Added granular Docker API actions: `delete_image` (`DELETE /images/{id}`), `delete_container` (`DELETE /containers/{id}?v=1`), `delete_volume` (`DELETE /volumes/{name}`).
   - Advisory items protected at reap time; dry-run touch-free preview verified.
3. **UI Sub-Navigation & Electability**:
   - Added Docker sub-navigation bar with sub-category tabs: `[All Docker]`, `[Images]`, `[Containers]`, `[Volumes]`, `[Build Cache]`.
   - Image origin sub-filtering chips: `[All]`, `[☁ Pullable]`, `[⚙ Built on Device]`, `[◌ Untagged]`.
   - 1-Click Multi-Select Actions: `Select All Reclaimable`, `Select Pullable Unused`, `Select Built on Device`, `Select Stopped Containers`, `Select Dangling Volumes`, `Select Build Cache`.
   - Expanded inspection cards detailing container state, attached volume chip tags, mountpoints, full hashes, and layer deduplication breakdown.

---

## Phase 2 — Space other cleaners commonly miss

Add each one with the `woosh-add-cleaner` skill: structural detection, size gate, restore hint,
and a refusal test where the item is risky. ★ = likely multi-GB on a dev Mac.

**Apple dev**
- ★ **CoreSimulator Devices**, `~/Library/Developer/CoreSimulator/Devices` (✓ Done 2026-11-02).
  Parses each `device.plist` using the pure-Rust `plist` crate without subprocesses. Cross-references
  installed runtimes in `~/Library/Developer/CoreSimulator/Profiles/Runtimes`, `/Library/...`, and
  `/Applications/Xcode.app/...`. Classifies unavailable runtimes as `Risk::Safe` with restore hint
  `xcrun simctl delete unavailable`, booted devices as `Risk::Advisory`, and shutdown devices as
  `Risk::Caution`.
- ★ **iOS device backups**, `~/Library/Application Support/MobileSync/Backup/*` (✓ Done 2026-11-02).
  Classified as `Risk::Caution`, requires Full Disk Access.
- ★ **iOS / iPadOS firmware**, `~/Library/iTunes/*Software Updates/*.ipsw` (✓ Done 2026-11-02).
  Safe; re-downloaded on demand.
- Xcode Previews `~/Library/Developer/Xcode/UserData/Previews`, `Xcode/Products`, and
  `DocumentationCache` (✓ Done 2026-11-02).

**Toolchain version managers.** Report the versions that are **not** the active default (✓ Done 2026-11-02):
- ★ `engine/src/domains/toolchains.rs`:
  - **Rustup** `~/.rustup/toolchains/*`: parses `~/.rustup/settings.toml`, detects default toolchain (e.g. `stable-aarch64-apple-darwin`), flags inactive versions (e.g. `nightly-aarch64-apple-darwin`) with `rustup toolchain install` restore hint.
  - **Gradle wrapper distributions** `~/.gradle/wrapper/dists/*`: compares version tuples numerically, preserves the newest version, flags older wrapper dists as `Risk::Safe`.
  - **NVM Node.js versions** `~/.nvm/versions/node/*`: parses `~/.nvm/alias/default`, preserves default alias (e.g. `18`), flags inactive major versions as `Risk::Caution` with `nvm install` restore hint.
  - **Pyenv Python versions** `~/.pyenv/versions/*`: parses `~/.pyenv/version`, flags inactive python versions with `pyenv install` restore hint.
- Homebrew old kegs (`Cellar/<f>/<ver>` that aren't linked). Offer `brew cleanup` as an action
  rather than deleting files.

**More caches:** `~/.pub-cache` (Dart/Flutter), `~/Library/Caches/pypoetry`, pipx, Composer,
RubyGems/Bundler, ccache/sccache, `~/Library/Caches/Cypress`, Prisma engines, Terraform plugin
cache, `~/.expo`, Metro/haste caches in `$TMPDIR`, `~/.cache/pre-commit`, Carthage, Mint.

**App caches.** These are huge and silent; never touch profiles, only cache dirs:
- ★ Slack, Discord, Teams, Spotify (`PersistentCache`), Zoom, Figma, Notion: their
  `Cache`, `Code Cache`, `GPUCache` and `Service Worker/CacheStorage` dirs.
- ★ Chrome, Edge, Brave and Arc HTTP caches; VS Code `CachedExtensionVSIXs` / `CachedData`.
- ★ **Sandboxed app caches**, `~/Library/Containers/*/Data/Library/Caches`. Aggregate into one
  finding with children.
- **If the owning app is running, mark it Caution or skip it.** Check with `sysctl` `KERN_PROC`
  rather than spawning `pgrep`, in keeping with the zero-subprocess goal.

**Creative / VMs.** Report only (Advisory) unless noted:
- ★ Adobe `Media Cache Files` (Safe), Unity `~/Library/Unity/cache`, Unreal `DerivedDataCache`.
- GarageBand/Logic sound libraries, Final Cut render files: Advisory, delete through the app.
- Colima/Lima (`~/.colima`, `~/.lima`), OrbStack, Podman machine, UTM, Parallels VM disks,
  Android system images per API level.

**System-level.** Report only, with the exact command to run:
- ★ Time Machine local snapshots (`tmutil listlocalsnapshots /`); core dumps in `/cores`;
  `~/Library/Logs`; Mail `Downloads`.
- Orphaned app leftovers (Application Support/Containers for bundle ids with no installed app).
  False positives are likely, so ship as Advisory first.
- Git repos with an oversized `.git`: Advisory, with `git gc` / `git lfs prune` as the hint.

---

## Phase 3 — Versatility and user-friendliness

- **Findings table basics (completed 2026-11-02):**
  - Sort by size (`bytes`), age (`days_idle`), name, or risk with clickable header indicators (▲ / ▼).
  - Text search filter matching name, detail, and path in real-time (with ⌘F shortcut).
  - Quick action: "Select all Safe" toggle button with counter pill.
  - Keyboard navigation: ↑ / ↓ row navigation, Space to toggle selection, Enter to expand detail, ⌘A to select all, Esc to clear search or close detail.
  - Virtualized row windowing with overscan keeps scrolling smooth across thousands of items.
- **UI Craft Refactor (Anti-AI-Slop, Raycast/Linear Pro Standard) (✓ Done 2026-11-02):**
  - Executed according to modern macOS desktop design systems (`Inter`/`SF Pro`, Raycast and Linear design tokens).
  - Upgraded font scale: 14px base body, 14.5px/15px crisp item titles, 12.5px paths/subtitles, 15.5px tabular bold numbers, 24px hero stats, 11px-12px badges/tags with comfortable breathing room.
  - Replaced cramped 236px sidebar with a 270px Pro Command Sidebar featuring a multi-segmented hero storage visualizer bar (`Free` vs `Used` vs `Reclaimable`), clear capacity readout, and categorized domain item capsules.
  - Stripped badge/pill confetti (`☁ PULLABLE`, `⚙ BUILT ON DEVICE`, `● ACTIVE`, `○ STOPPED`, `▤ DANGLING`, `◈ UNUSED`).
  - Unified macOS command bar with 12.5px segmented controls, search omnibox (`⌘F`), and tactile `⚡ Sweep Now` CTA button (`⌘R`).
  - Gorgeous Hero Idle state with radar emblem, fast sweep launcher, and key architecture spec pills.
  - Floating bottom action bar (62px) with 19px bold reclaim counter, deletion mode selector, and tactile Woosh button.
  - Row height upgraded to 56px with overscan windowing, 7px status dots, and 2-column developer inspector drawer with terminal snippet copy.
  - Near-black obsidian canvases (`#07080A`, `#0D1015`, `#131722`, `#1A202F`) with hairline borders (`rgba(255, 255, 255, 0.08)`) and inset highlights.
- **Stale projects view (✓ Done 2026-11-02):**
  - Group `node_modules` + `venv` + `target` by project root.
  - Extract idle timestamp directly from `.git/logs/HEAD` mtime via pure filesystem APIs (zero subprocesses).
  - Dedicated "Stale Projects" segmented view mode in `Findings.svelte` with idle days badges, dependency chips, 1-click batch selection, and inline dependency inspector drawer.
- **Children:** expand a finding into selectable children: Docker images, Ollama models,
  simulator devices, JetBrains IDE versions, app caches.
- **History view & Put Back (✓ Done 2026-11-02):**
  - Read `audit.jsonl` from application support via `reaper::read_audit`.
  - Dedicated `History.svelte` workspace view (⌘3) displaying audit cards, timestamps, modes, and receipts.
  - One-click **"Put Back"** action restores trashed items from Finder Trash (`~/.Trash`) back to original paths with safety checks.
- **Persist the last scan & Instant Startup (✓ Done 2026-11-02):**
  - Binary index cache header `[MAGIC: b"WOOSHIDX", VERSION: 1, EVENT_ID, TIMESTAMP, PAYLOAD_LEN]`.
  - Saves scan result to `~/Library/Application Support/Woosh/index.bin`.
  - Loaded automatically on startup via `api.loadCache()`, achieving **< 2 ms instant cold start**.
  - Titlebar displays `⚡ INDEX CACHE` status badge.
- **Settings + ignore list**, persisted in `~/Library/Application Support/Woosh/settings.json`:
- **First-run onboarding:** what Full Disk Access unlocks, and what Woosh never touches.
- Later: menu-bar extra, low-disk notification, weekly scheduled scan.

---

## Phase 4 — Performance (measure the whole pipeline, every time)

Current numbers: 6.7 s full sweep, discovery 433 ms, measuring about 85% of the total.

- **P1 — Overlap probes with discovery and measuring.** (✓ Done) `scan_streaming()` overlaps the
  concurrent probes leg with the discover + measure leg using `rayon::join`. Probes finish
  concurrently with filesystem walk, reducing probe phase wall-clock overhead to 0 ms.
- **P2 — Stream findings** (open item 2a): (✓ Done) `scan_streaming()` emits each `Finding` via
  callback as measured; Tauri shell re-emits as a `"finding"` event; UI appends streamed findings
  in real time.
- **P3 — `ATTR_DIR_ENTRYCOUNT`** (open item 2b): skip the final `getattrlistbulk` call that
  returns 0, which saves about one syscall per directory. **Re-derive the buffer layout with the
  C probe** in DEVELOPMENT.md (hard rule 5) and keep the byte differential (hard rule 6).
- **P4 — Hyper-Fast Persistent Indexer & Incremental FSEvents Sync (Blueprint: `docs/FAST_INDEXER_IMPLEMENTATION_PLAN.md`):**
  - Instant cold startup (< 3 ms) via zero-copy memory-mapped binary cache (`memmap2`).
  - Monotonic 64-bit `FSEventStreamEventId` tracking: replays only kernel-level modified directories since last event ID.
  - Re-evaluates touched paths in < 1 ms via `getattrlistbulk(2)`, eliminating full rescans entirely for routine usage.
  - In-memory Trigram Inverted Index + Roaring Bitmaps: sub-millisecond search (< 150 µs) with structured predicates (`size:`, `idle:`, `domain:`, `ext:`), 100x faster than macOS Spotlight.
- **P5 — Frontend:** (✓ Done) B5 ring buffer and B6 virtualized row windowing, plus building
  `selectedFindings` from an id → finding `Map` (O(selected) instead of O(N) filter on each toggle).

### Measured Pipeline Numbers (Apple Silicon host, `cargo run --release --example headless`)
- **Before optimization:**
  - Entries: 379,665 · Wall: 1,631 ms · Rate: 232,738 entries/s
  - Total reclaimable: 37.00 GB across 36 items
  - Phases: `discover` 340 ms, `measure` 659 ms, `probe` 535 ms, `done` 21 ms
- **After optimization (Phase 1 + Phase 4 P1/P2/P5):**
  - Entries: 391,410 (+11,745 entries from media roots) · Wall: 1,700 ms · Rate: 230,212 entries/s
  - Total reclaimable: **107.19 GB** across **84 items** (+70.19 GB found; Large files: 68.28 GB across 41 items)
  - Phases: `discover` 859 ms, `measure` 756 ms, `probe` **0 ms separate wall-clock** (overlapped via `rayon::join`), `done` 25 ms
  - Findings stream directly to UI as discovered and measured.

---

## Phase 5 — Release

- Notarization end to end (`make release`, which needs a Developer ID). Still open item 1.
- Tauri updater (needs signing first).


---

## Phase 6 — Anti-Emoji Purge, Bulletproof Shortcuts (⌘2), & Fast Filesystem Search Indexer (2026-11-02)

1. **Complete Anti-AI-Slop & Emoji Purge**:
   - Stripped all emojis (`🧹`, `⚡`, `↺`, `📈`, ``, `📄`, `📁`, `◈`, `📝`, `▣`, `🚀`, `🔍`, etc.) across titlebar, sidebar, search scopes, telemetry, and history views. Replaced with minimalist CSS micro-indicators and clean typography.
   - Removed fake `<span class="badge-tag">M3 · SILICON</span>` badge. Display real system information only (`arm64`).

2. **Bulletproof Workspace Shortcut Routing (`⌘2` Fix)**:
   - Configured capture-phase global keyboard listener (`window.addEventListener("keydown", handleGlobalKeys, { capture: true })`) in `App.svelte` and `<svelte:window onkeydown={handleGlobalKeys} />`.
   - Handled `e.code === "Digit2" || e.key === "2"` so workspace switching never fails regardless of keyboard layout or input focus.
   - Automatically focuses search input upon entering the Search workspace.

3. **High-Performance Filesystem Search Engine (`engine/src/index/search_index.rs`)**:
   - Fast parallel crawler over developer roots (`Desktop`, `Documents`, `Downloads`, `Projects`, `Developer`, `code`, `src`, `/Applications`) at 1M+ entries/s via `fastwalk`.
   - Dependency tree boundary pruning (`node_modules`, `.git`, `target`, `Pods`, `.Trash`, `Library/Caches`).
   - Query engine with tokenized structured predicates (`name:`, `dir:`, `app:`, `in:`, `ext:`, `size:`, `idle:`).
   - In-memory eviction upon reap: when items are reaped in Woosh, the paths are immediately evicted from the search index and saved to disk cache (`~/Library/Application Support/Woosh/search_index.bin`).
   - Rebuild Index CTA button and status indicator in `OmniSearch.svelte`.
