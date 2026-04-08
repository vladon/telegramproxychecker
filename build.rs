//! Build vendored TDLib via CMake (`cmake` crate) and emit Cargo link metadata.
//!
//! Sources: `third_party/td` when `CMakeLists.txt` exists; otherwise a pinned tarball
//! into `OUT_DIR` (SHA-256 verified). Nothing is installed system-wide.
//!
//! ## Isolated native builds per variant
//!
//! CMake output lives under `OUT_DIR/td-artifacts/<variant-id>/` so **gnu vs musl**, **v3 RUSTFLAGS**,
//! and **static vs dynamic** never reuse the same object tree. Set **`TDLIB_BUILD_VARIANT`** for each
//! release matrix row (see `Makefile`). If unset, the id is `default`; if `default` but
//! **`CARGO_ENCODED_RUSTFLAGS`** is non-empty, a short hash is appended so `-C target-cpu=x86-64-v3`
//! does not collide with a generic CPU build sharing the same Cargo `OUT_DIR`.
//!
//! Linking (artifacts under `td-artifacts/.../tdlib-install/lib`, never `/usr/lib`):
//! - **Linux GNU / macOS:** static `.a` TDLib chain + system crypto/zlib (+ optional zstd) + C++ runtime.
//! - **Linux musl (normal):** locally built **`libtdjson.so`** + rpath (avoids libstdc++/glibc static pain).
//! - **Linux musl + variant `*musl-static*`:** static TDLib `.a` chain; optional **`TDLIB_LINK_SSL_STATIC=1`**
//!   with **`OPENSSL_STATIC=1`** for fully static binaries when your toolchain provides static OpenSSL.
//! - **Windows / `TDLIB_LINK_SHARED=1`:** shared `tdjson` + DLL copy / rpath as before.

use cmake::Config;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned TDLib revision (upstream tag `v1.8.0`).
const TD_COMMIT: &str = "b3ab664a18f8611f4dfcd3054717504271eeaa7a";
const TD_TARBALL_SHA256: &str =
    "24a7f7e289e2ada4f214058504b1c5345dbe57213a7c546b0b9b4760a172642e";

