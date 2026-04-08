#!/usr/bin/env sh
# Fail unless ldd reports a fully static executable (required for *-static release rows).
set -e
binary="${1:?usage: verify-fully-static.sh /path/to/binary}"

if ! test -f "$binary"; then
  echo "verify-fully-static: missing file: $binary" >&2
  exit 1
fi

if ! command -v ldd >/dev/null 2>&1; then
  echo "verify-fully-static: ldd not found" >&2
  exit 1
fi

out="$(ldd "$binary" 2>&1)" || true

case "$out" in
  *"not a dynamic executable"*)
    echo "verify-fully-static: OK"
    exit 0
    ;;
  *"statically linked"*)
    echo "verify-fully-static: OK"
    exit 0
    ;;
esac

echo "verify-fully-static: expected fully static binary; ldd output:" >&2
echo "$out" >&2
exit 1
