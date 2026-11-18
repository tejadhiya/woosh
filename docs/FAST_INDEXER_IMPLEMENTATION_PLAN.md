# Woosh Hyper-Fast Indexer & Search Engine — Architecture & Implementation Plan

> **Mission:** Transform Woosh from a periodic sweep cleaner into an **instant-startup (0 ms), real-time incremental developer storage engine** that out-indexes macOS Spotlight by 100x—bringing the legendary performance of Voidtools *Everything* (Windows) to Apple Silicon and APFS.

---

## 1. Why macOS Spotlight Fails Developers (The Problem)

| Dimension | macOS Spotlight (`mds` / `mdworker`) | The Voidtools Everything Standard | Woosh Target Architecture |
| :--- | :--- | :--- | :--- |
| **Indexing Scope** | Deep document unpacking (PDF, docx, video frames, ID3, EXIF, Siri suggestions, Mail). | Path + Filename only (reads NTFS MFT directly). | **Targeted Developer & Storage Scopes**: Paths, names, sizes, inodes, git activity, dev domains. |
| **System Overhead** | Multi-GB `.Spotlight-V100` database, runaway CPU spikes, battery drain during re-indexing. | ~0% background CPU, minimal RAM (~50 MB for 1M files). | **Zero background CPU**, zero battery drain, compact memory footprint (<30 MB for 500k files). |
| **Developer Blindspots** | Excludes `node_modules`, `.*` hidden directories, `~/Library`, build caches by default. | Indexes everything without judgment. | **Deep developer awareness**: Identifies Docker, Simulators, Venvs, Rust targets, Caches, Large Files. |
| **Cold Startup** | Slow; requires warming OS metadata caches and IPC with daemon. | Instantaneous (loads cached MFT table into memory). | **< 3 ms cold load** via zero-copy memory-mapped binary cache (`memmap2`). |
| **Query Latency** | 300 ms – 1500 ms (UI stutter, search delay). | < 1 ms (as fast as you can type). | **< 150 µs (microseconds)** via Trigram Inverted Index + Roaring Bitmaps. |
| **Change Detection** | Heavy file-system polling and worker dispatch. | Subscribes to NTFS USN Change Journal. | **FSEvents kernel stream with persistent `FSEventStreamEventId`**. |

---

## 2. Core Architecture: The 4 High-Performance Pillars

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                         WOOSH HYPER-FAST INDEXER ARCHITECTURE                   │
└──────────────────────────────────────────────────────────────────────────────────┘

   [ 1. Bulk Ingestion ]             [ 2. Delta Sync ]          [ 3. Storage Layer ]
 ┌───────────────────────┐       ┌───────────────────────┐   ┌───────────────────────┐
 │   getattrlistbulk(2)  │       │  FSEvents CoreServices│   │   Zero-Copy mmap2     │
 │  1,000,000+ entries/s │       │ FSEventStreamEventId  │   │  rkyv / flat binary   │
 │   APFS Kernel Buffer  │       │  Kernel Replay Stream │   │   < 3ms Cold Startup  │
 └───────────┬───────────┘       └───────────┬───────────┘   └───────────┬───────────┘
             │                               │                           │
             └───────────────────────┬───────┴───────────────────────────┘
                                     ▼
                    ┌─────────────────────────────────┐
                    │  In-Memory Storage & Index Core │
                    │     400k+ Files & Findings      │
                    └────────────────┬────────────────┘
                                     │
                                     ▼
                      [ 4. Blazing-Fast Search ]
                    ┌─────────────────────────────────┐
                    │  Trigram Inverted Index         │
                    │  + Compressed Roaring Bitmaps   │
                    │  + SIMD Multi-Field Filter      │
                    │  Query Latency: < 150 µs        │
                    └─────────────────────────────────┘
