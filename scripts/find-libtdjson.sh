#!/usr/bin/env bash
# Print libtdjson.so path from target/<profile>/build/.../tdlib-install/lib/ (musl shared layout).
set -euo pipefail
root="${1:?target dir e.g. target/release}"
f="$(find "$root/build" -path '*/tdlib-install/lib/libtdjson.so' -print -quit 2>/dev/null || true)"
if [[ -z "$f" || ! -f "$f" ]]; then
  exit 1
fi
printf '%s\n' "$f"
