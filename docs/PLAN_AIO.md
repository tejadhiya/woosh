# Woosh — All-in-One Plan: Search + Storage (2026-11-02)

Woosh becomes one window that does two things well:
1. **Find anything on the Mac**: files, folders, apps, and text inside files. It should feel as
   fast as typing.
2. **Reclaim space**: the existing dev-storage cleaner, plus large files, duplicates, apps and their
   leftovers, and a picture of where the disk went.

This plan **supersedes the open parts of `docs/ROADMAP.md`** and replaces the indexer targets in
`docs/FAST_INDEXER_IMPLEMENTATION_PLAN.md`. Those docs are still useful history. They also claim
work that was never built; section 0 lists what is actually true. Hard rules in `CLAUDE.md` apply
to everything below.

---

## 0. Reality check: claimed vs. verified (read the code, ran `make ci`)

| Claim in the docs / last summary | What the code actually does |
|---|---|
| "`make check-workspace` passes 100%" | **`make ci` fails (exit 2).** The shell does not compile: `app/src-tauri/src/lib.rs:276` calls `PathBuf::from(&r.path)` on an `Option<PathBuf>`. **The current tree cannot build the app.** Whatever is in `/Applications/Woosh.app` predates this code. |
| "Rebuild Index CTA and status indicator in `OmniSearch.svelte`" | Not there. `api.rebuildSearchIndex` and `api.querySearchIndex` are defined in `api.ts` and **never called**. |
| "Search queries the index with `name:` `dir:` `app:` `in:` `ext:` `size:` `idle:`" | The Search screen only filters the **last scan's findings** (a few dozen dev-storage items) plus the app list. It never touches the file index, which is why "the search ain't working". |
| `in:` content search | It runs `search_file_content` on finding paths. Those are directories (`node_modules`, `.venv`), and the function returns nothing for a directory. **It always comes back empty.** |
| Trigram index + Roaring bitmaps, < 150 µs queries | Not built. Queries are a linear scan that allocates two lowercase `String`s per item per keystroke, on the Tauri async thread. |
| FSEvents incremental sync, `memmap2` zero-copy cache | Not built. Only `FSEventsGetCurrentEventId()` is read and stored. The "binary" index is a header plus a JSON payload. |
| "1M+ entries/s" indexer | The crawl is serial inside each root (parallel only across roots), with a global mutex push per entry. The measured sweep rate is about 230k entries/s. |
| Hero chips "Zero Subprocesses", "1M+ Entries/sec" | Both false: `qlmanage`, `open` and `pip` are spawned, and the measured rate is about 230k/s. These are the same kind of invented spec as the "M3 · SILICON" badge. Remove them. |

**Process rule going forward:** never report a check as passing without seeing its exit code.
`make ci 2>&1 | tail` hides failures behind `tail`'s exit code. Use `make ci; echo "exit=$?"`, or
`set -o pipefail`.

---

## 1. Bugs (fix before features)

