#!/usr/bin/env bash
# Verify and report the shareable DMG.
#
# This script deliberately inspects the app *inside the DMG*, not the sibling copy
# in bundle/macos/. Those are two different artifacts: `tauri build` emits the .app
# and the .dmg in one pass, so anything done to bundle/macos afterwards never
# reaches the DMG. Checking the sibling once hid a DMG containing a linker-signed
# binary that macOS reported to recipients as "Woosh is damaged and can't be
# opened" -- which no amount of `xattr` would have fixed.
#
# Signing happens during `make app` (APPLE_SIGNING_IDENTITY, default "-"), before
# the DMG is built. Nothing is signed here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET=aarch64-apple-darwin
DMG=$(ls -t "$ROOT/target/$TARGET/release/bundle/dmg"/*.dmg 2>/dev/null | head -1 || true)

[ -n "$DMG" ] || { echo "no DMG found — run 'make app' first" >&2; exit 1; }

MNT=$(mktemp -d /tmp/woosh-dmg.XXXXXX)
cleanup() { hdiutil detach "$MNT" >/dev/null 2>&1 || true; rmdir "$MNT" 2>/dev/null || true; }
trap cleanup EXIT

hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MNT" >/dev/null
APP="$MNT/Woosh.app"
[ -d "$APP" ] || { echo "DMG does not contain Woosh.app" >&2; exit 1; }

echo "DMG: $DMG  ($(du -h "$DMG" | cut -f1))"
echo
echo "signature of the app inside it:"
codesign -dv "$APP" 2>&1 | grep -E "Identifier|CodeDirectory|Signature=" | sed 's/^/  /'

echo
if codesign --verify --strict --deep "$APP" 2>/dev/null; then
  echo "  signature valid ✓"
else
  cat >&2 <<'BAD'

  SIGNATURE INVALID — do not ship this.

  Recipients will see "Woosh is damaged and can't be opened", and clearing the
  quarantine flag will NOT help: the signature itself is broken.

  Usually this means the app was signed after the DMG was built. Signing must
  happen during `make app` via APPLE_SIGNING_IDENTITY, not afterwards.
BAD
  exit 1
fi

if codesign -d --entitlements - "$APP" 2>/dev/null | grep -q allow-jit; then
  echo "  hardened runtime entitlements present ✓"
fi

echo
if spctl -a -t exec "$APP" >/dev/null 2>&1; then
  echo "Gatekeeper: accepted — opens normally on any Apple Silicon Mac."
else
  cat <<'NOTE'
Gatekeeper: unnotarized (expected without an Apple Developer account).

The signature is valid, so recipients get the ordinary "Apple could not verify"
prompt — not a "damaged" error. To open it the first time, either:

  System Settings -> Privacy & Security -> scroll to Security -> "Open Anyway"

or, once, after dragging Woosh to Applications:

  xattr -dr com.apple.quarantine /Applications/Woosh.app

`make release` with a Developer ID removes this step for everyone.
NOTE
fi
