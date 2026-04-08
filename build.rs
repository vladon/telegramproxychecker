//! Link against the TDLib JSON client shared library (`tdjson`).
//!
//! Set `TDLIB_LIB_DIR` to the directory containing `libtdjson.so` (Linux),
//! `libtdjson.dylib` (macOS), or `tdjson.lib` / `tdjson.dll` (Windows).
//! Alternatively, install a `tdjson` pkg-config file, or pass
//! `RUSTFLAGS=-L/path/to/lib` when building.

use std::env;
use std::path::Path;
use std::process;

fn main() {
    println!("cargo:rerun-if-env-changed=TDLIB_LIB_DIR");
    println!("cargo:rerun-if-env-changed=TDLIB_STATIC");
    println!("cargo:rerun-if-env-changed=TDLIB_ALLOW_BARE_LINK");

    if env::var("CARGO_FEATURE_TDLIB").is_err() {
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:warning=Windows: link with `tdjson.lib` from TDLIB_LIB_DIR (MSVC) or pass `-L` via RUSTFLAGS for GNU; ship `tdjson.dll` beside the exe or on PATH at runtime.");
    } else if target_os == "macos" {
        println!("cargo:warning=macOS: if loading fails at runtime, set `DYLD_LIBRARY_PATH` to the folder containing `libtdjson.dylib`, or fix install names with `install_name_tool` / `@rpath` when you build TDLib.");
    }

    let lib_name = "tdjson";

    if let Ok(dir) = env::var("TDLIB_LIB_DIR") {
        let path = Path::new(&dir);
        if path.is_dir() {
            println!("cargo:rustc-link-search=native={}", path.display());
        }
        if env::var("TDLIB_STATIC").as_deref() == Ok("1") {
            println!("cargo:rustc-link-lib=static={}", lib_name);
        } else {
            println!("cargo:rustc-link-lib=dylib={}", lib_name);
        }
        return;
    }

    if pkg_config::Config::new()
        .cargo_metadata(true)
        .probe("tdjson")
        .is_ok()
    {
        return;
    }

    // Without a link-search path, `-ltdjson` almost always fails with an opaque "unable to find
    // library" from the linker. Fail here with actionable instructions instead.
    if env::var("TDLIB_ALLOW_BARE_LINK").as_deref() == Ok("1") {
        println!("cargo:warning=tdjson: TDLIB_ALLOW_BARE_LINK=1: linking with -ltdjson only (you must supply -L via RUSTFLAGS or a default linker path).");
        println!("cargo:rustc-link-lib=dylib={}", lib_name);
        return;
    }

    eprintln!(
        "\
error: could not find `tdjson` for linking (TDLib JSON client).

Enable the `tdlib` feature only when you have TDLib installed, then either:

  export TDLIB_LIB_DIR=/path/to/dir/containing/libtdjson.so
  cargo build --release --features tdlib

Or install a pkg-config file for `tdjson` (so `pkg-config --libs tdjson` works) and run:

  cargo build --release --features tdlib

To build without TDLib (link parsing only; probes return a clear error at run time):

  cargo build --release

Advanced: if you link only via RUSTFLAGS (-L/path), set TDLIB_LIB_DIR to that path, or set
TDLIB_ALLOW_BARE_LINK=1 for bare `-ltdjson`."
    );
    process::exit(1);
}
