# tg-proxy-check

Cross-platform Rust CLI that checks whether **Telegram is reachable through a proxy** using the official [TDLib](https://github.com/tdlib/td) JSON client and the **`pingProxy`** API. It does **not** require logging in with a phone number or using chat APIs.

## What `latency_ms` means (important)

`latency_ms` is **not** an ICMP ping and **not** the raw TCP connect time to the proxy host.

It **is** the round-trip time reported by TDLib **`pingProxy`**: effectively **client → proxy → Telegram → back**, as measured inside TDLib.

## Supported proxy link formats

### MTProto

- `tg://proxy?server=HOST&port=PORT&secret=SECRET`
- `https://t.me/proxy?server=HOST&port=PORT&secret=SECRET`
- `http://t.me/proxy?server=HOST&port=PORT&secret=SECRET`

The same forms work with host `telegram.me` instead of `t.me`.

### SOCKS5

- `tg://socks?server=HOST&port=PORT`
- `tg://socks?server=HOST&port=PORT&user=USER&pass=PASS`
- `https://t.me/socks?...` and `http://t.me/socks?...` (with optional `user` / `pass`)

Query parameters are URL-decoded. For `--verbose` text output, `input_link=` shows a **redacted** copy of the URL (`secret`, `pass`, `password`, and `token` query values replaced) so MTProto secrets and SOCKS passwords are not written to the terminal. The in-memory parsed configuration still holds the real values for TDLib only.

## Usage

With default features, **`cargo build`** compiles TDLib from **`third_party/td`** (recommended: git submodule at the pinned commit) or, if that tree is empty, from a **pinned** downloaded archive under `target/`—see [Vendored TDLib](#vendored-tdlib-build) and [Native prerequisites](#native-prerequisites). Use `cargo build --no-default-features` for a parser-only binary without compiling TDLib.

```bash
export TG_API_ID="123456"
export TG_API_HASH="your_api_hash"

tg-proxy-check 'tg://proxy?server=1.2.3.4&port=443&secret=...'
tg-proxy-check --proxy-link 'https://t.me/socks?server=1.2.3.4&port=1080'
```

### Flags

| Flag | Meaning |
|------|--------|
| `--proxy-link URL` | Same as positional link; **do not** pass both. |
| `--verbose` | Extra diagnostics; sensitive query params redacted in `input_link=`; see note on TDLib logs below. |
| `--json` | Single-line JSON on stdout. |
| `--timeout SECONDS` | Wall-clock limit for the whole probe (default: 60). |
| `--api-id` / `--api-hash` | Override `TG_API_ID` / `TG_API_HASH`. |

## Example output

**Success (default):**

```text
OK type=mtproto server=1.2.3.4 port=443 latency_ms=412
OK type=socks5 server=1.2.3.4 port=1080 latency_ms=128
```

**Failure (default):**

```text
FAIL type=mtproto server=1.2.3.4 port=443 error="Proxy connection failed"
FAIL type=socks5 server=1.2.3.4 port=1080 error="Timeout"
```

**JSON success:**

```json
{"ok":true,"proxy_type":"mtproto","server":"1.2.3.4","port":443,"latency_ms":412,"message":"Telegram reachable through proxy"}
```

**JSON failure:**

```json
{"ok":false,"proxy_type":"socks5","server":"1.2.3.4","port":1080,"error":"Proxy connection failed","message":"Telegram unreachable through proxy"}
```

## Exit codes

| Code | Meaning |
|------|--------|
| 0 | Success (Telegram reachable through the proxy). |
| 1 | Proxy link parsed, but Telegram not reachable through the proxy. |
| 2 | Invalid link or CLI usage. |
| 3 | Probe timed out (`--timeout`). |
| 4 | TDLib initialization failure. |
| 5 | Internal / unexpected error. |

## Vendored TDLib build

TDLib is **vendored and built by Cargo**. You do **not** clone TDLib separately, run CMake by hand, or install `libtdjson.so` (or any TDLib library) into `/usr/lib` or similar.

### Source tree

- **Normal / reproducible path:** check out TDLib under **`third_party/td`** at the pinned commit (see [`third_party/README.md`](third_party/README.md)). `build.rs` runs CMake against that directory.
- **Bootstrap path:** if `third_party/td` has no `CMakeLists.txt`, `build.rs` downloads the **same** pinned commit as a tarball into `target/<triplet>/<profile>/build/tg-proxy-check-*/out/td-src/`, verifies SHA-256, then builds. That needs `curl` or `wget` once.

Pinned revision: **[`v1.8.0`](https://github.com/tdlib/td/tree/v1.8.0)** (commit `b3ab664a18f8611f4dfcd3054717504271eeaa7a`), defined as `TD_COMMIT` in `build.rs`.

### Where native artifacts go

| Stage | Location (under `target/.../build/tg-proxy-check-<hash>/out/`) |
|--------|------------------------------------------------------------------|
| CMake build tree | `tdlib-cmake/build/` |
| `cmake --install` prefix | `tdlib-install/` (libraries in `tdlib-install/lib/`) |
| Tarball extract (if used) | `td-src/td-<commit>/` |

Nothing in this flow writes under `third_party/` except your own git submodule checkout.

### CMake driver

`build.rs` uses the Rust [**`cmake`**](https://crates.io/crates/cmake) crate so generator selection, compiler flags, and MSVC `--config` handling match common Rust native-build practice. The CMake target **`install`** is built so `tdjson` and `tdjson_static` (and dependencies) are produced consistently before files are installed into `tdlib-install/`.

### Linkage (no system `libtdjson`)

`build.rs` prints **`cargo:rustc-link-search=native=…/tdlib-install/lib`** and links the **locally installed** artifacts only:

| Platform | Strategy |
|----------|-----------|
| **Linux** (glibc, default) | Static chain: `libtdjson_static.a` + other TDLib `libtd*.a` inside `-Wl,--start-group` / `--end-group`, then dynamic **OpenSSL**, **zlib**, optional **zstd** (if TDLib was built with zstd), **dl**, **pthread**, **libstdc++**. |
| **macOS** (default) | Same static `.a` list, linked with **`-Wl,-force_load,…`** per archive (macOS `ld` has no `--start-group`), then **ssl/crypto/z** and **`libc++`**. |
| **Linux musl** | **Shared** `libtdjson.so` from the same install prefix + **rpath** to that directory (static `libstdc++/OpenSSL` pairing on musl is not the default here). |
| **Windows** | **Shared** `tdjson`; import library from `tdlib-install/lib`; **`tdjson.dll`** is copied into `target/<debug\|release>/` for `cargo run`. |

Optional: **`TDLIB_LINK_SHARED=1`** on glibc Linux forces the **local shared** `libtdjson.so` + rpath instead of the static `.a` chain (debugging or unusual link environments).

Rust FFI lives in `src/tdjson_sys.rs`; symbols are resolved from the paths above—there is no `#[link(name = "tdjson")]` relying on a system search path.

### `td_send` and memory safety

`td_send` is called with a nul-terminated JSON buffer. Per TDLib’s contract, the library **copies** that string before returning from `td_send`, so it is safe for the Rust `CString` to be dropped immediately after the call (as in this codebase).

### Verbose TDLib logs

With `--verbose`, internal TDLib log lines are printed. Lines that appear to mention `password`, `secret`, `api_hash`, `proxytype`, or `token` are replaced with a placeholder to reduce accidental credential leakage; this is heuristic and not a cryptographic guarantee.

## Native prerequisites

You need a normal **native toolchain**; nothing from TDLib has to be pre-installed.

| Prerequisite | Notes |
|--------------|--------|
| **Rust** | Stable toolchain, `cargo`. |
| **C++ compiler** | GCC or Clang on Linux/macOS; **MSVC** or MinGW on Windows. |
| **CMake** | 3.10+ (TDLib warns on older minimums). On `PATH`. |
| **OpenSSL** | Dev package with headers + libraries (`libssl-dev`, Homebrew `openssl`, etc.). On Windows, set **`OPENSSL_ROOT_DIR`** if CMake cannot find OpenSSL. |
| **zlib** | Dev package (`zlib1g-dev`, Xcode CLT, etc.). |
| **gperf** | Required by TDLib code generation (`apt install gperf`, Homebrew `gperf`, etc.). |
| **libzstd** | Often pulled in by TDLib for compression; install dev package if the link step reports missing `zstd`. |
| **curl** or **wget** | Only if `third_party/td` is not populated: first-time download of the pinned tarball. |

Use a **`third_party/td`** submodule to avoid network fetch during builds.

## Build instructions

**Default (full probe):**

```bash
cargo build --release
```

The first build compiles TDLib and can take **several minutes** and several GB under `target/`. Later `cargo build` runs are incremental (CMake + Ninja/Make reuse the tree under `out/tdlib-cmake/build` until the source path or options change).

**Parser-only (no CMake / no TDLib compile):**

```bash
cargo build --release --no-default-features
cargo test --no-default-features
```

### Linux

Example Debian/Ubuntu packages:

```bash
sudo apt install build-essential cmake libssl-dev zlib1g-dev gperf libzstd-dev curl
cargo build --release
```

### macOS

Install Xcode Command Line Tools, CMake, OpenSSL, and gperf (e.g. via Homebrew). If CMake does not find OpenSSL, set `OPENSSL_ROOT_DIR` to the Homebrew prefix before building.

### Windows

Install **CMake**, a C++ toolchain (**Visual Studio Build Tools** with C++ workload, or MinGW), and OpenSSL (e.g. **Shining Light** builds). Set **`OPENSSL_ROOT_DIR`** to your OpenSSL installation so CMake can locate it. The build script copies **`tdjson.dll`** into `target/<debug|release>/` beside the executable when linking the shared library.

### Platform caveats

- **musl / Alpine:** The build selects the **shared** `libtdjson.so` from `tdlib-install/lib` and sets **rpath** to that directory; do not strip `target/` if you rely on that path, or ship `libtdjson.so` next to your binary and adjust the loader.
- **Windows + MSVC:** Use an **x64** toolchain consistent with Rust’s `x86_64-pc-windows-msvc` target. Set **`OPENSSL_ROOT_DIR`** if CMake cannot find OpenSSL.
- **Windows + GNU:** `windows-gnu` may use different import-library names; if linking fails, try MSVC target or report an issue with the exact linker error.
- **macOS + Homebrew OpenSSL:** If CMake finds OpenSSL but the **Rust** link step cannot find `-lssl`, export library search paths (e.g. `LIBRARY_PATH` / `RUSTFLAGS=-L...`) pointing at the same prefix you pass as **`OPENSSL_ROOT_DIR`**.
- **Air-gapped / CI:** Use **`third_party/td`** at the pinned commit so no tarball download runs.

## Troubleshooting

### Probe times out (exit code 3)

- Increase `--timeout` if the proxy or route is slow; `pingProxy` measures a full path through the proxy to Telegram, not a local TCP connect.
- Run with `--verbose` and inspect `authorization_states_seen` and `wall_duration_ms` to see how far TDLib got before the deadline.
- Firewall or TLS interception on the proxy can stall the handshake indefinitely within your timeout.

### Invalid link or CLI (exit code 2)

- Pass exactly one of the positional link or `--proxy-link` (not both).
- `TG_API_ID` / `TG_API_HASH` must be set (or passed via flags) and `api_id` must be a positive integer.
- `--timeout` must be greater than zero.

### TDLib initialization failure (exit code 4)

- Confirm you did **not** use `--no-default-features` if you expect a working probe. Wrong or mismatched `api_id` / `api_hash` pairs often surface as TDLib errors during startup, not as parser errors.

### Internal / unexpected (exit code 5)

- Rare: JSON or filesystem issues during the probe. `--verbose` may include `utf8_line_bytes=` in internal errors if `td_receive` returned non-JSON (diagnostic only; the line body is not printed).

### Native build / CMake failures

| Symptom | What to do |
|---------|------------|
| **`cmake` not found** | Install CMake 3.10+ and ensure it is on `PATH` (or set **`CMAKE`** to the `cmake` binary). |
| **C/C++ compiler not found** | Install a toolchain (GCC/Clang, Xcode CLT, or MSVC Build Tools). The `cmake` crate forwards the same compilers Cargo uses for the target. |
| **Could NOT find OpenSSL** | Install dev packages (`libssl-dev`, Homebrew `openssl`, Windows Shining Light build) and/or set **`OPENSSL_ROOT_DIR`**. |
| **Could NOT find gperf** | Install `gperf` (TDLib code generation). |
| **zlib not found** | Install zlib development package (`zlib1g-dev`, etc.). |
| **Download / network error** | Add **`third_party/td`** at the pinned commit (`third_party/README.md`) so no download runs. |
| **Wrong generator on Windows** | Set **`CMAKE_GENERATOR`** (e.g. `Ninja` or a Visual Studio generator) if auto-detection fails. |
| **CMake “home dir change” / weird cache** | `cargo clean -p tg-proxy-check` (or full `cargo clean`) and rebuild. |

### Runtime loader issues (shared `tdjson` only)

Applies to **Windows**, **Linux musl**, **`TDLIB_LINK_SHARED=1`**, or any path where the **shared** `tdjson` library is linked:

- **Linux:** **rpath** points at `…/out/tdlib-install/lib`. If you move the binary without that tree, set **`LD_LIBRARY_PATH`** to that `lib` directory or copy **`libtdjson.so`** (and compatible OpenSSL/zlib) next to the binary.
- **macOS (shared mode):** Same idea with **`libtdjson.dylib`** and **`DYLD_LIBRARY_PATH`** / install names if you relocate the binary.
- **Windows:** **`tdjson.dll`** is copied to **`target/<profile>/`** during build; for distribution, keep **`tdjson.dll`** beside **`tg-proxy-check.exe`**.

## Development

```bash
cargo test --no-default-features   # fast: parser tests only
cargo test                       # requires default features / TDLib build
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets -- -D warnings
```

---

## Design note (FFI)

- **Approach:** `build.rs` drives **TDLib** with the **`cmake`** crate (`install` target → `OUT_DIR/tdlib-install`). Low-level **tdjson** C calls live in `src/tdjson_sys.rs`; `src/tdlib_live.rs` (behind the `tdlib` feature) handles `pingProxy` / authorization. Link metadata is emitted from `build.rs` only—no system `libtdjson` discovery.
- **Pinned version:** Upstream tag **`v1.8.0`** (commit `b3ab664a18f8611f4dfcd3054717504271eeaa7a`); bump `TD_COMMIT` / `TD_TARBALL_SHA256` / submodule instructions together when upgrading.
- **Caveats:** All `td_receive` calls run on **one thread**; the pointer returned by `td_receive` is only valid until the next `td_receive` / `td_execute` on that thread—this implementation copies the string immediately. Temporary TDLib database directories are created under the system temp folder per run. Every exit path after `td_create_client_id` runs `close` and clears the log callback so the next probe in-process does not inherit state. Timeouts carry a `ProbeTimeoutContext` so verbose output still shows elapsed time and authorization states reached.