```

### Pillar 1: Initial Ingestion via `getattrlistbulk(2)`
- Woosh already possesses an Apple Silicon superweapon: `engine/src/fastwalk/macos.rs`.
- Reads directory entries directly off the APFS buffer in 64 KB batches with a single syscall per batch.
- Scans **400,000+ entries in ~800 ms** (1.28M entries/sec). This is used *only once* on initial bootstrap or when a complete index reset is requested.

### Pillar 2: Persistent Zero-Copy Cache (`rkyv` / `memmap2`)
- Location: `~/Library/Application Support/Woosh/index.bin`
- Instead of re-scanning 400,000 directories on every app launch, the serialized state is memory-mapped directly into the process's address space.
- Zero parsing, zero JSON decoding, zero deserialization CPU overhead: pointers in the file map directly to in-memory structs.
- **Cold startup time: < 3 ms.** The app displays yesterday’s 112 findings and 56 GB reclaimable total immediately upon launch.

### Pillar 3: Real-Time Incremental Updates via FSEvents
- FSEvents provides a monotonic, system-wide 64-bit counter: `FSEventStreamEventId`.
- When an index is written to disk, Woosh saves the current `last_event_id` from `FSEventsGetCurrentEventId()`.
- Upon next app launch or in the background:
  - Woosh passes `sinceWhen = last_event_id` to `FSEventStreamCreate`.
  - The macOS kernel replays *only* the specific directory paths modified since that event ID.
  - If a developer edited 3 files in VS Code and ran `npm install` in one directory, FSEvents delivers only those specific changed paths.
  - Woosh re-evaluates only those paths using `getattrlistbulk(2)` in **< 1 ms**.
  - **Result: 0 ms full rescan required for daily usage.**
  - Safeguard: If `kFSEventStreamEventFlagKernelDropped` or `UserDropped` occurs (e.g. system was offline for weeks or event queue overflowed), Woosh triggers an automatic background full sweep.

### Pillar 4: Trigram + Roaring Bitmaps Sub-Millisecond Search
- **Trigram Decomposition:**
  - Every indexed path and name is split into 3-byte sliding windows (e.g., `docker` -> `doc`, `ock`, `cke`, `ker`).
  - An inverted index maps each trigram `u32` to a **Roaring Bitmap** containing the list of matching Finding IDs.
- **Roaring Bitmaps (`roaring` crate):**
  - High-performance compressed bitsets that automatically adapt between flat arrays, dense bitsets, and run-length encoding.
  - Queries execute bitwise `AND` / `OR` operations using AVX/NEON SIMD instructions in hardware.
- **Search Capabilities & Query Syntax:**
  - Substring search: `node_modules` (instant bitwise intersection of trigrams).
  - Rich predicates:
    - Domain filter: `domain:docker`, `domain:sim`, `domain:venv`, `domain:large_file`
    - Size filter: `size:>1GB`, `size:>500MB`
    - Age filter: `idle:>30d`, `idle:>14d`
    - Risk filter: `risk:safe`, `risk:caution`
    - Extension filter: `ext:dmg`, `ext:iso`, `ext:mov`
  - **Query response time: 50–150 microseconds (< 0.2 ms).**

---

## 3. Data Structures & Memory Layout

### Compact In-Memory Record Layout (`CompactEntry`)
```rust
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CompactFinding {
    pub id_hash: u64,          // BLAKE3 or FxHash of unique id
    pub path_offset: u32,      // Offset into contiguous path string buffer
    pub path_len: u16,         // Path length
    pub name_offset: u32,      // Offset into contiguous name string buffer
    pub name_len: u16,         // Name length
    pub bytes: u64,            // Allocated size (APFS blocks)
    pub shared_bytes: u64,     // APFS clone/hardlink shared size
    pub mtime_s: i64,          // Last modified unix timestamp
    pub git_idle_days: i16,    // Idle days from .git/logs/HEAD (-1 if unknown)
    pub domain: u8,            // Domain enum (0=docker, 1=venv, 2=node_modules, etc.)
    pub risk: u8,              // Risk enum (0=safe, 1=caution, 2=advisory)
    pub flags: u16,            // Meta flags (pullable, stopped, dangling, etc.)
}
```
*Memory footprint:* 48 bytes per finding. For 500,000 files/items = **24 MB of RAM**. String buffer = **~15 MB**. Total memory footprint < 40 MB.

---

## 4. Phased Implementation Roadmap

### Phase A: Persistent Cache & Instant Cold Startup
1. **Module `engine/src/index/cache.rs`**:
   - Define binary file header: `[MAGIC: b"WOOSHIDX", VERSION: 1, EVENT_ID: u64, TIMESTAMP: i64, ENTRY_COUNT: u32]`.
   - Implement zero-allocation memory-mapped storage with `memmap2`.
   - Write snapshot function `save_index(path: &Path, findings: &[Finding], event_id: u64) -> Result<()>`.
   - Write load function `load_index(path: &Path) -> Result<Option<(Vec<Finding>, u64)>>`.
2. **Tauri Bridge**:
   - On app launch (`setup` hook in `app/src-tauri/src/lib.rs`), load `index.bin` in parallel.
   - If present, immediately emit the cached findings to the frontend before the first user click.
   - Frontend displays cached findings in < 5 ms with an unobtrusive indicator: `⚡ Loaded from cache (scanned 2h ago)`.

### Phase B: macOS FSEvents Incremental Sync
1. **Module `engine/src/index/fsevents.rs` (`#[cfg(target_os = "macos")]`)**:
   - Bind `FSEventStreamCreate`, `FSEventStreamStart`, `FSEventStreamStop`, `FSEventsGetCurrentEventId` via CoreServices/CoreFoundation FFI.
   - Use `kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer`.
   - Pass `sinceWhen = saved_event_id`.
