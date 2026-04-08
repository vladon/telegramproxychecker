# Release matrix for tg-proxy-check (Linux x86_64).
# Each target sets TDLIB_BUILD_VARIANT so TDLib CMake output never mixes gnu/musl/v3/static.
#
# Prerequisites: rustup targets, C++ toolchain, CMake, OpenSSL dev, zlib, gperf.
# Musl: rustup target add x86_64-unknown-linux-musl and a linker that can link musl (e.g. musl-tools).

DIST ?= dist
CARGO ?= cargo
RUST_RELEASE := --release

.PHONY: release-all \
	build-gnu build-musl build-gnu-v3 build-musl-v3 \
	build-musl-static build-musl-v3-static \
	verify-musl-static verify-musl-v3-static \
	clean-dist

release-all: build-gnu build-musl build-gnu-v3 build-musl-v3 build-musl-static build-musl-v3-static

clean-dist:
	rm -rf $(DIST)

$(DIST):
	mkdir -p $(DIST)

# GNU glibc, generic x86-64 ISA
build-gnu: $(DIST)
	TDLIB_BUILD_VARIANT=linux-x86_64-gnu \
		$(CARGO) build $(RUST_RELEASE) --target x86_64-unknown-linux-gnu
	cp -f target/x86_64-unknown-linux-gnu/release/tg-proxy-check \
		$(DIST)/tg-proxy-check-linux-x86_64-gnu

# musl, shared tdjson (no system libtdjson; rpath to build tree copy at link time — runtime needs bundled .so next to binary or RPATH baked in)
build-musl: $(DIST)
	TDLIB_BUILD_VARIANT=linux-x86_64-musl \
		$(CARGO) build $(RUST_RELEASE) --target x86_64-unknown-linux-musl
	cp -f target/x86_64-unknown-linux-musl/release/tg-proxy-check \
		$(DIST)/tg-proxy-check-linux-x86_64-musl
	@so=$$(find target/x86_64-unknown-linux-musl/release/build -path '*/td-artifacts/linux-x86_64-musl/tdlib-install/lib/libtdjson.so' 2>/dev/null | head -1); \
	if [ -n "$$so" ]; then cp -f "$$so" $(DIST)/libtdjson-linux-x86_64-musl.so && echo "Copied $$so -> $(DIST)/libtdjson-linux-x86_64-musl.so"; fi

# GNU + x86-64-v3 (AVX2, etc.) — requires CPU with v3 features at runtime
build-gnu-v3: $(DIST)
	TDLIB_BUILD_VARIANT=linux-x86_64-gnu-v3 \
		RUSTFLAGS='-C target-cpu=x86-64-v3' \
		$(CARGO) build $(RUST_RELEASE) --target x86_64-unknown-linux-gnu
	cp -f target/x86_64-unknown-linux-gnu/release/tg-proxy-check \
		$(DIST)/tg-proxy-check-linux-x86_64-gnu-v3

# musl + v3
build-musl-v3: $(DIST)
	TDLIB_BUILD_VARIANT=linux-x86_64-musl-v3 \
		RUSTFLAGS='-C target-cpu=x86-64-v3' \
		$(CARGO) build $(RUST_RELEASE) --target x86_64-unknown-linux-musl
	cp -f target/x86_64-unknown-linux-musl/release/tg-proxy-check \
		$(DIST)/tg-proxy-check-linux-x86_64-musl-v3
	@so=$$(find target/x86_64-unknown-linux-musl/release/build -path '*/td-artifacts/linux-x86_64-musl-v3/tdlib-install/lib/libtdjson.so' 2>/dev/null | head -1); \
	if [ -n "$$so" ]; then cp -f "$$so" $(DIST)/libtdjson-linux-x86_64-musl-v3.so; fi

# musl static TDLib (.a) + crt-static Rust; optional static OpenSSL via TDLIB_LINK_SSL_STATIC + OPENSSL_STATIC
build-musl-static: $(DIST)
	TDLIB_BUILD_VARIANT=linux-x86_64-musl-static \
		RUSTFLAGS='-C target-feature=+crt-static' \
		$(CARGO) build $(RUST_RELEASE) --target x86_64-unknown-linux-musl
	cp -f target/x86_64-unknown-linux-musl/release/tg-proxy-check \
		$(DIST)/tg-proxy-check-linux-x86_64-musl-static
	$(MAKE) verify-musl-static

build-musl-v3-static: $(DIST)
	TDLIB_BUILD_VARIANT=linux-x86_64-musl-v3-static \
		RUSTFLAGS='-C target-cpu=x86-64-v3 -C target-feature=+crt-static' \
		$(CARGO) build $(RUST_RELEASE) --target x86_64-unknown-linux-musl
	cp -f target/x86_64-unknown-linux-musl/release/tg-proxy-check \
		$(DIST)/tg-proxy-check-linux-x86_64-musl-v3-static
	$(MAKE) verify-musl-v3-static

verify-musl-static:
	./scripts/verify-static.sh "$(DIST)/tg-proxy-check-linux-x86_64-musl-static"

verify-musl-v3-static:
	./scripts/verify-static.sh "$(DIST)/tg-proxy-check-linux-x86_64-musl-v3-static"