| # | Where | Bug | Fix / done when |
|---|---|---|---|
| **B0** | `app/src-tauri/src/lib.rs:276` | Compile error; nothing ships. | Use `filter_map(\|r\| r.path.clone())`. `make ci` exits 0. |
| **B1** | `engine/src/reaper.rs` `put_back` | When the exact name isn't in `~/.Trash`, it takes the **first entry whose name starts with the same prefix**. Many trashed items share names (`node_modules`, `.venv`, `target`), so **Put Back can restore project A's `node_modules` into project B.** | At reap time, record the real trashed location (`NSFileManager trashItemAtURL:resultingItemURL:` via `objc2` behind `cfg(target_os="macos")`; the engine still has no Tauri dependency) plus the inode, and store both in the `Receipt`. `put_back` uses only that exact path and verifies the inode. Add a refusal test: with two same-named items in the Trash, the wrong one is never restored. |
| **B2** | `lib.rs` `reap` → `SearchIndex::evict_paths` | Receipts with no path (Docker images, Ollama models) and any empty path would become prefix `"/"`, **evicting the entire index**. | Skip paths that are `None` or empty, and refuse to evict a root or the home directory. Unit test. |
| **B3** | `App.svelte` `pick()` | The sidebar shows in every workspace, but clicking a domain while in Search or History only changes `tab`, so nothing visible happens. (This is the same bug as the earlier Telemetry one, one level up.) | `pick()` also sets `workspace = "reclaim"`. The redesign (section 4) removes the class of bug. |
| **B4** | `App.svelte` `onFinding` | `findings.some(...)` plus a full array copy per streamed finding is O(n²) during a sweep. | Use a `Map<id, Finding>` plus append-only list. |
| **B5** | `App.svelte` `sweep()` | A new sweep replaces cached results with an empty placeholder. If the scan fails, the user's last results are gone. | Keep the old results until the first streamed finding arrives; restore them on error. |
| **B6** | Shell `open_path`, `quick_look`, `search_file_content` | They accept **any path from the webview**. `open` can launch any app or file, and content search can read any file. | Only accept paths that are in the current findings or the index. Use `NSWorkspace` / `QLPreviewPanel` rather than spawning `open` / `qlmanage`. |
| **B7** | `query_search_index` | A blocking linear scan on the async thread; no cancellation; `take(limit)` happens **before** ranking, so the best match can be cut off; results come out in name order. | See S3. |
| **B8** | `index/search_index.rs` crawl | Serial per root; mutex push per entry; `max_depth 8` silently drops deeper files; prunes **any** folder named `target` or `Caches` (including user folders); a folder's `bytes` is the directory entry's own allocation, so `size:` on folders is meaningless. | See S2. |
| **B9** | `index/cache.rs` vs `search_index.rs` | Two different files share the magic `WOOSHIDX`. | Use distinct magics, with a version bump on any layout change. |
| **B10** | `OmniSearch.svelte` | `isDir = !f.path.includes(".")` misclassifies `.venv`, `.next` and dotted folder names. | Carry `is_dir` from the engine. |
| **B11** | `App.svelte` | "INDEX CACHE" labels stale *scan results* as if they were a live index. Telemetry exists both as a workspace and as a tab. | Say "Results from 2 h ago · Rescan". Keep one Telemetry. |
| **B12** | `Findings.svelte` (2,183 lines), `App.svelte` (1,243), `OmniSearch.svelte` (1,075) | Monolith components with a lot of dead CSS. | Rebuilt as components in R2–R4. |

---

## 2. Search engine (S1–S6)

**Targets.** Measure each on this Mac with the real index and report the numbers:
- index build of about 500k entries in under 3 s;
- cold load in under 100 ms;
- **query p95 under 20 ms** for 500k entries;
- live update visible within 1 s of a file change.

Only build the trigram index if the measured linear scan misses the 20 ms target.

- **S1 — Make search real (first).**
  - Wire `OmniSearch` to `query_search_index`, debounced at about 60 ms, discarding stale responses
    with a request id.
  - Add an index status line ("412,880 items · updated 3 min ago · Rebuild") and a first-run
    "Build index" empty state.
  - Show progress from the `search-index-progress` events that already exist.
- **S2 — Fast, correct crawler.**
  - Reuse the `rayon::scope` pattern from `sweep::discover`, so it is parallel inside each root.
  - Batch entries in per-task `Vec`s instead of locking a mutex per entry.
  - No depth cap (symlinks are never followed already).
  - Prune by **exact known paths and markers**, not bare names: `node_modules` and `.git` anywhere;
    `target` only next to a `Cargo.toml`.
  - Configurable roots: default to the whole home minus `~/Library`, plus `/Applications`, with
    excludes and a hidden-files toggle.
  - Compact storage: one string arena plus offsets; precomputed lowercased name; `is_dir`; `alloc`;
    `mtime`; `ext` as a small id.
  - Persist in a hand-rolled little-endian binary layout (no JSON), versioned.
