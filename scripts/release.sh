#!/usr/bin/env bash
# Developer ID signed + notarized + stapled DMG (plan 8.1, Tier 2).
# Reads credentials from the environment so nothing secret lands in the repo.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET=aarch64-apple-darwin

missing=0
for v in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  if [ -z "${!v:-}" ]; then echo "missing \$$v" >&2; missing=1; fi
done
if [ "$missing" = 1 ]; then
  cat >&2 <<'NOTE'

Notarization needs an Apple Developer account. Set:

  export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
  export APPLE_ID="you@example.com"
  export APPLE_PASSWORD="app-specific-password"   # appleid.apple.com -> App-Specific Passwords
  export APPLE_TEAM_ID="TEAMID"

Until then `make dmg` produces a shareable (unnotarized) DMG — see its output
for the one-line step recipients need.
NOTE
  exit 1
fi

echo "==> building"
( cd "$ROOT/app" && pnpm install && pnpm exec tauri build --target "$TARGET" )

APP="$ROOT/target/$TARGET/release/bundle/macos/Woosh.app"
echo "==> signing with Developer ID"
codesign --force --deep --options runtime --timestamp \
  --entitlements "$ROOT/app/src-tauri/entitlements.plist" \
  --sign "$APPLE_SIGNING_IDENTITY" "$APP"
codesign --verify --strict --verbose=2 "$APP"

DMG=$(ls -t "$ROOT/target/$TARGET/release/bundle/dmg"/*.dmg | head -1)
echo "==> signing the DMG itself"
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"

echo "==> notarizing (this takes a few minutes)"
xcrun notarytool submit "$DMG" \
  --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" \
  --wait

echo "==> stapling"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl -a -vv --type install "$DMG" || true

echo
echo "release ready: $DMG"
echo "This opens with no warnings on any Apple Silicon Mac."
