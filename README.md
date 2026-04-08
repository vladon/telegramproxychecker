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

With default features, **`cargo build`** downloads a pinned TDLib source tarball (or uses `third_party/td` if present), compiles it with CMake, and links it automatically—see [Native prerequisites](#native-prerequisites). Use `cargo build --no-default-features` for a parser-only binary without compiling TDLib.

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

TDLib is **not** expected to be installed system-wide. `build.rs` uses **CMake** to compile a **pinned** upstream revision ([`v1.8.0`](https://github.com/tdlib/td/tree/v1.8.0) / commit `b3ab664a18f8611f4dfcd3054717504271eeaa7a`) into `target/*/build/.../out/`.

- **Source:** If `third_party/td/CMakeLists.txt` exists (recommended: git submodule; see [`third_party/README.md`](third_party/README.md)), that tree is used. Otherwise the same commit is fetched as a GitHub tarball (SHA-256 checked in `build.rs`).
- **Linking:** On **Linux**, the Rust binary links **static** TDLib JSON archives (`tdjson_static` and dependencies) and uses the system **OpenSSL**, **zlib**, **libzstd**, **`libstdc++`**, and **`pthread`** shared libraries. On **macOS** and **Windows**, the build links the **shared** `tdjson` library produced by CMake and sets an **rpath** (macOS/Linux shared fallback) or copies **`tdjson.dll`** next to the executable under `target/<profile>/` (Windows).

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
| **curl** or **wget** | Used by `build.rs` to download the pinned tarball when `third_party/td` is not populated. |

Optional: **`TDLIB_LINK_SHARED=1`** on Linux forces linking the built **`libtdjson.so`** instead of the static `.a` chain (debugging only).

## Build instructions

**Default (full probe):**

```bash
cargo build --release
```

The first build compiles TDLib and can take **several minutes** and several GB under `target/`. Later `cargo build` runs are incremental.

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

- **musl / Alpine:** Static linking `libstdc++` into a fully static binary is not what this `build.rs` targets; prefer **glibc** Linux for the default static-TDLib path, or set **`TDLIB_LINK_SHARED=1`** and ensure a compatible `libtdjson.so` + `libstdc++.so` at run time.
- **Windows + MSVC:** Use a **x64** native toolchain consistent with Rust’s `x86_64-pc-windows-msvc` target.
- **Air-gapped builds:** Add the TDLib tree as **`third_party/td`** at the pinned commit so `build.rs` does not download anything (see `third_party/README.md`).

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

- **“Could NOT find OpenSSL”:** Install OpenSSL development files and/or set **`OPENSSL_ROOT_DIR`**.
- **“Could NOT find gperf”:** Install `gperf` and ensure it is on `PATH`.
- **Missing `zstd` at link time:** Install `libzstd` development package.
- **Download failures:** Install `curl` or `wget`, or populate **`third_party/td`** as a submodule (see `third_party/README.md`).
- **Stale CMake cache after changing TDLib source:** `cargo clean` and rebuild.

### Runtime loader issues (shared `tdjson` path)

- **macOS** builds using the shared library embed an **rpath** to the TDLib install directory under `target/*/build/.../out/`. Do not delete `target/` before running binaries that depend on that path, or ship **`libtdjson.dylib`** with your binary and adjust loader paths.
- **Windows:** The build copies **`tdjson.dll`** into `target/<profile>/`. For distribution, ship **`tdjson.dll`** next to **`tg-proxy-check.exe`**.

## Development

```bash
cargo test --no-default-features   # fast: parser tests only
cargo test                       # requires default features / TDLib build
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets -- -D warnings
```

---

## Design note (FFI)

- **Approach:** `build.rs` vendors and compiles **TDLib** with CMake; low-level **tdjson** C calls live in `src/tdjson_sys.rs`; `src/tdlib_live.rs` (behind the `tdlib` feature) handles the `pingProxy` / authorization flow. Raw FFI + `serde_json` avoids immature bindings while keeping full control over `@extra` correlation and the authorization-state sequence.
- **Pinned version:** Upstream tag **`v1.8.0`** (commit `b3ab664a18f8611f4dfcd3054717504271eeaa7a`); bump deliberately in `build.rs` / `third_party/README.md` when upgrading.
- **Caveats:** All `td_receive` calls run on **one thread**; the pointer returned by `td_receive` is only valid until the next `td_receive` / `td_execute` on that thread—this implementation copies the string immediately. Temporary TDLib database directories are created under the system temp folder per run. Every exit path after `td_create_client_id` runs `close` and clears the log callback so the next probe in-process does not inherit state. Timeouts carry a `ProbeTimeoutContext` so verbose output still shows elapsed time and authorization states reached.