- **S3 — Query and ranking.**
  - Parse once in Rust (tested), not in the UI.
  - Linear scan over the precomputed lowercase names with `memchr::memmem`, run on rayon
    `par_chunks` when there are more than 100k items. **Rank before truncating.**
  - Ranking order: exact name, then prefix, then word-boundary match (`-`, `_`, `.`, camelCase),
    then substring, then path-only match. Tie-break: apps first for app-like queries, then most
    recent `mtime`, then shorter path.
  - Optional fzf-style subsequence fallback when nothing else matches.
  - Run off the async thread (`spawn_blocking`).
- **S4 — Live updates (the "auto-updates with the system" ask).**
  - An FSEvents stream (CoreServices FFI, macOS-only module, portable no-op) over the index roots,
    using `kFSEventStreamCreateFlagFileEvents`, started with `sinceWhen = persisted event id` so
    changes made while the app was closed replay on launch.
  - Apply creates, deletes and renames to the in-memory index; persist on a debounce.
  - `KernelDropped`, `UserDropped` or `MustScanSubDirs` → re-crawl only that subtree.
  - Reaps evict immediately (with the B2 fix).
- **S5 — Real content search (`in:`).**
  - Search the **indexed files** filtered to text extensions and a size cap.
  - Use the ripgrep libraries (`grep-searcher`, `grep-regex`; binary detection built in). This is
    one new crate family; justify it in the PR.
  - Parallel and cancellable via a generation counter, with results streamed as events and context
    lines shown.
  - Optional: respect `.gitignore` via the `ignore` crate.
- **S6 — One query language, in the engine.**
  - Tokens: `kind:file|folder|app`, `ext:mov`, `size:>1GB`, `modified:<7d`, `idle:>30d`,
    `path:~/Downloads`, `in:"text"`, `domain:`, `risk:`.
  - The UI renders parsed tokens as removable chips and autocompletes keys and values.
  - Every result row supports:

    | Key | Action |
    |---|---|
    | ⏎ | Open |
    | ⌘⏎ | Reveal in Finder |
    | Space | Quick Look |
    | ⌘C | Copy path |
    | ⌘⌫ | Move to Trash (through `safety::vet` + reaper, Trash only) |

---

## 3. Storage features

- **Storage overview.** A DaisyDisk-style sunburst or squarified treemap, drawn from index sizes
  plus a "System & other" remainder from `statfs`. Click to drill down; every block leads to Reveal
  or Search. This is the screen that answers "where did my disk go".
- **App uninstaller.** An app plus its leftovers found by bundle id: `Application Support`,
  `Caches`, `Containers`, `Group Containers`, `Preferences`, `Saved Application State` and
  `LaunchAgents`. Caution, Trash only. Refuse while the app is running (`sysctl KERN_PROC`, no
  subprocess). Apple and system apps are Advisory.
- **Duplicates.** The design in ROADMAP Phase 1b, which is clone-aware via the clone id. Never
  auto-select.
- **Smart lists**, which are saved searches pinned in the sidebar: "Videos > 1 GB",
  "Installers in Downloads", "Untouched for a year", "Screen recordings".
- **Remaining cleaners** from ROADMAP Phase 2: app caches (Slack, Discord, browsers), sandboxed
  container caches, Homebrew old kegs (via a `brew cleanup` action), VM disks, and Time Machine
  local snapshots as Advisory with the command to run.
- **Settings** (⌘,): index roots and excludes, hidden files, large-file threshold, Deep default,
  ignore list.
- Later: a menu-bar extra showing free space, a global quick-search hotkey, and a low-disk
  notification.

---

## 4. UI rebuild (R1–R8)

