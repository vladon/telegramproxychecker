# Third-party sources

## TDLib (`td/`)

**Expected layout:** `third_party/td` should contain a checkout of [TDLib](https://github.com/tdlib/td) at the **pinned commit** used in `build.rs` (`TD_COMMIT`, same as tag `v1.8.0`). CMake always runs against this directory when `third_party/td/CMakeLists.txt` is present.

### Recommended: git submodule

```bash
git submodule add https://github.com/tdlib/td.git third_party/td
cd third_party/td && git fetch --depth 1 origin b3ab664a18f8611f4dfcd3054717504271eeaa7a && git checkout b3ab664a18f8611f4dfcd3054717504271eeaa7a
```

### Fallback (no submodule)

If `third_party/td` is empty (only `.gitkeep`), `build.rs` downloads the same commit as a **GitHub archive** into `target/*/build/.../out/td-src/`, verifies SHA-256, and builds from there. This requires `curl` or `wget` and network access on the first build.

The pinned revision exposes the multiplex JSON API: `td_create_client_id`, `td_send`, `td_receive`.
