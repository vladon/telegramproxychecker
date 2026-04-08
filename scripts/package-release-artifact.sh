#!/usr/bin/env bash
# Package a release tarball + sha256. Usage:
#   package-release-artifact.sh <output_dir> <package_basename> <binary_path> [libtdjson.so ...]
# <package_basename> must not include .tar.gz (e.g. tg-proxy-check-1.2.3-linux-x86_64-gnu).
set -euo pipefail

out_dir="${1:?output dir}"
base="${2:?package basename}"
binary="${3:?binary path}"
shift 3 || true

mkdir -p "$out_dir"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

cp "$binary" "$stage/tg-proxy-check"
chmod +x "$stage/tg-proxy-check"

for f in "$@"; do
  if [[ -n "$f" && -f "$f" ]]; then
    cp "$f" "$stage/"
  fi
done

archive="${out_dir}/${base}.tar.gz"
tar -czf "$archive" -C "$stage" .
(
  cd "$out_dir"
  sha256sum "${base}.tar.gz" | tee "${base}.tar.gz.sha256" >/dev/null
)

echo "packaged: $archive"
