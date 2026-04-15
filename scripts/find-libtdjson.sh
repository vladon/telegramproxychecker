#!/usr/bin/env bash
# Print libtdjson.so path: legacy build/.../out tree or stable target/tdlib-build-cache/.../tdlib-install/lib/.
set -euo pipefail
root="${1:?target dir e.g. target/release}"
f="$(find "$root/build" -path '*/tdlib-install/lib/libtdjson.so' -print -quit 2>/dev/null || true)"
if [[ -z "$f" || ! -f "$f" ]]; then
  f="$(find "$root/tdlib-build-cache" -path '*/tdlib-install/lib/libtdjson.so' -print -quit 2>/dev/null || true)"
fi
# TDLib install lives under workspace target/ (see build.rs), not under target/<triple>/release/.
if [[ -z "$f" || ! -f "$f" ]]; then
  _tdcache="${CARGO_TARGET_DIR:-target}/tdlib-build-cache"
  f="$(find "$_tdcache" -path '*/tdlib-install/lib/libtdjson.so' -print -quit 2>/dev/null || true)"
fi
if [[ -z "$f" || ! -f "$f" ]]; then
  exit 1
fi
printf '%s\n' "$f"
