# Extends the cross musl image with a musl sysroot at /musl-local (zlib, OpenSSL, zstd) for TDLib CMake.
# Built with project root as context so COPY can see scripts/.
# cross-rs supplies CROSS_BASE_IMAGE when building this Dockerfile (see their custom image docs).
ARG CROSS_BASE_IMAGE
FROM $CROSS_BASE_IMAGE

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gperf perl make ca-certificates curl cmake \
    && rm -rf /var/lib/apt/lists/*

COPY scripts/cross-prebuild-musl.sh /tmp/cross-prebuild-musl.sh
RUN chmod +x /tmp/cross-prebuild-musl.sh && /tmp/cross-prebuild-musl.sh && rm -f /tmp/cross-prebuild-musl.sh
