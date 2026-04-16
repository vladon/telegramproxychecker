#!/usr/bin/env bash
# Exit 0 if vendored TDLib install tree exists for this CI profile (see build.rs paths).
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT"
TD="$(sed -n 's/^const TD_COMMIT: &str = "\(.*\)".*/\1/p' build.rs | head -1)"
BASE="target/tdlib-build-cache/cmake/${TD}"
case "${1:?gnu or musl-static}" in
  gnu)
    test -d "$BASE/x86_64-unknown-linux-gnu/unix-static/tdlib-install/lib"
    ;;
  musl-static)
    test -d "$BASE/x86_64-unknown-linux-musl/musl-static/tdlib-install/lib"
    ;;
  *)
    echo "usage: $0 gnu|musl-static" >&2
    exit 2
    ;;
esac
