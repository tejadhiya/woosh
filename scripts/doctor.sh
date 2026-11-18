#!/usr/bin/env bash
# Report host toolchain and signing readiness. Read-only; installs nothing.
set -uo pipefail

ok()   { printf "  \033[32m✓\033[0m %-26s %s\n" "$1" "${2:-}"; }
warn() { printf "  \033[33m!\033[0m %-26s %s\n" "$1" "${2:-}"; }
bad()  { printf "  \033[31m✗\033[0m %-26s %s\n" "$1" "${2:-}"; }

echo
echo "Woosh doctor"
echo "────────────────────────────────────────────────────────────"
echo "host (bundling only — everything else runs in Docker)"

[ "$(uname -m)" = "arm64" ] && ok "architecture" "arm64" \
  || bad "architecture" "$(uname -m) — Woosh targets Apple Silicon only"

sw_vers -productVersion >/dev/null 2>&1 \
  && ok "macOS" "$(sw_vers -productVersion)" || bad "macOS" "not macOS"

command -v cargo >/dev/null && ok "cargo" "$(cargo --version | cut -d' ' -f2)" \
  || bad "cargo" "missing — needed only for the final bundle step"
command -v pnpm  >/dev/null && ok "pnpm" "$(pnpm --version)" || bad "pnpm" "missing"
xcrun --show-sdk-path >/dev/null 2>&1 \
  && ok "macOS SDK" "$(xcrun --show-sdk-version 2>/dev/null)" \
  || bad "macOS SDK" "run: xcode-select --install"

rustup target list --installed 2>/dev/null | grep -q aarch64-apple-darwin \
  && ok "rust target" "aarch64-apple-darwin" \
  || warn "rust target" "run: rustup target add aarch64-apple-darwin"

echo
echo "docker (build + verification)"
if docker info >/dev/null 2>&1; then
  ok "docker" "$(docker version --format '{{.Server.Version}}' 2>/dev/null)"
else
  bad "docker" "daemon not reachable — start Docker Desktop"
fi

echo
echo "signing (plan §8.1)"
if security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
  ok "Developer ID" "found — \`make release\` can notarize"
else
  warn "Developer ID" "none — builds are ad-hoc signed (Tier 0/1)"
fi
for v in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  [ -n "${!v:-}" ] && ok "$v" "set" || warn "$v" "unset"
done

echo
echo "runtime integrations (optional — absence is not an error)"
[ -S /var/run/docker.sock ] && ok "docker socket" "/var/run/docker.sock" \
  || warn "docker socket" "not present; Docker domain will report VM disk only"
curl -s --max-time 1 http://127.0.0.1:11434/api/tags >/dev/null 2>&1 \
  && ok "ollama api" "127.0.0.1:11434" \
  || warn "ollama api" "not running; will fall back to sizing blobs"
echo