/// Order matches TDLib’s dependency graph for the JSON static stack (verified against install tree).
const TD_STATIC_LIBS: &[&str] = &[
    "tdjson_static",
    "tdjson_private",
    "tdclient",
    "tdapi",
    "tdcore",
    "tdnet",
    "tdactor",
    "tddb",
    "tdsqlite",
    "tdutils",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TDLIB");
    println!("cargo:rerun-if-env-changed=OPENSSL_ROOT_DIR");
    println!("cargo:rerun-if-env-changed=TDLIB_LINK_SHARED");
    println!("cargo:rerun-if-env-changed=TDLIB_LINK_SSL_STATIC");
    println!("cargo:rerun-if-env-changed=TDLIB_BUILD_VARIANT");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CMAKE");
    println!("cargo:rerun-if-env-changed=CMAKE_GENERATOR");

    if env::var("CARGO_FEATURE_TDLIB").is_err() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let td_src = resolve_td_source(&manifest_dir, &out_dir);
    println!(
        "cargo:rerun-if-changed={}",
        td_src.join("CMakeLists.txt").display()
    );

    let artifact_root = td_artifact_root(&out_dir);
    fs::create_dir_all(&artifact_root).expect("create td artifact root");

    let install_dir = artifact_root.join("tdlib-install");
    fs::create_dir_all(&install_dir).expect("create tdlib install dir");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let msvc = target_env == "msvc";

    let variant_raw = env::var("TDLIB_BUILD_VARIANT").unwrap_or_else(|_| "default".into());
    let v = variant_raw.to_ascii_lowercase();
    let musl_static_tdlib = target_env == "musl"
        && (v.contains("musl-static") || v.contains("musl-v3-static"));

    let mut cfg = Config::new(&td_src);
    cfg.out_dir(artifact_root.join("tdlib-cmake"))
        .profile("Release")
        .define(
            "CMAKE_INSTALL_PREFIX",
            install_dir
                .to_str()
                .expect("install path must be valid UTF-8"),
        )
        .define("TD_ENABLE_LTO", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .build_target("install");

    if msvc {
        cfg.static_crt(false);
    }

    if let Ok(root) = env::var("OPENSSL_ROOT_DIR") {
        if !root.is_empty() {
            cfg.define("OPENSSL_ROOT_DIR", root);
        }
    }

    let _cmake_out = cfg.build();

    let lib_dir = install_dir.join("lib");
    if !lib_dir.is_dir() {
        eprintln!(
            "error: TDLib install missing lib directory: {}",
            lib_dir.display()
        );
        std::process::exit(1);
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    let force_shared = env::var("TDLIB_LINK_SHARED").as_deref() == Ok("1");
    let use_shared =
        target_os == "windows" || force_shared || (target_env == "musl" && !musl_static_tdlib);

    if use_shared {
        link_local_shared(&lib_dir, &target_os);
    } else if target_os == "macos" {
        link_unix_static_apple(&lib_dir);
        link_system_crypto_z(&lib_dir, &target_os, false);
        println!("cargo:rustc-link-lib=c++");
    } else if musl_static_tdlib {
        link_unix_static_gnu(&lib_dir);
        link_system_crypto_z(&lib_dir, &target_os, ssl_static_enabled());
        println!("cargo:rustc-link-lib=static=stdc++");
    } else {
        // Linux GNU and other non-macOS Unix (BSD, etc.): GNU ld / lld style groups.
        link_unix_static_gnu(&lib_dir);
        link_system_crypto_z(&lib_dir, &target_os, false);
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}

/// Separate CMake/install trees per release variant (and per distinct RUSTFLAGS when variant is default).
fn td_artifact_root(out_dir: &Path) -> PathBuf {
    let base = env::var("TDLIB_BUILD_VARIANT").unwrap_or_else(|_| "default".into());
    let safe = sanitize_variant(&base);
    let rf = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let segment = if safe == "default" && !rf.is_empty() {
        let h = Sha256::digest(rf.as_bytes());
        let hex: String = h.iter().take(8).map(|b| format!("{b:02x}")).collect();
        format!("default_{hex}")
    } else {
        safe
    };
    out_dir.join("td-artifacts").join(segment)
}

fn sanitize_variant(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return "default".into();
    }
    t.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn ssl_static_enabled() -> bool {
    env::var("TDLIB_LINK_SSL_STATIC").as_deref() == Ok("1")
}

fn resolve_td_source(manifest_dir: &Path, out_dir: &Path) -> PathBuf {
    let submodule = manifest_dir.join("third_party/td/CMakeLists.txt");
    if submodule.is_file() {
        return manifest_dir.join("third_party/td");
    }

    let extract_root = out_dir.join("td-src");
    let stamp = extract_root.join(format!(".td-extracted-{TD_COMMIT}"));
    if stamp.is_file() {
        let inner = fs::read_to_string(&stamp).expect("read stamp");
        let p = PathBuf::from(inner.trim());
        if p.join("CMakeLists.txt").is_file() {
            return p;
        }
    }

    let _ = fs::remove_dir_all(&extract_root);
    fs::create_dir_all(&extract_root).expect("mkdir extract");

    let tarball = out_dir.join(format!("td-{TD_COMMIT}.tar.gz"));
    let url = format!("https://github.com/tdlib/td/archive/{TD_COMMIT}.tar.gz");
    download_file(&url, &tarball);
    verify_tarball(&tarball);

    {
        let file = File::open(&tarball).expect("open tarball");
        let dec = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(dec);
        archive
            .unpack(&extract_root)
            .expect("unpack TDLib tarball");
    }

    let mut td_root: Option<PathBuf> = None;
    for e in fs::read_dir(&extract_root).expect("read extract dir") {
        let e = e.expect("dir entry");
        let p = e.path();
        if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("td-"))
            && p.join("CMakeLists.txt").is_file()
        {
            td_root = Some(p);
            break;
        }
    }

    let root = td_root.expect("expected td-<ref>/ directory in tarball");
    fs::write(&stamp, root.to_str().expect("utf8 path")).expect("write stamp");
    root
}

fn download_file(url: &str, dest: &Path) {
    if dest.is_file() {
        return;
    }
    if Command::new("curl")
        .args([
            "-fL",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "-o",
        ])
        .arg(dest)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return;
    }
    if Command::new("wget")
        .args(["-O"])
        .arg(dest)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return;
    }
    panic!(
        "Could not download TDLib source.\n\
Populate `third_party/td` at the pinned commit (see third_party/README.md) or install curl/wget.\n\
URL: {url}"
    );
}

