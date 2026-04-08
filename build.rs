//! Vendor and build TDLib from a pinned upstream revision (see `third_party/README.md`).
//!
//! Source resolution:
//! 1. If `third_party/td/CMakeLists.txt` exists (git submodule or manual checkout), use it.
//! 2. Otherwise download a pinned tarball into `OUT_DIR`, verify SHA-256, and extract.
//!
//! Build: CMake configures TDLib, builds target `tdjson`, installs to `OUT_DIR/tdlib-install`.
//! Linking:
//! - Linux (GNU/musl): static `tdjson_static` + TDLib `.a` chain, system OpenSSL/zlib/zstd, `libstdc++`.
//! - macOS / Windows: link the built `tdjson` shared library and set rpath / loader path hints.

use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned TDLib revision (tag `v1.8.0` on upstream).
const TD_COMMIT: &str = "b3ab664a18f8611f4dfcd3054717504271eeaa7a";
const TD_TARBALL_SHA256: &str =
    "24a7f7e289e2ada4f214058504b1c5345dbe57213a7c546b0b9b4760a172642e";

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TDLIB");
    println!("cargo:rerun-if-env-changed=OPENSSL_ROOT_DIR");
    println!("cargo:rerun-if-env-changed=TDLIB_LINK_SHARED");
    println!("cargo:rerun-if-env-changed=CMAKE");
    println!("cargo:rerun-if-env-changed=CMAKE_GENERATOR");

    if env::var("CARGO_FEATURE_TDLIB").is_err() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let td_src = resolve_td_source(&manifest_dir, &out_dir);
    println!("cargo:rerun-if-changed={}", td_src.join("CMakeLists.txt").display());

    let build_dir = out_dir.join("tdlib-build");
    let install_dir = out_dir.join("tdlib-install");
    fs::create_dir_all(&build_dir).expect("create tdlib build dir");
    fs::create_dir_all(&install_dir).expect("create tdlib install dir");

    // TDLib is always built in Release mode to keep configure/build predictable (MSVC multi-config)
    // and to avoid extremely slow Debug C++ builds on every `cargo build`.
    let cmake_build_type = "Release";

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let msvc = target_env == "msvc";

    if !build_dir.join("CMakeCache.txt").is_file() {
        configure_tdlib(
            &td_src,
            &build_dir,
            &install_dir,
            cmake_build_type,
            msvc,
        );
    }
    build_tdjson(&build_dir, msvc, cmake_build_type);
    install_tdlib(&build_dir, &install_dir, msvc, cmake_build_type);

    let lib_dir = install_dir.join("lib");
    if !lib_dir.is_dir() {
        panic!(
            "TDLib install missing lib directory: {}",
            lib_dir.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    let force_shared = env::var("TDLIB_LINK_SHARED").as_deref() == Ok("1");
    let use_static = !force_shared && target_os == "linux";

    if use_static {
        link_linux_static(&lib_dir);
        println!("cargo:rustc-link-lib=dylib=stdc++");
    } else {
        link_shared(&lib_dir, &target_os);
        match target_os.as_str() {
            "macos" => println!("cargo:rustc-link-lib=c++"),
            "linux" => println!("cargo:rustc-link-lib=dylib=stdc++"),
            _ => {}
        }
    }
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
Install `curl` or `wget`, or add a git submodule at third_party/td (see third_party/README.md).\n\
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
The pinned archive may have been replaced; update TD_COMMIT / TD_TARBALL_SHA256 in build.rs.",
            expected = TD_TARBALL_SHA256
        );
    }
}

fn configure_tdlib(
    src: &Path,
    build: &Path,
    install: &Path,
    cmake_build_type: &str,
    msvc: bool,
) {
    let mut cmd = Command::new("cmake");
    cmd.arg("-S")
        .arg(src)
        .arg("-B")
        .arg(build)
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", install.display()))
        .arg(format!("-DCMAKE_BUILD_TYPE={cmake_build_type}"))
        .arg("-DTD_ENABLE_LTO=OFF")
        .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON");

    if msvc {
        cmd.arg("-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL");
    }

    if let Ok(root) = env::var("OPENSSL_ROOT_DIR") {
        if !root.is_empty() {
            cmd.arg(format!("-DOPENSSL_ROOT_DIR={root}"));
        }
    }

    run(&mut cmd, "cmake (configure TDLib)");
}

fn build_tdjson(build: &Path, msvc: bool, cmake_build_type: &str) {
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "1".into());
    let mut cmd = Command::new("cmake");
    cmd.arg("--build")
        .arg(build)
        .arg("--parallel")
        .arg(&jobs)
        .arg("--target")
        .arg("tdjson")
        .arg("--target")
        .arg("tdjson_static");
    if msvc {
        cmd.arg("--config").arg(cmake_build_type);
    }
    run(&mut cmd, "cmake --build tdjson");
}

fn install_tdlib(build: &Path, install: &Path, msvc: bool, cmake_build_type: &str) {
    let mut cmd = Command::new("cmake");
    cmd.arg("--install")
        .arg(build)
        .arg("--prefix")
        .arg(install);
    if msvc {
        cmd.arg("--config").arg(cmake_build_type);
    }
    run(&mut cmd, "cmake --install TDLib");
}

fn run(cmd: &mut Command, what: &str) {
    let st = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "error: failed to spawn {what}: {e}\n\
Install CMake and a C++ toolchain and ensure both are on PATH."
            );
            std::process::exit(1);
        }
    };
    if !st.success() {
        eprintln!("error: {what} failed with status {st}");
        std::process::exit(1);
    }
}

/// Full paths inside one linker group — `cargo:rustc-link-lib=static` does not reliably stay
/// between paired `link-arg` groups when rustc merges flags.
fn link_linux_static(lib_dir: &Path) {
    println!("cargo:rustc-link-arg=-Wl,--start-group");
    for name in [
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
    ] {
        let p = lib_dir.join(format!("lib{name}.a"));
        if !p.is_file() {
            panic!(
                "TDLib CMake install did not produce {}; try `cargo clean` and rebuild.",
                p.display()
            );
        }
        println!("cargo:rustc-link-arg={}", p.display());
    }
    println!("cargo:rustc-link-arg=-Wl,--end-group");

    println!("cargo:rustc-link-lib=dylib=ssl");
    println!("cargo:rustc-link-lib=dylib=crypto");
    println!("cargo:rustc-link-lib=dylib=z");
    if tdlib_built_with_zstd(lib_dir) {
        println!("cargo:rustc-link-lib=dylib=zstd");
    }
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rustc-link-lib=dylib=pthread");
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

fn link_shared(lib_dir: &Path, target_os: &str) {
    println!("cargo:rustc-link-lib=dylib=tdjson");

    let abs = fs::canonicalize(lib_dir).unwrap_or_else(|_| lib_dir.to_path_buf());
    let p = abs.to_string_lossy();
    match target_os {
        "macos" => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", p);
        }
        "linux" => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", p);
        }
        "windows" => {
            copy_tdjson_dll_next_to_exe(lib_dir);
        }
        _ => {}
    }
}

/// Windows has no rpath; copy `tdjson.dll` next to the built exe so `cargo run` and `target/*/tg-proxy-check.exe` work.
fn copy_tdjson_dll_next_to_exe(lib_dir: &Path) {
    let dll = lib_dir.join("tdjson.dll");
    if !dll.is_file() {
        println!(
            "cargo:warning=expected {} — TDLib Windows install layout may differ; ensure tdjson.dll is on PATH.",
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
        println!("cargo:warning=copied tdjson.dll to {} for the loader.", dest.display());
    }
}
