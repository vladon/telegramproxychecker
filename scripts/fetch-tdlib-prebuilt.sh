#!/usr/bin/env bash
# Download optional TDLib tree from GitHub Release "tdlib-prebuilt" (public repo, no token).
# Usage: fetch-tdlib-prebuilt.sh <owner/repo> <gnu|musl-static> <TD_COMMIT>
# Writes hit=true|false to GITHUB_OUTPUT when set.
set -uo pipefail

REPO="${1:?owner/repo}"
GROUP="${2:?gnu or musl-static}"
TD="${3:?TD_COMMIT}"

URL="https://github.com/${REPO}/releases/download/tdlib-prebuilt/tdlib-${GROUP}-${TD}.tar.zst"
mkdir -p target

hit=false
if curl -fsSL --retry 2 --connect-timeout 45 "$URL" -o /tmp/tdlib-prebuilt.tar.zst; then
  if tar -I zstd -xf /tmp/tdlib-prebuilt.tar.zst -C target 2>/dev/null; then
    hit=true
  fi
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "hit=${hit}" >>"$GITHUB_OUTPUT"
fi
