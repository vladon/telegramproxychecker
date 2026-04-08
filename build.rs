//! Link against the TDLib JSON client shared library (`tdjson`).
//!
//! Set `TDLIB_LIB_DIR` to the directory containing `libtdjson.so` (Linux),
//! `libtdjson.dylib` (macOS), or `tdjson.lib` / `tdjson.dll` (Windows).
//! Alternatively, install a `tdjson` pkg-config file, or pass
//! `RUSTFLAGS=-L/path/to/lib` when building.

use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=TDLIB_LIB_DIR");
    println!("cargo:rerun-if-env-changed=TDLIB_STATIC");

    if env::var("CARGO_FEATURE_TDLIB").is_err() {
        println!("cargo:warning=Building without `tdlib` feature: TDLib is not linked; probe_proxy will report a build-time error.");
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

    // Default: dynamic link; linker searches default paths / LIBRARY_PATH / PATH.
    println!("cargo:warning=tdjson: TDLIB_LIB_DIR not set and pkg-config did not find tdjson; linking with -ltdjson (ensure the library is on the linker path).");
    println!("cargo:rustc-link-lib=dylib={}", lib_name);
}
