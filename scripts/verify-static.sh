#!/usr/bin/env sh
# Verify a binary does not dynamically link libtdjson (release matrix "static" TDLib story).
# Full libc/OpenSSL static linking is optional and depends on TDLIB_LINK_SSL_STATIC / toolchain.

set -e
binary="${1:?usage: verify-static.sh /path/to/binary}"

if ! test -f "$binary"; then
  echo "verify-static: missing file: $binary" >&2
  exit 1
fi

# musl ldd is a script; glibc ldd works on gnu binaries
if ! command -v ldd >/dev/null 2>&1; then
  echo "verify-static: ldd not found; skipping dynamic checks" >&2
  exit 0
fi

out="$(ldd "$binary" 2>&1)" || true

case "$out" in
  *"not a dynamic executable"*)
    echo "verify-static: OK (fully static executable per ldd)"
    exit 0
    ;;
  *"statically linked"*)
    echo "verify-static: OK (ldd reports statically linked)"
    exit 0
    ;;
esac

if echo "$out" | grep -q '[Ll]ibtdjson'; then
  echo "verify-static: FAIL: binary still links libtdjson dynamically:" >&2
  echo "$out" >&2
  exit 1
fi

echo "verify-static: OK (no dynamic libtdjson; other DSOs may still be present)"
echo "$out"
