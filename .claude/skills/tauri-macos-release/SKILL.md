---
name: tauri-macos-release
description: >-
  Automated CI/CD, Apple Silicon native (aarch64-apple-darwin) DMG packaging,
  dual-path code signing (ad-hoc vs Apple Notarization), Gatekeeper quarantine
  handling, and 1-line curl installers for Tauri v2 & Rust macOS desktop applications.
  Activate when setting up or debugging GitHub Actions release workflows, packaging DMGs,
  or automating macOS releases.
---

# Tauri v2 & Rust macOS CI/CD Release Playbook

This skill provides the architectural specifications, CI/CD workflow templates, code signing strategies, and troubleshooting procedures for shipping native Apple Silicon macOS applications built with Tauri v2 and Rust.

---

## 1. Core Workflow Architecture (`.github/workflows/release.yml`)

### Runner Specification (Critical)
* **Runner Image:** Always use `runs-on: macos-latest`.
* **Breaking Change Notice:** GitHub officially retired `macos-14` runner images on November 2, 2026. Specifying `macos-14` triggers:
  `The configuration 'macos-14-arm64-us-default' is not supported.`
  `macos-latest` automatically targets modern Apple Silicon runners (M1/M2/M3).
* **Compilation Target:** Strictly target `aarch64-apple-darwin`.

### Trigger Policy
Trigger releases on semantic version tags or manual invocation:
```yaml
on:
  push:
    tags:
      - 'v*.*.*'
  workflow_dispatch:
```

### Dual-Path Signing Architecture
Tauri v2 supports two production paths:
1. **Ad-Hoc Signing (Free / Open Source):** Uses identity `-`. Works on all Apple Silicon Macs. Requires clearing the quarantine attribute on download.
2. **Apple Developer Notarization ($99/yr):** Signs with `Developer ID Application`, submits to Apple `notarytool`, and staples ticket to DMG. Zero Gatekeeper warnings.

#### Critical Gotcha: Empty Secret Environment Variables
If repository secrets are passed directly as environment variables:
```yaml
env:
  APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
```
When secrets are unset, bash exports `APPLE_CERTIFICATE=""`. Tauri CLI v2 detects non-null environment variables and attempts to import empty certificate data into the keychain, failing with:
`failed to run command security import: failed to import keychain certificate`.

**The Solution (Safe Environment Variable Unset):**
Before invoking `tauri build`, test the secret variables and explicitly `unset` them if empty:
```bash
if [ -z "${APPLE_CERTIFICATE:-}" ]; then
  echo "==> Apple Developer secrets absent. Using Ad-hoc signing (-)..."
  unset APPLE_CERTIFICATE
  unset APPLE_CERTIFICATE_PASSWORD
  unset APPLE_SIGNING_IDENTITY
  unset APPLE_ID
  unset APPLE_PASSWORD
  unset APPLE_TEAM_ID
  export APPLE_SIGNING_IDENTITY="-"
else
  echo "==> Apple Developer secrets detected. Configuring keychain..."
  # Decode and import certificate
fi
```

---

## 2. Standard Production Release Workflow Template

```yaml
name: Release Apple Silicon DMG

on:
  push:
    tags:
      - 'v*.*.*'
  workflow_dispatch:

permissions:
  contents: write

jobs:
  release:
    name: Build & Publish Apple Silicon DMG
    runs-on: macos-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4

      - name: Setup Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin
          components: clippy, rustfmt

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: 22

      - name: Setup pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 10
          run_install: false

      - name: Install Frontend Dependencies
        run: pnpm --prefix app install

      - name: Build Tauri Application & DMG
        env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
        run: |
          set -euo pipefail

          if [ -z "${APPLE_CERTIFICATE:-}" ]; then
            echo "==> No Apple Developer secrets configured. Unsetting variables for ad-hoc build..."
            unset APPLE_CERTIFICATE
            unset APPLE_CERTIFICATE_PASSWORD
            unset APPLE_SIGNING_IDENTITY
            unset APPLE_ID
            unset APPLE_PASSWORD
            unset APPLE_TEAM_ID
            export APPLE_SIGNING_IDENTITY="-"
          else
            echo "==> Apple Developer secrets found. Preparing keychain..."
            KEYCHAIN_PATH=$RUNNER_TEMP/build.keychain
            KEYCHAIN_PASSWORD=$(openssl rand -base64 32)
            security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
            security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
            security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

            CERT_PATH=$RUNNER_TEMP/cert.p12
            echo "$APPLE_CERTIFICATE" | base64 --decode > "$CERT_PATH"
            security import "$CERT_PATH" -k "$KEYCHAIN_PATH" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
            security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
            security list-keychains -d user -s "$KEYCHAIN_PATH" $(security list-keychains -d user | sed -e 's/^[[:space:]]*//' | tr -d '"')
          fi

          echo "==> Building Tauri release for aarch64-apple-darwin..."
          pnpm --prefix app exec tauri build --target aarch64-apple-darwin

      - name: Prepare Release Artifacts & Checksums
        id: artifacts
        run: |
          set -euo pipefail
          DMG_PATH=$(find target/aarch64-apple-darwin/release/bundle/dmg -name "*.dmg" | head -n 1)
          if [ -z "$DMG_PATH" ]; then
            echo "Error: DMG not found" >&2
            exit 1
          fi
          DMG_NAME=$(basename "$DMG_PATH")
          echo "Found DMG: $DMG_PATH"

          # Compute SHA-256 Checksum
          cd "$(dirname "$DMG_PATH")"
          shasum -a 256 "$DMG_NAME" > "${DMG_NAME}.sha256"

          echo "dmg_path=${DMG_PATH}" >> $GITHUB_OUTPUT
          echo "dmg_name=${DMG_NAME}" >> $GITHUB_OUTPUT
          echo "checksum_path=${DMG_PATH}.sha256" >> $GITHUB_OUTPUT

      - name: Publish GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            ${{ steps.artifacts.outputs.dmg_path }}
            ${{ steps.artifacts.outputs.checksum_path }}
          tag_name: ${{ github.ref_name }}
          name: Release ${{ github.ref_name }}
          draft: false
          prerelease: false
          generate_release_notes: true
```

