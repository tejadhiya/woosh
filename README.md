# Woosh

**Developer storage reclaimer for Apple Silicon.** Tauri v2 shell, pure-Rust engine.

Finds the build artifacts, dependency trees, model weights, container layers and
toolchain caches that quietly eat a dev machine's disk — then removes them to the
Finder Trash, after writing down how to get each one back.

> On the machine it was built on: **81.6 GB across 95 items, in 6.7 seconds.**

| | |
|---|---|
| **Requires** | Apple Silicon (M1 or later), macOS 11+ |
| **Share** | `Woosh_0.1.1_aarch64.dmg` (~2.5 MB) |
| **Build** | `make ci` (Docker) → `make app` (host) |
| **Docs** | [Developer guide](docs/DEVELOPMENT.md) · [Design + measurements](docs/IMPLEMENTATION_PLAN.md) · [Agent context](CLAUDE.md) |

---

## What it finds

| Domain | Covers |
|---|---|
| **Virtual environments** | Any folder name — detection is structural (`pyvenv.cfg`, `bin/activate`, `conda-meta`), so `mynenv` and `custom-env` are found too. Reads packages straight from `dist-info/METADATA`, no subprocess. |
| **node_modules** | Per-project, with the right reinstall command from the lockfile (pnpm / bun / yarn / npm). Yarn PnP projects are correctly skipped. |
| **Build artifacts** | Rust `target/` (gated on a sibling `Cargo.toml`), `__pycache__`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`, `.next`, `.turbo`, `.svelte-kit`, `.nuxt`, `.angular`, `.vite`, `.tox`, `.nx` |
| **AI & ML** | Hugging Face Hub, Ollama (live API — exact sizes, not parsed stdout), LM Studio, PyTorch Hub, Whisper, Triton, NLTK, Keras, InsightFace, DeepFace, EasyOCR |
| **Docker** | Unused images, build cache, stopped containers, dangling volumes — via `/var/run/docker.sock`. Plus `Docker.raw` sizing that handles sparseness correctly. |
| **Global Python** | Every interpreter (Homebrew, python.org, pyenv, Apple SIP), with essential tooling separated from third-party clutter and a backup written before any prune. |
| **~45 toolchain caches** | npm, pip, Homebrew, Gradle (incl. daemon logs), Cargo registry + git checkouts, Yarn Berry, pnpm, Bun, Deno, TurboRepo, Maven, Go, Bazel, uv, conda, CocoaPods, SwiftPM, Xcode DerivedData / Archives / DeviceSupport, **iOS simulator runtimes**, JetBrains, VS Code, Cursor, Electron, node-gyp, Playwright, Puppeteer |

Plus a live telemetry feed showing discovery paths, throughput and freed bytes as
it works.

## What it will not do

- **Never deletes without a way back.** Every finding carries a restore command. A
  virtual environment is not removed until `req-inplace.txt` — every package and
  version, plus the three lines that rebuild it — is written into the project root.
  **If that write fails, the deletion is abandoned.**
- **Never silently escalates.** A failed move-to-Trash is a failed receipt, not a
  permanent delete. Irreversible deletion is opt-in and makes you type the word.
- **Never offers your build output.** `dist/` and `build/` are skipped for speed
  but deliberately never reported — too often something you actually want.
- **Never over-promises space.** `pnpm` hardlinks and APFS clones mean the same
  bytes appear under many paths. Woosh counts each exactly once and shows shared
  bytes separately, so the total is what you really get back.

Protected paths, your home directory root, mount points, network volumes and
anything reached through a symlink out of the permitted tree are all refused — and
re-checked immediately before deletion, not just when they were found.

---

## Install

### Quick Install (One-Liner)

Open Terminal and run:

```bash
curl -fsSL https://raw.githubusercontent.com/tejadhiya/woosh/main/install.sh | bash
```

This automatically downloads the latest Apple Silicon DMG from GitHub Releases, installs `Woosh.app` to `/Applications`, and clears macOS quarantine.

### Manual Install

1. Download `Woosh_0.1.1_aarch64.dmg` (or latest) from [GitHub Releases](https://github.com/tejadhiya/woosh/releases).
2. Open the DMG and drag **Woosh** to your `/Applications` folder.

### If macOS blocks it

macOS has **two different messages here, with two different causes.** Read which
one you got:

**"Apple could not verify Woosh is free of malware"** — normal. This is Gatekeeper
refusing an *unnotarized* download, not a malware detection. Nothing is wrong with
the binary; macOS just will not vouch for an app that has not been notarized by
Apple, and it words that alarmingly. Fix below.

**"Woosh is damaged and can't be opened"** — *not* normal, and **clearing
quarantine will not help.** That message means the code signature itself is
invalid, which is a packaging bug, not something you can work around. Ask for a
rebuilt DMG. (`make dmg` now refuses to pass a DMG whose signature does not
verify, so this should not reach you again.)

For the first message, either:

- **System Settings → Privacy & Security**, scroll to Security, click **Open
  Anyway** next to the Woosh message. (On macOS 15+ this is the only reliable
  route; right-click → Open no longer works.)

- Or run this once:
  ```bash
  xattr -dr com.apple.quarantine /Applications/Woosh.app
  ```

Never disable Gatekeeper globally — that weakens the whole machine for one app.

With an Apple Developer account, `make release` notarizes the build and the
warning disappears for everyone.

### Permissions

Desktop, Documents and Downloads prompt on first scan — that covers most findings.

Full Disk Access is an **enhancement, never a gate**: without it the sweep still
runs on everything reachable and reports how many paths it could not read, with a
link to the right Settings pane. macOS provides no API to request it, so the app
can only point you there.

> Heads up: Full Disk Access grants **reset on every update** of an unsigned
> build. macOS identifies an app by bundle id *plus code signature*, and an ad-hoc
> signature changes on every rebuild — so each new DMG looks like a different app.
> A Developer ID signature fixes this permanently.

---

## Build

Every build and verification step runs in Docker. Exactly one step runs on the
host — bundling the `.app`, which needs `WKWebView` and `codesign` and so cannot
happen in a Linux container. Nothing is installed on your Mac.

```bash
make doctor   # toolchain + signing readiness (read-only)
make ci       # fmt, clippy, tests, workspace typecheck, frontend, bench — all Docker
make app      # bundle Woosh.app   (host)
make dmg      # sign + report the shareable DMG (host)
```

`make help` lists everything. Full workflows, testing notes and the traps worth
knowing: **[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)**.

### Headless

```bash
cargo run --release --example headless          # real sweep, no GUI, deletes nothing
cargo run --release --example headless -- --deep
```

---

## Performance

~500k entries/s on ordinary trees, ~300k/s on `node_modules` (which average ~5
entries per directory, so the syscall cost per entry is ~20× higher). Against
`du -sk` on identical input: **4.925 s → 1.085 s — 4.5× faster**, and unlike
per-directory `du` it de-duplicates hardlinks.

The engine uses `getattrlistbulk(2)`, which returns name, type, inode and
**allocated size** for a whole batch of entries in one syscall — so sizing is free
and sparse files are correct without a special case.

[IMPLEMENTATION_PLAN.md §1](docs/IMPLEMENTATION_PLAN.md) has the full breakdown,
including the optimizations that were implemented, measured, and reverted when the
numbers did not support them.

## Layout

```
engine/          pure Rust. No Tauri dependency, so the suite runs in Linux Docker.
  fastwalk/      getattrlistbulk(2) on macOS; std::fs elsewhere. One trait, proven equal.
  sweep.rs       depth-bounded discovery + pure-sum measurement, both rayon
  dedup.rs       (dev, ino) set → exclusive vs shared bytes
  safety.rs      deletion boundary
  reaper.rs      trash-backed deletion, receipts, audit log
  telemetry.rs   60 Hz frame coalescer + log voice
  domains/       venv · node · global_python · models · docker · caches · artifacts
app/             Svelte 5 frontend + the thin Tauri shell
docker/          build + verification images
.claude/skills/  agent skills: woosh-build · woosh-add-cleaner · woosh-ship
```

Ported from a Python prototype, which remains the reference for domain logic.
Deviations from it are individually justified in
[IMPLEMENTATION_PLAN.md §3](docs/IMPLEMENTATION_PLAN.md).

---

## Maintainer & Contact

Woosh is built and maintained by **Tejadhiya**.

- **Email:** [tejadhiya2@gmail.com](mailto:tejadhiya2@gmail.com)
- **GitHub:** [https://github.com/tejadhiya/woosh](https://github.com/tejadhiya/woosh)

Feel free to open an issue or reach out directly for questions, feedback, or ideas!

---

## License

Woosh is dual-licensed under either:

- **MIT license** ([LICENSE-MIT](LICENSE-MIT) or [opensource.org/licenses/MIT](https://opensource.org/licenses/MIT))
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or [apache.org/licenses/LICENSE-2.0](https://www.apache.org/licenses/LICENSE-2.0))

at your option.

