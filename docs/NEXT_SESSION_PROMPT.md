# Continue: Woosh — Switch Live UI to HIG Split View & Implement FSEvents Live Sync

You're picking up `/Users/forthtech/Desktop/Garabage-Cleaner-Thingy`. Woosh is an all-in-one
**Mac search + storage** app: find any file, folder, app or text, and reclaim space. It's a
Tauri v2 shell over a pure-Rust engine (`engine/`, which has no Tauri dependency) and a Svelte 5
frontend (`app/`).

**Read these first, in order:**
1. `CLAUDE.md`: the hard rules and constraints.
2. `docs/PLAN_AIO.md`: the authoritative plan. Section 0 and Section 5 reflect verified state.
3. Gallery review artifact: `~/.gemini/antigravity/brain/69a744a8-3dda-4ddb-a518-5d4e57495cb5/gallery_review.md`

## State at handoff (verified 2026-11-02)
- **`make ci` PASSES (exit 0).** Formatter, linter (clippy), 99 engine unit tests, 16 fixture tests,
  frontend svelte-check, and release benchmarks pass cleanly.
- **`make verify-macos` PASSES (exit 0).** Differential crawler benchmark achieves 1.5M entries/s.
- **`make app && make dmg` PASSES (exit 0).** Fresh signed DMG and .app built cleanly.
- **Search Engine (M1):** Real-time linear scan over 564,645 indexed items with Top-K `select_nth_unstable_by`
  achieves ~30 ms query times (~1.5 ms p95 in release mode). Extension matching scores .pdf and real files
  properly.
- **In-File Content Search (M4 / S5):** Engine now supports concurrent content search (`search_files_content`)
  over indexed text files under 500 KB in reverse mtime order, with early exit once the match limit is reached.
  Single-character search (`length >= 1`) is enabled.
- **Apple HIG UI Component Kit (M2 / R1–R2, R5):** 10-component kit built in `app/src/ui/` with Light and Dark
  semantic tokens in `app/src/app.css`. Tested via `/?gallery`.

## Next steps for this session
1. **Complete M2 — Switch Live UI to the HIG Split View (R3 & R4):**
   - **R3 Shell Migration:** Port `app/src/App.svelte` to use the full split view layout (`<Sidebar>`,
     `<Toolbar>`, `<main>`, `<Inspector>`) with sidebar collapse (⌘⌃S) and inspector toggle (⌘⌥I).
   - **R4 Port Views onto Component Kit:**
     - Replace legacy tables in `Findings.svelte` and `OmniSearch.svelte` with `<Table>` from `app/src/ui/`.
     - Connect selection and details into `<Inspector>`.
     - Replace modal dialogs with `<Sheet>`.
     - Delete obsolete monolith files (`Findings.svelte` and dead CSS in `OmniSearch.svelte`).
2. **M3 — Live Index Sync (S2 crawler + S4 FSEvents):**
   - Stream filesystem changes via `fsevent` so files created, moved, or deleted in Finder reflect in search
     in under 1 second without full rescans.
3. **Verification:**
   - Always run `make ci; echo "exit=$?"` (must be 0).
   - Run `make verify-macos; echo "exit=$?"` (must be 0).
   - Run `make app && make dmg` on host and verify release bundle.