fn verify_tarball(path: &Path) {
    let mut file = File::open(path).expect("open tarball");
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).expect("read tarball");
    let hash = format!("{:x}", Sha256::digest(&buf));
    if hash != TD_TARBALL_SHA256 {
        panic!(
            "TDLib tarball SHA-256 mismatch.\n\
Expected: {expected}\n\
Actual:   {hash}\n\
Update TD_COMMIT / TD_TARBALL_SHA256 in build.rs if bumping TDLib.",
            expected = TD_TARBALL_SHA256
        );
    }
}

fn link_unix_static_gnu(lib_dir: &Path) {
    println!("cargo:rustc-link-arg=-Wl,--start-group");
    for name in TD_STATIC_LIBS {
        let p = lib_dir.join(format!("lib{name}.a"));
        if !p.is_file() {
            panic!(
                "TDLib install missing {}; run `cargo clean` and rebuild.",
                p.display()
            );
        }
        println!("cargo:rustc-link-arg={}", p.display());
    }
    println!("cargo:rustc-link-arg=-Wl,--end-group");
}

/// macOS `ld` does not support `--start-group`; `-force_load` each archive preserves symbol resolution.
fn link_unix_static_apple(lib_dir: &Path) {
    for name in TD_STATIC_LIBS {
        let p = lib_dir.join(format!("lib{name}.a"));
        if !p.is_file() {
            panic!(
                "TDLib install missing {}; run `cargo clean` and rebuild.",
                p.display()
            );
        }
        println!(
            "cargo:rustc-link-arg=-Wl,-force_load,{}",
            p.display()
        );
    }
}

fn link_system_crypto_z(lib_dir: &Path, target_os: &str, static_ssl: bool) {
    if static_ssl {
        println!("cargo:rustc-link-lib=static=ssl");
        println!("cargo:rustc-link-lib=static=crypto");
        println!("cargo:rustc-link-lib=static=z");
    } else {
        println!("cargo:rustc-link-lib=dylib=ssl");
        println!("cargo:rustc-link-lib=dylib=crypto");
        println!("cargo:rustc-link-lib=dylib=z");
    }
    if tdlib_built_with_zstd(lib_dir) {
        if static_ssl {
            println!("cargo:rustc-link-lib=static=zstd");
        } else {
            println!("cargo:rustc-link-lib=dylib=zstd");
        }
    }
    match target_os {
        "linux" => {
            println!("cargo:rustc-link-lib=dylib=dl");
            println!("cargo:rustc-link-lib=dylib=pthread");
        }
        "macos" | "windows" => {}
        // Other Unix (BSD, etc.): pthread is commonly required for the static C++ stack.
        _ => println!("cargo:rustc-link-lib=dylib=pthread"),
    }
}

fn tdlib_built_with_zstd(lib_dir: &Path) -> bool {
    let tdcore = lib_dir.join("libtdcore.a");
    let Ok(out) = Command::new("nm")
        .arg("-u")
        .arg(&tdcore)
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("ZSTD_")
}

fn link_local_shared(lib_dir: &Path, target_os: &str) {
    println!("cargo:rustc-link-lib=dylib=tdjson");

    let abs = fs::canonicalize(lib_dir).unwrap_or_else(|_| lib_dir.to_path_buf());
    let p = abs.to_string_lossy();

    match target_os {
        "macos" | "linux" => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", p);
        }
        "windows" => {
            copy_tdjson_dll_next_to_exe(lib_dir);
        }
        _ => {}
    }

    // Shared tdjson still needs OpenSSL/zlib at runtime through the .so/.dylib.
    println!("cargo:rustc-link-lib=dylib=ssl");
    println!("cargo:rustc-link-lib=dylib=crypto");
    println!("cargo:rustc-link-lib=dylib=z");

    match target_os {
        "macos" => println!("cargo:rustc-link-lib=c++"),
        "linux" => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
        _ => {}
    }
}

fn copy_tdjson_dll_next_to_exe(lib_dir: &Path) {
    let dll = lib_dir.join("tdjson.dll");
    if !dll.is_file() {
        println!(
            "cargo:warning=expected {}; place tdjson.dll next to the exe or on PATH.",
            dll.display()
        );
        return;
    }
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = manifest_dir.join("target").join(&profile);
    let _ = fs::create_dir_all(&target_dir);
    let dest = target_dir.join("tdjson.dll");
    if fs::copy(&dll, &dest).is_ok() {
        println!(
            "cargo:warning=copied tdjson.dll to {} for the Windows loader.",
            dest.display()
        );
    }
}