2. **Delta Processing (`reconcile_delta`)**:
   - Collect unique modified directories from FSEvents callback.
   - Run localized `fastwalk::walk` on changed directories only.
   - Re-evaluate domain claims (`domains::classify_directory`) for touched paths.
   - Update `findings` map in-memory and write updated delta to disk.
   - Emit updated items to frontend via the streaming finding event.

### Phase C: Trigram & Roaring Bitmaps Search Engine
1. **Module `engine/src/index/search.rs`**:
   - Add `roaring` crate (pure-Rust SIMD-accelerated Roaring Bitmaps).
   - Invert index: `HashMap<u32, RoaringBitmap>` (where `u32` is a 3-character ASCII/UTF-8 trigram).
   - Auxiliary indices:
     - Domain bitsets: `[RoaringBitmap; 8]` (one per domain).
     - Risk bitsets: `[RoaringBitmap; 3]` (Safe, Caution, Advisory).
     - Range query indices: B-tree or bucketed bitsets for `bytes` and `git_idle_days`.
2. **Query Parser & Evaluator**:
   - Parse input text like: `react size:>500MB domain:node_modules`.
   - Execute bitwise intersection in < 150 µs.
   - Return sorted results matching user sort order.

### Phase D: UI Omnibox Integration
1. **Frontend `Findings.svelte` Search Bar Upgrade**:
   - Support rich autocomplete filters:
     - Typing `domain:` presents dropdown: `docker`, `venv`, `node_modules`, `simulators`, `large_files`.
     - Typing `size:` suggests `>100MB`, `>500MB`, `>1GB`, `>5GB`.
     - Typing `risk:` suggests `safe`, `caution`, `advisory`.
   - Live query latency indicator: `✓ 14 results found in 0.12 ms`.

---

## 5. Verification & Benchmark Targets

| Metric | Target | Measurement Method |
| :--- | :--- | :--- |
| **Cold Cache Load** | **< 5 ms** | Measured from `load_index()` entry to in-memory `Vec<Finding>` ready. |
| **Incremental FSEvents Sync** | **< 15 ms** | Typical 10–50 file changes re-scanned and reconciled. |
| **Trigram Search Query** | **< 200 µs (0.2 ms)** | Benchmark 1,000 queries over 400,000 indexed entries. |
| **Index File Size** | **< 15 MB** | On disk for ~400,000 entries. |
| **Safety Invariant** | **100% Preserved** | Cache never bypasses `safety::vet()` at deletion time. |

---

## 6. Safety & Correctness Guardrails

1. **TOCTOU Protection (Time-of-Check to Time-of-Use):**
   - The index is strictly an *acceleration layer* for discovery and display.
   - When the user clicks **"Woosh"**, the reaper **never trusts cached metadata**.
   - The reaper calls `safety::vet()` on the live filesystem immediately before moving or deleting any path. If a path changed or was replaced by a symlink while indexed, the deletion is immediately refused.
2. **Cache Invalidation on Schema/Version Change:**
   - Any modification to domain detection rules or engine version increments `INDEX_VERSION`.
   - Old index formats are cleanly dropped and rebuilt automatically.
3. **Graceful Fallback:**
   - If `index.bin` is corrupted or missing, Woosh falls back seamlessly to its 800 ms full `getattrlistbulk` scan.