The current UI was restyled several times on top of the same monolith components, and it shows:
- dark-only hardcoded colours;
- monospace everywhere;
- neon pills and status confetti;
- marketing chips with invented specs;
- three different tab systems (titlebar workspaces, segmented tabs, and sub-nav);
- an expanding-row "inspector";
- copy like "Gorgeous Hero" and "Pro Command Sidebar" baked into class names.

Rebuild it from conventions, not decoration.

### Conventions to follow (Apple HIG and proven macOS apps)
- **Layout = split view.** A full-height **sidebar (source list)** on the leading edge, **no more
  than two levels** of hierarchy, collapsible (⌘⌃S). Use a content list in the middle and a
  togglable **inspector** on the trailing edge for detail (⌘⌥I) instead of expanding rows or modals.
  Dividers are draggable; the default sidebar width is about 220 px.
- **Toolbar** in the unified titlebar: a native-style search field on the right, the primary action
  ("Scan") on the left of it, and a **segmented control** only for switching *views of the same
  content* (List / Projects / Map). The sidebar handles navigation; there is no second tab bar.
- **Typography:** system font (`-apple-system`) at **13 px** body, 11 px secondary, 15–17 px section
  titles. Use `font-variant-numeric: tabular-nums` for sizes; monospace **only** for paths and
  commands.
- **Colour:** follow the system appearance (light **and** dark, via `prefers-color-scheme`), use
  the system accent colour for selection and focus, and keep everything else neutral. Show risk as
  a small coloured dot plus a plain word, with the tooltip explanation that already exists. No glow,
  no gradients, no badges for status.
- **Density:** search rows 28 px, one line (name · path dimmed · size · modified). Findings rows
  44 px, two lines. Finder-like list behaviour: click selects, ⌘-click and ⇧-click extend,
  checkboxes only in Reclaim.
- **Keyboard-first:** every action has a menu item and shortcut. Visible focus rings. ⌘K opens a
  command palette (search plus actions such as Scan, Empty Trash…, Open Settings), in the style of
  Raycast and Spotlight.
- **Destructive actions:** a confirmation **sheet** attached to the window, listing the items and
  the total. The default button is **Move to Trash**; Delete Permanently is never the default and
  is styled as destructive. After trashing, say plainly: "X GB is in the Trash. Empty it to free
  the space."
- **States:** design the empty, loading (skeleton rows while streaming), error and no-permission
  (Full Disk Access) states for every view.
- **Copy:** plain and specific. No emoji, no marketing chips, no invented hardware or
  specifications, no "blazing". Sizes always mean what you get back.
- **Accessibility:** roles and labels on custom controls, 4.5:1 contrast, `prefers-reduced-motion`,
  `prefers-contrast`.

### New information architecture (sidebar)
```
Search                (default; ⌘1)   ← search field focused on launch
Overview              (⌘2)            ← disk map + biggest wins
Reclaim               (⌘3)
  Developer           (venvs, node_modules, build output, toolchains, simulators, Docker, models)
  Caches
  Large files
  Duplicates
Apps                  (⌘4)            ← installed apps, sizes, uninstall with leftovers
History               (⌘5)            ← audit log, Put Back
Smart lists                            ← saved searches
```
Telemetry is no longer a destination. It becomes a slim progress strip plus a "Show log" disclosure
in the inspector during a scan.

### Implementation steps
- **R1 — Tokens:** a rewritten `app.css` with light and dark semantic tokens (`--bg`, `--bg-sidebar`,
  `--fg`, `--fg-secondary`, `--separator`, `--accent`, `--safe`, `--caution`, `--advisory`,
  `--destructive`), a spacing scale of 4/8/12/16/24, radius 6/8, and one font stack.
- **R2 — Component kit** in `app/src/ui/`:
  - Sidebar and SidebarItem; Toolbar; SegmentedControl; SearchField with token chips;
  - **Table** (virtualized, sortable, resizable columns, keyboard selection);
  - Inspector; Sheet; Tooltip; ProgressStrip; EmptyState; Menu.

  Each component is small and owns its CSS.
