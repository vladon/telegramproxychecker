# Third-party sources

## TDLib (`td/`)

**Expected layout:** `third_party/td` should contain a checkout of [TDLib](https://github.com/tdlib/td) at the **pinned commit** in `build.rs` (`TD_COMMIT`). When bumping the pin, update **`ci/tdlib-build-fingerprint`** in the repo root to the same full hash (CI enforces a match). CMake always runs against this directory when `third_party/td/CMakeLists.txt` is present.

### Recommended: git submodule

```bash
git submodule add https://github.com/tdlib/td.git third_party/td
cd third_party/td && git fetch --depth 1 origin 8ff05a0e7e064fa796593f3105c2dcf983e279d4 && git checkout 8ff05a0e7e064fa796593f3105c2dcf983e279d4
```

### Fallback (no submodule)

If `third_party/td` is empty (only `.gitkeep`), `build.rs` downloads the same commit as a **GitHub archive** into **`target/tdlib-build-cache/source/<commit>/`**, verifies SHA-256, and builds from there. This requires `curl` or `wget` and network access on the first build.

The pinned revision exposes the multiplex JSON API: `td_create_client_id`, `td_send`, `td_receive`.
