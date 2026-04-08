# Extends the cross musl image with a musl sysroot at /musl-local (zlib, OpenSSL, zstd) for TDLib CMake.
# Built with project root as context so COPY can see scripts/.
# cross-rs passes CROSS_BASE_IMAGE when building; default allows local `docker build` smoke tests.
ARG CROSS_BASE_IMAGE=ghcr.io/cross-rs/cross:x86_64-unknown-linux-musl
FROM $CROSS_BASE_IMAGE

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gperf perl make ca-certificates curl cmake \
    && rm -rf /var/lib/apt/lists/*

COPY scripts/cross-prebuild-musl.sh /tmp/cross-prebuild-musl.sh
RUN chmod +x /tmp/cross-prebuild-musl.sh && /tmp/cross-prebuild-musl.sh && rm -f /tmp/cross-prebuild-musl.sh
