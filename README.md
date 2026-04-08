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

Build **with** TDLib for real probes: `cargo build --release --features tdlib` (see [Build instructions](#build-instructions)). A plain `cargo build --release` produces a binary that parses links but cannot run `pingProxy` until you rebuild with `--features tdlib`.

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

## TDLib dependency (dynamic linking)

### `td_send` and memory safety

`td_send` is called with a nul-terminated JSON buffer. Per TDLib’s contract, the library **copies** that string before returning from `td_send`, so it is safe for the Rust `CString` to be dropped immediately after the call (as in this codebase).

### Verbose TDLib logs

With `--verbose`, internal TDLib log lines are printed. Lines that appear to mention `password`, `secret`, `api_hash`, `proxytype`, or `token` are replaced with a placeholder to reduce accidental credential leakage; this is heuristic and not a cryptographic guarantee.

This project links the **shared TDLib JSON client** library:

| Platform | Typical library name |
|----------|----------------------|
| Linux | `libtdjson.so` |
| macOS | `libtdjson.dylib` |
| Windows | `tdjson.dll` (import library `tdjson.lib` for MSVC) |

You must build or install TDLib so that:

1. The **linker** can find `tdjson` when building (see below).
2. The **dynamic loader** can find the `.so` / `.dylib` / `.dll` at **runtime** (e.g. `LD_LIBRARY_PATH` on Linux, `PATH` on Windows).

Minimum expected API: multiplexed JSON functions `td_create_client_id`, `td_send`, `td_receive` (TDLib 1.8+ style). Older installs that only expose `td_json_client_*` may require a small FFI adjustment.

### Pointing the build at `libtdjson`

Set **`TDLIB_LIB_DIR`** to the directory containing the library, enable the **`tdlib`** feature, then build:

```bash
export TDLIB_LIB_DIR=/opt/tdlib/lib
cargo build --release --features tdlib
```

Optional static link hint for `build.rs`:

```bash
export TDLIB_STATIC=1
```

If you ship a `tdjson.pc` file, `pkg-config` is tried automatically.

## Build instructions

**Without TDLib** (always works; probes exit with a clear “built without tdlib” error):

```bash
cargo build --release
cargo test
```

**With TDLib** (real `pingProxy` checks):

```bash
export TDLIB_LIB_DIR=/path/to/lib   # if pkg-config does not find tdjson
cargo build --release --features tdlib
```

### Linux

1. Install or build TDLib; note the directory with `libtdjson.so`.
2. `export TDLIB_LIB_DIR=/path/to/lib` (if not in a default linker path).
3. `cargo build --release --features tdlib`
4. At run time: ensure `LD_LIBRARY_PATH` includes the directory with `libtdjson.so` if needed.

### macOS

Same as Linux for `libtdjson.dylib`. If the loader cannot find the library, set `DYLD_LIBRARY_PATH` to the directory containing the dylib (SIP may restrict some `DYLD_*` uses for system binaries). Custom TDLib builds sometimes need `install_name_tool` or an `@rpath` baked into the dylib.

### Windows

- The loader searches the executable’s directory first, then `PATH`. Keep `tdjson.dll` next to `tg-proxy-check.exe` for the least fragile layout.
- **MSVC**: point `TDLIB_LIB_DIR` at the folder containing `tdjson.lib` (import library) for the link step; ship the matching `tdjson.dll` at run time.
- **GNU / MinGW**: you may need `RUSTFLAGS=-L/path/to/lib` in addition to `TDLIB_LIB_DIR`, depending on your toolchain.

### Cargo feature `tdlib`

TDLib is **not** enabled by default so the project builds without `libtdjson`. Enable it for linking and probes:

```bash
cargo build --release --features tdlib
```

Without `tdlib`, `probe_proxy` returns a clear initialization error at run time (exit code 4).

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

- Confirm the binary was built **with** `--features tdlib` and that `td_create_client_id` succeeds (see verbose output / TDLib logs if enabled). If you used a plain `cargo build --release`, rebuild with `--features tdlib`.
- Wrong or mismatched `api_id` / `api_hash` pairs often surface as TDLib errors during startup, not as parser errors.

### Internal / unexpected (exit code 5)

- Rare: JSON or filesystem issues during the probe. `--verbose` may include `utf8_line_bytes=` in internal errors if `td_receive` returned non-JSON (diagnostic only; the line body is not printed).

### TDLib linking and runtime failures

**Build stops in `build.rs` with “could not find `tdjson` for linking”**

- You ran with `--features tdlib` but neither `TDLIB_LIB_DIR` nor `pkg-config tdjson` points at the library (bare `-ltdjson` used to fail with a cryptic linker error).
- Set `TDLIB_LIB_DIR` to the directory that contains `libtdjson.so` (or the platform equivalent), then `cargo build --release --features tdlib` again.
- If you only wanted a parser-only binary, omit `--features tdlib`: `cargo build --release`.
- If you deliberately link only via `RUSTFLAGS=-L...`, either set `TDLIB_LIB_DIR` to that same directory or set `TDLIB_ALLOW_BARE_LINK=1` to opt back into bare `-ltdjson`.

**Link step: “cannot find `-ltdjson`” / unresolved `td_create_client_id`**

- Set `TDLIB_LIB_DIR` to the directory containing the import library / `.so` / `.dylib`, then rebuild.
- On some MinGW setups you may also need `RUSTFLAGS=-L/path/to/lib` so the linker sees the library.

**Run time: error loading shared library / `libtdjson.so` not found**

- Linux: add the directory containing `libtdjson.so` to `LD_LIBRARY_PATH`, or install the library into a path the dynamic loader already searches (e.g. `/usr/lib`).
- macOS: ensure the dylib is on the loader path (`DYLD_LIBRARY_PATH` for local builds; SIP may limit this for some binaries). You may need `install_name_tool` or an `@rpath` on custom builds.
- Windows: place `tdjson.dll` next to `tg-proxy-check.exe` or on `PATH`.

**Wrong TDLib ABI / version**

- The crate expects the multiplexed JSON API (`td_create_client_id`, `td_send`, `td_receive`). Very old installs that only ship `td_json_client_*` need a different FFI layer; mismatched headers vs binary often crash or return garbage—rebuild TDLib and this tool against the same version.

**Parser-only workflows**

- `cargo build --release` and `cargo test` work without `libtdjson`. Add `--features tdlib` when you install TDLib.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
# optional: type-check / lint the TDLib FFI path
cargo clippy --all-targets --features tdlib -- -D warnings
```

---

## Design note (FFI)

- **Approach:** Low-level **tdjson** calls live in `src/tdjson_sys.rs`; `src/tdlib_live.rs` (behind the `tdlib` feature) handles the `pingProxy` / authorization flow, with `build.rs` making link flags explicit. This avoids immature generated bindings while keeping full control over `@extra` correlation and the authorization-state sequence.
- **Why not a Rust TDLib crate:** Few crates track upstream closely; raw FFI + `serde_json` is simpler to keep buildable and debuggable.
- **Assumptions:** A compatible `tdjson` shared library is installed; JSON field names match your TDLib version (snake_case keys as in upstream TL).
- **Caveats:** All `td_receive` calls used here run on **one thread**; the pointer returned by `td_receive` is only valid until the next `td_receive` / `td_execute` on that thread—this implementation copies the string immediately. Temporary TDLib database directories are created under the system temp folder per run. Every exit path after `td_create_client_id` runs `close` and clears the log callback so the next probe in-process does not inherit state. Timeouts carry a `ProbeTimeoutContext` so verbose output still shows elapsed time and authorization states reached.
