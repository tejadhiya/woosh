# Portable skills: macOS apps via Tauri + Docker

Three agent skills capturing how Woosh was built and shipped, written to be
reused on any Tauri v2 + Rust project targeting macOS.

| Skill | Use it for |
|---|---|
| `tauri-docker-build` | Docker/host split, crate layout, compose + Makefile, build failures |
| `macos-app-bundle` | `.app` + DMG config, Info.plist, entitlements, icons, signing, verification |
| `macos-distribution` | Sharing, Gatekeeper messages, notarization, TCC / Full Disk Access |

## Install

Per project (committed with the repo):
```bash
cp -R skills/* /path/to/other-project/.claude/skills/
```

For every project on a machine:
```bash
mkdir -p ~/.claude/skills && cp -R skills/* ~/.claude/skills/
```

Each skill is a folder containing `SKILL.md`; the `description` in its frontmatter
is what tells an agent when to load it.