---

## 3. macOS Gatekeeper Quarantine: Roots & Solutions

### The Root Cause
When files are downloaded via web browsers (Safari, Chrome, Firefox), macOS attaches an extended attribute:
`com.apple.quarantine`.
Gatekeeper enforces that binaries with quarantine must be notarized by Apple. Unnotarized binaries trigger:
* *"App is damaged and can't be opened. You should move it to the Trash."* OR
* *"App cannot be opened because Apple cannot check it for malicious software."*

### Resolution Options
1. **Automated 1-Line Installer (`install.sh`):**
   Downloads directly via curl (which does not attach `com.apple.quarantine`), mounts DMG, copies `.app` to `/Applications`, and executes `xattr -dr com.apple.quarantine /Applications/App.app`.
2. **Terminal One-Liner (For manual DMG downloaders):**
   ```bash
   xattr -dr com.apple.quarantine /Applications/App.app
   ```
3. **GUI Bypass:**
   System Settings -> Privacy & Security -> Security section -> Click **"Open Anyway"**.

---

## 4. The 1-Line Curl Installer Blueprint (`install.sh`)

Place an `install.sh` at the root of the repository:
```bash
#!/usr/bin/env bash
set -euo pipefail

REPO="${REPO_NAME:-your-username/your-repo}"
BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}${BOLD}==> Installing App for Apple Silicon...${NC}"

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
  echo -e "${RED}Error: App requires macOS on an Apple Silicon (M1/M2/M3/M4) Mac.${NC}" >&2
  exit 1
fi

RELEASE_JSON=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)
DMG_URL=$(echo "$RELEASE_JSON" | grep -o 'https://[^"]*aarch64\.dmg' | head -1 || true)

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo -e "${BLUE}==> Downloading DMG...${NC}"
curl -fsSL "$DMG_URL" -o "$TMP_DIR/app.dmg"

MOUNT_DIR=$(mktemp -d)
hdiutil attach "$TMP_DIR/app.dmg" -nobrowse -readonly -mountpoint "$MOUNT_DIR" -quiet

APP_SOURCE=$(find "$MOUNT_DIR" -maxdepth 1 -name "*.app" | head -1)
APP_NAME=$(basename "$APP_SOURCE")

echo -e "${BLUE}==> Installing to /Applications/${APP_NAME}...${NC}"
rm -rf "/Applications/${APP_NAME}"
cp -R "$APP_SOURCE" "/Applications/"
hdiutil detach "$MOUNT_DIR" -quiet

# Clear Gatekeeper quarantine
xattr -dr com.apple.quarantine "/Applications/${APP_NAME}" 2>/dev/null || true

echo -e "${GREEN}${BOLD}==> Successfully installed ${APP_NAME}!${NC}"
```

---

## 5. Release Checklist for Agents

When preparing a new release:
1. **Synchronize Semantic Versioning:**
   * Rust workspace: `Cargo.toml` (`[workspace.package] version = "X.Y.Z"`)
   * Tauri config: `app/src-tauri/tauri.conf.json` (`"version": "X.Y.Z"`)
   * Frontend package: `app/package.json` (`"version": "X.Y.Z"`)
   * UI About display: `app/src/App.svelte`
2. **Ensure Clean `.gitignore`:**
   * Never push local media, recordings (`*.mov`, `*.mp4`, `*.gif`, `docs/media/`), or `.dmg` files.
3. **Commit & Tag:**
   ```bash
   git commit -am "chore: bump version to X.Y.Z"
   git push origin main
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin vX.Y.Z
   ```
4. **Verify GitHub Release:**
   * Confirm GitHub Actions job completes on `macos-latest`.
   * Verify GitHub marks the new release as `Latest` so `/releases/latest` API serves the new version.
