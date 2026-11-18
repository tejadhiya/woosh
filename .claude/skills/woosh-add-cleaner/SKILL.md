---
name: woosh-add-cleaner
description: Add or modify a Woosh cleaner — a new cache path, a new detector for project directories, or a new scan domain. Use when asked to make Woosh find or clean something it currently misses, or to change what it reports. Covers the two kinds of cleaner, the safety contract, and the tests each needs.
---

# Adding a cleaner to Woosh

There are two kinds. Pick the right one — most requests are the first.

## Kind 1: a known path (90% of cases)

A fixed location like `~/Library/Caches/foo`. **Add one row to the table** in
`engine/src/domains/caches.rs`:

```rust
CacheDef {
    id: "cache:foo",                      // unique; grep the file to confirm
    group: Group::PackageManagers,        // or AppleMobile / Ides / MlCaches / System
    name: "Foo cache",
    detail: "What it is and why it is safe to remove, in one sentence.",
    spec: "~/Library/Caches/foo",         // ~ resolves to the discovered host home
    env_override: Some("FOO_CACHE_DIR"),  // if the tool supports relocating it
    risk: Risk::Safe,
    cleanup_hint: Some("foo cache clean"),
},
```

Nothing else is needed — the probe, sizing, de-duplication and UI grouping are
all table-driven.

**Choosing `risk` is the real decision:**

- `Safe` — regenerated automatically, no network, no meaningful wait. These get
  **pre-selected** for the user after a scan, so be strict.
- `Caution` — recoverable, but costs real time or bandwidth. Model weights, the
  pnpm store, `~/.m2`, anything that re-downloads.
- `Advisory` — reported, **never deleted**. For things needing their own tool
  (e.g. `/nix/store` needs `nix-collect-garbage`).

If `env_override` names a variable not already in the allowlist, extend the
`env_overrides_are_only_set_where_the_tool_supports_one` test.

## Kind 2: a project-local directory

Something found *inside* projects, like `node_modules` or `target/`. This runs in
the discovery walk, so it must be cheap.

1. **Detect from the listing you already have.** `detect()` in `engine/src/lib.rs`
   receives `(path, entries, depth)` — the directory's contents are already in
   hand, so testing for a marker file costs **zero extra syscalls**. Only stat
   when the listing cannot answer (see `venv::detect`, which checks `pyvenv.cfg`
   for free and only stats `bin/activate` when a `bin` child exists).

2. **Gate name-based matches.** A directory called `target` is only a Rust build
   if a sibling `Cargo.toml` exists. Without a gate you will offer to delete
   unrelated folders. Pay the stat only for the name that needs it.

3. **Return the specific variant** in `Verdict::Claim { tag, kind }`. `kind` is
   how the enrichment pass knows *which* marker matched — it cannot re-detect
   from a bare path, because the markers live in the listing the walker consumed.
   (A bug once labelled every conda env as a plain venv for exactly this reason.)

4. **Add a size floor** so trivial hits do not clutter the list.

For simple regenerable caches (`.mypy_cache`, `.turbo`, …) just add a row to
`REGENERABLE` in `engine/src/domains/artifacts.rs` — no detector change needed.

## Non-negotiables

- **Every finding carries a restore path.** `every_finding_carries_a_restore_path_or_is_advisory`
  enforces it. If you cannot say how to get it back, the risk is `Advisory`.
- **Never claim a directory that might be wanted output.** `dist/` and `build/`
  are pruned from the walk for speed but deliberately **never reported** — they
  are too often a real artifact someone needs. `build_output_is_not_classified_as_an_artifact`
  guards this.
- **Deleting is not your concern.** `reaper.rs` handles it, and `safety.rs` vets
  every path. Do not add deletion logic to a domain.
- **No subprocesses** unless the operation genuinely requires the tool's own
  bookkeeping. The only sanctioned one is `pip uninstall`, because it consumes
  `RECORD` files and console scripts; hand-deleting `dist-info` corrupts the
  install. Talk to daemons over their socket/HTTP API via `engine/src/http.rs`
  (see `domains/docker.rs` and `domains/models.rs`).

## Tests to add

Unit tests next to the code, plus a fixture branch when behaviour is structural:

1. Add the shape to `scripts/make-fixture.sh` with a comment saying what it pins.
2. Assert in `engine/tests/fixture.rs` — including the **negative** case. The
   valuable assertions are the ones proving you do *not* claim something
   (`cargo_target_requires_the_sibling_manifest`, `yarn_pnp_projects_are_not_reported`).
3. Beware substring collisions in path assertions: `"js-pnp"` matches `"js-pnpm"`.
   Match exact path components.
4. Use `tempfile::tempdir()` for scratch files — tests run in parallel and writing
   into `.fixture/` races the differential's file count.

Then: `make check` (Docker) and, if you touched `fastwalk/`, `make verify-macos`.

## Finding new targets worth adding

`docs/IMPLEMENTATION_PLAN.md` §4 lists researched targets with expected sizes.
The biggest real-world wins are usually iOS simulator runtimes (20–60 GB), Gradle
daemon logs, and per-tool browser downloads (Playwright, Puppeteer).
