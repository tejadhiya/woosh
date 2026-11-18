---
name: macos-app-bundle
description: Produce a correct macOS .app and DMG from a Tauri v2 project — bundle config, Info.plist, entitlements, icons, code signing, and verifying the artifact you actually ship. Use when asked to build a Mac app or DMG, add macOS permissions strings, set entitlements, generate app icons, or diagnose a bundle that won't launch.
---

# Building a correct macOS .app and DMG

## Rule 1: sign DURING the build, never after

`tauri build` emits the `.app` **and** the `.dmg` in a single pass. Anything you do
to `bundle/macos/X.app` afterwards **never reaches the DMG** — the DMG already
froze its own copy.

Signing afterwards ships a DMG containing a binary with only the Rust linker's
placeholder signature:

```
Identifier=myapp-abb8b67b066efaf2          <- linker-generated, not your bundle id
flags=0x20002(adhoc,linker-signed)         <- placeholder, not a real signature
codesign --verify: "code has no resources but signature indicates they must be present"
```

Recipients see **"X is damaged and can't be opened"**, and removing quarantine
does **not** fix it — the signature itself is invalid.

Export the identity into the build so Tauri signs before assembling the DMG:

```bash
APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}" pnpm exec tauri build --target aarch64-apple-darwin
```

`-` means ad-hoc. With a real `Developer ID Application: …` identity the same line
produces a properly signed build.

## Rule 2: verify the artifact you ship, not its sibling

The `.app` in `bundle/macos/` and the `.app` inside the DMG are different files.
Always mount the DMG and check the one inside it:

```bash
MNT=$(mktemp -d)
hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MNT"
codesign -dv "$MNT/MyApp.app"                         # expect your bundle id, flags=(adhoc,runtime)
codesign --verify --strict --deep "$MNT/MyApp.app"    # MUST pass
codesign -d --entitlements - "$MNT/MyApp.app"         # entitlements present?
hdiutil detach "$MNT"
```

Make the packaging script **exit non-zero** when verification fails. A pipeline
that checks the wrong copy will pass a broken release.

To simulate what a recipient experiences:

```bash
cp -R "$MNT/MyApp.app" /tmp/t/
xattr -w com.apple.quarantine "0081;00000000;Safari;" /tmp/t/MyApp.app
codesign --verify --strict --deep /tmp/t/MyApp.app   # still valid under quarantine?
xattr -dr com.apple.quarantine /tmp/t/MyApp.app && open /tmp/t/MyApp.app
```

## tauri.conf.json essentials

```json
{
  "productName": "MyApp",
  "identifier": "com.example.myapp",
  "build": { "frontendDist": "../dist", "devUrl": "http://localhost:1420",
             "beforeBuildCommand": "pnpm run build" },
  "app": { "windows": [{ "label": "main", "title": "MyApp", "width": 1180, "height": 780 }] },
  "bundle": {
    "active": true,
    "targets": ["app", "dmg"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns"],
    "macOS": { "minimumSystemVersion": "11.0", "entitlements": "entitlements.plist" }
  }
}
```

- **Do not end the identifier in `.app`** (e.g. `com.foo.app`) — Tauri warns it
  collides with the bundle extension. Fix it before first release: the identifier
  is part of how macOS identifies the app for permissions, so changing it later
  resets every user's grants.
- Give the window an explicit `"label"` so capability files can target it.

## Info.plist — merged, not replaced

Put a partial `src-tauri/Info.plist` next to `tauri.conf.json`; Tauri merges it
into the generated one.

```xml
<key>NSDesktopFolderUsageDescription</key>
<string>MyApp reads your Desktop to find …</string>
<key>NSDocumentsFolderUsageDescription</key>
<string>…</string>
<key>NSDownloadsFolderUsageDescription</key>
<string>…</string>
<key>LSArchitecturePriority</key>
<array><string>arm64</string></array>
```

**If the app reads Desktop/Documents/Downloads, these strings are mandatory.**
macOS shows them in the consent prompt. Without them the user gets a blank-reason
prompt or a silent denial — and then an app that mysteriously finds nothing.

## Entitlements

```xml
<key>com.apple.security.cs.allow-jit</key><true/>                  <!-- WebKit's JS engine -->
<key>com.apple.security.cs.disable-library-validation</key><true/>
```

- **Hardened Runtime on** (`--options runtime`) — required for notarization.
- **App Sandbox off** only if the app genuinely needs broad filesystem or socket
  access (system tools, dev tools). This rules out the Mac App Store; decide that
  consciously.
- Don't add `NSAppleEventsUsageDescription` unless you actually script other apps.
  Prefer native APIs (e.g. move to Trash via `NSFileManager` / the Rust `trash`
  crate rather than `osascript` + Finder) and the prompt never appears.

## Architecture

- **Apple Silicon only:** `--target aarch64-apple-darwin` plus
  `LSArchitecturePriority = [arm64]`. An arm64 app on an Intel Mac does not fail
  politely — it just won't launch — so say "Apple Silicon required" on the DMG/README.
- **Universal:** `--target universal-apple-darwin`. Any unsafe struct-layout code
  must then be tested on both slices.

## Icons

`tauri::generate_context!()` is a proc macro and reads icons **at compile time**,
so they must exist before the first build. Generate them reproducibly (e.g. a PIL
script rendering at 4× then Lanczos-downsampling), then build `.icns`:

```bash
mkdir icon.iconset
for s in 16 32 128 256 512; do
  sips -z $s $s master.png --out icon.iconset/icon_${s}x${s}.png
  sips -z $((s*2)) $((s*2)) master.png --out icon.iconset/icon_${s}x${s}@2x.png
done
iconutil -c icns icon.iconset -o icons/icon.icns
```

Check the result at 32 px — that's what the Dock and Finder lists show.

## Inspecting the bundle without breaking it

**`plutil -extract KEY json file.plist` rewrites the file in place.** It will
silently replace your whole `Info.plist` with the extracted value. Always pass `-o -`:

```bash
P=MyApp.app/Contents/Info.plist
plutil -extract CFBundleIdentifier raw -o - "$P"
plutil -extract LSArchitecturePriority json -o - "$P"
plutil -lint "$P"
```

## Checklist before handing anyone a DMG

1. Built with `APPLE_SIGNING_IDENTITY` set during the build.
2. App **inside the DMG** passes `codesign --verify --strict --deep`.
3. Bundle id and `flags=(…,runtime)` correct; entitlements present.
4. Info.plist has every usage string for every protected folder you touch.
5. Survives the quarantine simulation above and launches.
