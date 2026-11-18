# Woosh — UI Craft Refactor Plan (Anti-AI-Slop Standard)

Benchmark: **Raycast / Linear / OrbStack**
Commitment: Dark-mode macOS utility, high information density, monochromatic calm, precision alignment.

---

## The Problem: Identifying "AI-Slop" in Developer Tooling

When LLMs generate user interfaces without rigorous design constraints, they default to recognizable tropes:
1. **Pill & Badge Confetti:** Every state, tag, flag, and category is rendered as a colorful rounded pill (`border-radius: 9999px`) with 10–20% opacity backgrounds and colored borders. In a list of 30 items, 80+ brightly colored pills compete for attention.
2. **Emoji Prepending:** Inserting emojis (`☁`, `⚙`, `◌`, `▤`, `▥`, `◈`, `◆`, `●`) in front of text buttons, labels, and table cells to artificially make them look "designed".
3. **Stacked Toolbars / Double Navigation:** Adding another layer of toolbar underneath an existing toolbar whenever new filters are introduced, wasting vertical space and creating visual clutter.
4. **Unstructured Card Sprawl:** Expanded details dumped into unaligned key-value rows or floating web cards with rounded corners, drop shadows, and poor information hierarchy.
5. **Lack of Tabular Alignment:** Proportional fonts used for numbers, dates, and sizes causing ragged visual columns that jump around as numbers change.

---

## Craft Standards & Anti-AI-Slop Principles

### Principle 1: Monochromatic Calm (95% neutral, 5% semantic)
The interface is near-black obsidian, dark gunmetal, and slate. Color is an exception, not the rule.
- Base background: `#090B0E` (deep obsidian canvas)
- Surface elevation 1 (sidebar, titlebar, toolbars): `#0D1015`
- Surface elevation 2 (cards, active segments, inputs): `#13181F`
- Surface elevation 3 (popovers, active highlights, tooltips): `#1A202A`
- Hairline borders: `1px solid rgba(255, 255, 255, 0.06)` or `#1E2530`
- Text hierarchy:
  - High ink: `#F0F4F8` (headings, primary titles, reclaimable numbers)
  - Mid ink: `#94A3B8` (subtitles, paths, idle times, labels)
  - Faint ink: `#525E6F` (meta keys, inactive tabs, shortcuts)

### Principle 2: Strict Typographic Hierarchy & Tabular Alignment
- Font family: `-apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif`
- Monospace family: `ui-monospace, "SF Mono", Menlo, monospace`
- All sizes, dates, hashes, IDs, and metrics use `font-variant-numeric: tabular-nums` or `.mono`.
- Font scale:
  - Title: 13px / font-weight: 550
  - Subtitle / Paths: 11px / font-weight: 400
  - Table headers & section keys: 10px / font-weight: 600 / letter-spacing: 0.05em / uppercase
  - Badges / Micro-tags: 10px / font-weight: 500 / no chunky border-radius

### Principle 3: Micro-Indicators Over Neon Pills
Instead of `[☁ PULLABLE]` or `[● ACTIVE]` pills:
- Replace with a 6px status dot (`● active`, `○ stopped`, `● dangling`) using soft desaturated colors:
  - Running / Active: `#38BDF8` (soft sky/slate blue)
  - Stopped / Inactive / Dangling: `#94A3B8` (quiet slate) or `#FBBF24` (soft amber if caution)
  - Safe / Reclaimable: `#34D399` (refined emerald/mint)
- Secondary origin text is quiet typography in the subtitle or dedicated column:
  e.g., `registry` (dim) or `local build` (subtle violet/slate text, no heavy container pill).

### Principle 4: Unified 36px macOS Command & Segmented Navigation
Consolidate the top toolbar and docker sub-navigation into one sleek, 36px segmented bar:
- Native macOS style segmented control: `[ All | Images (12) | Containers (8) | Volumes (4) | Build Cache ]`
- Recessed background with sliding or elevated active segment.
- Integrated search input with clear shortcut hint `⌘F`.
- Dynamic, contextual batch action on the right side of the bar that adapts cleanly to the active view:
  - On "All": "Select All Safe" or "Select All Reclaimable"
  - On "Images": "Select Pullable Unused" or sub-origin toggle
  - On "Containers": "Select Stopped Containers"
  - On "Volumes": "Select Dangling Volumes"

### Principle 5: Clean Table Row Architecture
Strict column grid with fixed alignment:
`[ 36px Checkbox | flex Item (Title + Path/Subtitle) | 90px Origin | 80px Idle | 70px Risk | 120px Size ]`
- No emoji prefixes.
- Clean row hover states: subtle `rgba(255, 255, 255, 0.03)` with no jarring color shift.
- Selected row state: restrained mint tint `rgba(52, 211, 153, 0.04)` with a crisp hairline border.

### Principle 6: Developer Inspector Drawer (Expanded Row)
When a row is clicked or Enter is pressed:
- Rather than an arbitrary list of badges and text, render a precision 2-column technical inspector:
  - Left column (Overview): Resource ID with one-click copy button, full tags / mounts, size breakdown (exclusive vs shared).
  - Right column (Safety & Actions): Exact safety policy explanation, terminal restore / recreate command in a stylized code block with copy button, and quick actions (Reveal in Finder, Quick Look).
- Clipboard feedback: brief "Copied!" inline state.

---

## Implementation Plan

1. **Step 1: CSS Refactor (`app/src/app.css` & `app/src/App.svelte`)**
   - Update color variables to the Raycast/Linear obsidian palette.
   - Hairline borders (`rgba(255, 255, 255, 0.06)`).
   - Refine `.btn`, `.btn-primary`, `.btn-danger`, checkboxes, and scrollbars.
   - Refine sidebar domain list and header titlebar.

2. **Step 2: Component Refactor (`app/src/views/Findings.svelte`)**
   - Eliminate double toolbars: integrate into a unified 36px segmented header.
   - Strip emojis and replace pill confetti with 6px status dots and quiet micro-tags.
   - Update table header and row grid to strict column alignment.
   - Implement the 2-column Developer Inspector Drawer with copy-to-clipboard actions and syntax-styled command box.

3. **Step 3: Verification & Polish**
   - Run `make ui` (0 errors, 0 warnings).
   - Run `make ci` (all Docker tests pass).
   - Run `make verify-macos` on host.
   - Build and verify `.app` and DMG.
