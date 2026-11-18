---
name: macos-distribution
description: Share a macOS app with other people and get it past Gatekeeper — the "damaged" vs "could not verify / malware" warnings, quarantine, Developer ID signing, notarization with notarytool, stapling, and Full Disk Access / TCC behaviour across updates. Use when a user asks how to share a Mac app or DMG, why macOS refuses to open it, or how to do a signed release.
---

# Distributing a macOS app

## What to send

**The `.dmg`** — not the raw `.app` folder and nothing from `target/`. A good DMG
contains the app plus an `Applications` symlink for drag-to-install. State the
architecture requirement up front (e.g. "Apple Silicon, macOS 11+").

## The two Gatekeeper messages mean different things

Read the exact wording before suggesting anything.

| Message | Cause | Recipient can fix? |
|---|---|---|
| **"Apple could not verify 'X' is free of malware…"** | Valid signature, but **not notarized**. Normal for any build without an Apple Developer account. Not an actual malware detection. | Yes |
| **"X is damaged and can't be opened"** | The **code signature is invalid**. A packaging bug — usually signing after the DMG was built (see `macos-app-bundle`). | **No.** Rebuild. `xattr` won't help. |

Diagnose "damaged" by mounting the DMG and running `codesign --verify --strict
--deep` on the app inside it. If verification fails, fix the build; never coach
users around it.

## Getting past "could not verify" (unnotarized builds)

Give recipients one of these, once per machine:

**System Settings (no terminal):** try to open the app, dismiss the warning, then
**System Settings → Privacy & Security** → scroll to **Security** → **Open Anyway**
→ confirm. On macOS 15+ this is the only reliable route; the old right-click → Open
trick no longer works.

**Terminal:**
```bash
xattr -dr com.apple.quarantine /Applications/X.app
```

**Never** suggest `sudo spctl --master-disable` — it disables Gatekeeper for the
whole machine to work around one app.

## Removing the warning for everyone: Developer ID + notarization

Requires an Apple Developer Program membership and a **Developer ID Application**
certificate (not "Apple Development" / "Mac Development").

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx"   # app-specific password, appleid.apple.com
export APPLE_TEAM_ID="TEAMID"

# 1. Build — signing happens inside the build (Tauri reads these vars and will
#    also notarize automatically when APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are set)
APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" pnpm exec tauri build --target aarch64-apple-darwin

# 2. Sign the DMG itself, notarize, staple
codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"
xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl -a -vv --type install "$DMG"     # should now say "accepted"
```

- Notarization needs Hardened Runtime (`--options runtime`) and `--timestamp`.
- If `notarytool` reports "Invalid", fetch details:
  `xcrun notarytool log <submission-id> --apple-id … --team-id …`
- Keep credentials in the environment or a keychain profile
  (`notarytool store-credentials`). **Never commit them.**
- Script it so it refuses to run when any variable is missing, with a clear message
  about what to set — and let unsigned builds keep working without it.

## Tiers — pick one deliberately

| Tier | Signing | Recipient experience |
|---|---|---|
| 0 — local dev | ad-hoc (`-`) | Runs on the build machine |
| 1 — informal sharing | ad-hoc, verified | One-time "Open Anyway" / `xattr` step |
| 2 — real release | Developer ID + notarized + stapled | Opens normally, no warnings |

## Permissions (TCC) that recipients will hit

- **Desktop / Documents / Downloads** prompt automatically on first access, using
  the `NS*FolderUsageDescription` strings from Info.plist.
- **Full Disk Access cannot be requested programmatically** — no API exists. Detect
  it (try opening a TCC-protected file such as
  `~/Library/Application Support/com.apple.TCC/TCC.db`), deep-link the user to
  `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles`,
  and design the app to **degrade gracefully** without it rather than gating on it.
- **Grants reset on every update of an ad-hoc build.** TCC identifies an app by
  bundle id **plus code signature**; an ad-hoc signature's `cdhash` changes on every
  rebuild, so each new DMG is a different app to macOS. Warn users. A stable
  Developer ID signature is the real fix.

## What to tell a user who just shared the app

1. Which file to send (the DMG) and the architecture requirement.
2. The one-time "Open Anyway" step, with the exact menu path.
3. That permission grants will reset on updates until the build is notarized.
4. If anyone sees "damaged": stop, it's a build bug — rebuild and re-verify.
