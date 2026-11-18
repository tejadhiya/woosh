#!/usr/bin/env bash
# Woosh installer for macOS Apple Silicon
# Usage: curl -fsSL https://raw.githubusercontent.com/Tej-Adhiya/woosh/main/install.sh | bash

set -euo pipefail

REPO="${WOOSH_REPO:-Tej-Adhiya/woosh}"

BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}${BOLD}==> Installing Woosh for macOS Apple Silicon...${NC}"

# Check macOS
if [ "$(uname -s)" != "Darwin" ]; then
  echo -e "${RED}Error: Woosh is exclusively built for macOS.${NC}" >&2
  exit 1
fi

# Check Apple Silicon
ARCH="$(uname -m)"
if [ "$ARCH" != "arm64" ]; then
  echo -e "${RED}Error: Woosh requires an Apple Silicon Mac (M1, M2, M3, M4 or later).${NC}" >&2
  echo -e "Your architecture is: $ARCH (Intel Macs are not supported)." >&2
  exit 1
fi

# Fetch latest release DMG download URL from GitHub API
echo -e "${BLUE}==> Finding latest release from ${REPO}...${NC}"
RELEASE_JSON=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)

if [ -z "$RELEASE_JSON" ]; then
  RELEASE_JSON=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases" 2>/dev/null | grep -A 20 '"assets"' | head -25 || true)
fi

DMG_URL=$(echo "$RELEASE_JSON" | grep -o 'https://[^"]*aarch64\.dmg' | head -1 || true)

if [ -z "$DMG_URL" ]; then
  echo -e "${RED}Error: Could not locate an Apple Silicon release DMG for ${REPO}.${NC}" >&2
  echo -e "Please check https://github.com/${REPO}/releases manually." >&2
  exit 1
fi

DMG_NAME="$(basename "$DMG_URL")"
echo -e "${GREEN}✓ Found release:${NC} $DMG_NAME"

# Create temporary workspace
TMP_DIR=$(mktemp -d /tmp/woosh-install.XXXXXX)
MNT_DIR=$(mktemp -d /tmp/woosh-mnt.XXXXXX)

cleanup() {
  hdiutil detach "$MNT_DIR" >/dev/null 2>&1 || true
  rm -rf "$MNT_DIR" "$TMP_DIR" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo -e "${BLUE}==> Downloading $DMG_NAME...${NC}"
curl -fL --progress-bar -o "$TMP_DIR/Woosh.dmg" "$DMG_URL"

echo -e "${BLUE}==> Mounting disk image...${NC}"
hdiutil attach "$TMP_DIR/Woosh.dmg" -nobrowse -readonly -mountpoint "$MNT_DIR" >/dev/null

if [ ! -d "$MNT_DIR/Woosh.app" ]; then
  echo -e "${RED}Error: DMG did not contain Woosh.app${NC}" >&2
  exit 1
fi

echo -e "${BLUE}==> Installing to /Applications/Woosh.app...${NC}"
if [ -d "/Applications/Woosh.app" ]; then
  echo -e "${YELLOW}Replacing previous installation...${NC}"
  rm -rf "/Applications/Woosh.app"
fi

cp -R "$MNT_DIR/Woosh.app" "/Applications/"

echo -e "${BLUE}==> Detaching disk image...${NC}"
hdiutil detach "$MNT_DIR" >/dev/null 2>&1 || true

echo -e "${BLUE}==> Removing Gatekeeper quarantine attribute...${NC}"
xattr -dr com.apple.quarantine "/Applications/Woosh.app" 2>/dev/null || true

# Verify code signature
if codesign --verify --strict "/Applications/Woosh.app" 2>/dev/null; then
  echo -e "${GREEN}✓ Code signature verified.${NC}"
fi

echo
echo -e "${GREEN}${BOLD}🎉 Woosh has been successfully installed to /Applications/Woosh.app!${NC}"
echo -e "You can launch it via Spotlight (${BOLD}Cmd + Space${NC} → type ${BOLD}Woosh${NC}) or run:"
echo -e "  ${BOLD}open /Applications/Woosh.app${NC}"
echo
