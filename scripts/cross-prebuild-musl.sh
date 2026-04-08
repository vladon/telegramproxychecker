#!/usr/bin/env bash
# Run inside cross-rs Docker (pre-build): musl-linked deps for TDLib CMake (OpenSSL, zlib, zstd).
# Install prefix is fixed so Cross.toml can pass OPENSSL_ROOT_DIR=/musl-local.
set -euo pipefail

PREFIX=/musl-local
export CC=x86_64-linux-musl-gcc
export CXX=x86_64-linux-musl-g++

rm -rf "$PREFIX"
mkdir -p "$PREFIX"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cd "$work"

# --- zlib (static; OpenSSL uses it) ---
curl -fL --retry 3 --connect-timeout 30 "https://zlib.net/zlib-1.3.1.tar.gz" | tar xz
cd zlib-1.3.1
./configure --prefix="$PREFIX" --static
make -j"$(nproc)"
make install
cd "$work"
rm -rf zlib-1.3.1

# --- OpenSSL (shared + static libs for musl) ---
curl -fL --retry 3 --connect-timeout 30 \
  "https://github.com/openssl/openssl/releases/download/openssl-3.3.2/openssl-3.3.2.tar.gz" | tar xz
cd openssl-3.3.2
./Configure linux-x86_64 \
  --prefix="$PREFIX" \
  --openssldir="$PREFIX/ssl" \
  shared zlib \
  -I"$PREFIX/include" \
  -L"$PREFIX/lib" \
  no-tests \
  "CC=$CC"
make -j"$(nproc)"
make install_sw
cd "$work"
rm -rf openssl-3.3.2

# --- libzstd (TDLib often links ZSTD) ---
curl -fL --retry 3 --connect-timeout 30 \
  "https://github.com/facebook/zstd/releases/download/v1.5.6/zstd-1.5.6.tar.gz" | tar xz
cd zstd-1.5.6/build/cmake
cmake -G "Unix Makefiles" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_C_COMPILER="$CC" \
  -DZSTD_BUILD_PROGRAMS=OFF \
  -DZSTD_BUILD_CONTRIB=OFF \
  -DZSTD_BUILD_TESTS=OFF \
  .
make -j"$(nproc)"
make install
cd "$work"
rm -rf zstd-1.5.6

echo "musl cross sysroot ready at $PREFIX"
