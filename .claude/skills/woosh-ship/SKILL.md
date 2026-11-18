---
name: woosh-ship
description: Ship Woosh to other people — produce the DMG, deal with Gatekeeper blocking it as unverified malware, code-sign, notarize, or explain Full Disk Access and TCC behaviour. Use when asked how to share the app, why macOS refuses to open it, or how to do a signed release.
---

# Shipping Woosh

## What to hand someone

**`target/aarch64-apple-darwin/release/bundle/dmg/Woosh_0.1.0_aarch64.dmg`** (~2.4 MB).

It contains `Woosh.app` and an `/Applications` symlink — the standard drag-to-install
layout. Build it with `make app` (which signs *during* the build, before the DMG
is assembled) and then `make dmg`, which verifies the signature of the app inside
the DMG and prints the recipient's instructions. `make dmg` fails rather than
shipping an unverifiable bundle.

Do **not** hand over the raw `.app` directory or the `target/` tree.

**Requirement to state up front: Apple Silicon (M1 or later), macOS 11+.** There is
no Intel build. An arm64-only `.app` on an Intel Mac does not fail politely — it
just will not launch.

## "Woosh is damaged and can't be opened"

**This is a packaging bug, not a Gatekeeper policy message, and `xattr` will not
fix it.** It means the app's code signature is invalid. On Apple Silicon every
binary must carry a valid signature; an invalid one plus quarantine produces
exactly this wording.

The cause we hit: **`tauri build` emits the `.app` and the `.dmg` in one pass**, so
signing `bundle/macos/Woosh.app` *afterwards* never reaches the DMG — it froze a
copy carrying only the Rust linker's placeholder (`flags=...linker-signed`,
`Identifier=woosh_app-<hash>`), which fails `codesign --verify` with *"code has no
resources but signature indicates they must be present"*.

Signing must happen **during** the build:

```bash
APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}" pnpm exec tauri build --target aarch64-apple-darwin
```

`make app` does this (defaulting to `-` for ad-hoc). Always diagnose by mounting
the DMG and checking the app **inside it** — not the sibling in `bundle/macos/`,
which is a different artifact:

```bash
hdiutil attach <dmg> -nobrowse -readonly -mountpoint /tmp/m
codesign -dv /tmp/m/Woosh.app              # expect Identifier=com.woosh.cleaner, flags=(adhoc,runtime)
codesign --verify --strict --deep /tmp/m/Woosh.app
hdiutil detach /tmp/m
```

`make dmg` performs exactly this check and exits non-zero if it fails.

## "Apple could not verify this app is free of malware"

This is Gatekeeper refusing an **unnotarized** download. It is not a malware
detection and nothing is actually wrong with the binary — macOS simply will not
vouch for an app that has not been signed with a Developer ID and notarized by
Apple, and it uses alarming wording for that.

Two ways past it, for the recipient:

**A — System Settings (no terminal).** Try to open Woosh, dismiss the warning,
then go to **System Settings → Privacy & Security**, scroll to the Security
section, and click **"Open Anyway"** next to the message about Woosh. Confirm.
This is the only reliable route on macOS 15+; the old right-click → Open trick no
longer works there.

**B — one command.** After dragging Woosh to Applications:

```bash
xattr -dr com.apple.quarantine /Applications/Woosh.app
```

Both are one-time, per machine.

**The real fix** is `make release` with an Apple Developer account ($99/yr): the
app is signed with a Developer ID, notarized by Apple and stapled, and it then
opens with no warning at all. See below.

Never tell someone to disable Gatekeeper globally (`spctl --master-disable`).
That weakens their machine far beyond this one app.

## Signed + notarized release

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="app-specific-password"   # appleid.apple.com → App-Specific Passwords
export APPLE_TEAM_ID="TEAMID"
make release
```

`scripts/release.sh` builds, signs the app and the DMG, submits to `notarytool
--wait`, staples the ticket and verifies. Credentials come from the environment
only — never commit them. `make doctor` reports which are set.

Notarization takes a few minutes and requires a **Developer ID Application**
certificate (not a "Mac Development" one).

## Permissions the recipient will meet

**Desktop / Documents / Downloads** prompt automatically on the first scan. The
reason text comes from `NS*FolderUsageDescription` in `app/src-tauri/Info.plist`
— these are mandatory, since those three are the engine's primary scan roots.
Without them macOS shows a blank-reason prompt or denies silently, and the user
gets an empty scan they cannot explain.

**Full Disk Access cannot be requested programmatically.** There is no API. Woosh
probes for it by trying to read a TCC-protected path, deep-links to the right
Settings pane, and treats it as an **enhancement, never a gate** — without it the
sweep still runs on everything reachable and reports how many paths were skipped.

**Tell people this about updates:** TCC identifies an app by bundle id *plus code
signature*. For an ad-hoc build that is the `cdhash`, which changes on every
rebuild — so each new DMG is, to macOS, a different app, and Full Disk Access
grants reset. A stable Developer ID signature is what actually fixes it.

## Bundle invariants

`app/src-tauri/entitlements.plist` deliberately has **no App Sandbox** — a tool
reading `/var/run/docker.sock` and `~/.cargo` cannot live in one. That rules out
Mac App Store distribution, which is the correct trade. Hardened Runtime stays
**on** (required for notarization) with `allow-jit` for WebKit's JS engine.

`NSAppleEventsUsageDescription` is **not** needed and must not be added: deletion
goes through the `trash` crate (`NSFileManager`), so Woosh never drives Finder via
AppleScript and never triggers that prompt.