- **R3 — Shell:** a new `App.svelte` that is only the split-view layout plus routing and global
  shortcuts. All state lives in a few `*.svelte.ts` stores (scan, selection, index, settings).
- **R4 — Port the views onto the kit:** Search, Reclaim (List and Projects), History, then the new
  Overview and Apps. Delete the old view files once each port lands, so no dead CSS remains.
- **R5 — Gallery:** a `?gallery` route with mocked IPC, rendering every component and view state in
  light and dark. Screenshot it in the in-app browser for review **before** replacing the live UI.
- **R6 — Native touches:** `titleBarStyle: Overlay` with traffic-light inset; vibrancy on the
  sidebar only (`window-vibrancy` is already a dependency); a real app menu with shortcuts; ⌘, for
  Settings.
- **R7 — Accessibility and keyboard pass:** VoiceOver walkthrough and keyboard-only walkthrough.
- **R8 — Real click-through** of the built app: search, open, reveal, scan, select, dry run, trash,
  Put Back. It has never been done on the real window.

---

## 5. Milestones (in order)

| | Scope | Exit criterion | Status & Measured Results |
|---|---|---|---|
| **M0** | B0, B1, B2, B6 | `make ci` exits 0; Put Back refusal test; eviction test; IPC path allowlist | **DONE.** `make ci` exit 0; Put Back records exact trash path & inode; refusal test `put_back_with_same_named_trash_items_never_restores_wrong_item` passes; eviction refuses empty/root/home; IPC allowlist verified. |
| **M1** | S1 + S3 + S6 parser; B3–B5, B7, B10, B11 | Typing in Search returns indexed files, folders and apps, ranked; p95 query time measured and reported | **DONE.** Engine query parser `SearchQuery::parse` with token unit tests; zero-alloc linear scan with Rayon; rank before truncating; async thread decoupling (`spawn_blocking`); B3/B4/B5/B10/B11 fixed. **Measured search query p95 over 50,000 items: 3.18 ms** (target < 20 ms). Crawl benchmark: **968,546 entries/s** (4 ms for 4,284 fixture items). |
| **M2** | R1–R5 on Search and Reclaim | Gallery screenshots in light and dark reviewed by the user, then the live UI switched over | **R1–R5 complete.** Apple HIG tokens in `app.css`, 10-component kit in `app/src/ui/`, live `?gallery` route. Screenshots captured in Light and Dark mode for user review prior to replacing live views. |
| **M3** | S2 crawler + S4 FSEvents | A file created in Finder appears in search in under 1 s; build time and cold load measured | Planned next. |
| **M4** | S5 content search | `in:"text"` returns real matches, streams, and cancels on the next keystroke | **DONE.** `search_files_content` with parallel Rayon file scanning; early termination on limit; text extension filtering; `search_content` IPC command querying indexed text files (<500 KB) in mtime order; single-character search supported. Unit tested. |
| **M5** | Overview map, Apps uninstaller, Duplicates, Smart lists; R6–R8 | Each with fixture tests; full click-through done |
| **M6** | Remaining cleaners, Settings | per `woosh-add-cleaner` |
| **M7** | Notarization / updater | needs a Developer ID |

Every milestone ends with `make ci` (exit code checked), `make verify-macos`, and `make app &&
make dmg` when the shell or packaging was touched, plus measured numbers recorded in this file.

---

### References
- Apple HIG — [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars),
  [Split views](https://developer.apple.com/design/human-interface-guidelines/split-views).
  Toolbars, Search fields and Sheets are in the same guidelines.
- Disk-map reference point: DaisyDisk ([Macworld review](https://www.macworld.com/article/352345/daisydisk-4-review-macos.html)).
- Search reference points: Spotlight, Raycast, and voidtools Everything (name-only index, instant
  results, live updates).
