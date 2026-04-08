# Third-party sources

## TDLib (`td/`)

This crate builds [TDLib](https://github.com/tdlib/td) automatically from a **pinned commit** during `cargo build` (see `build.rs`). You do **not** need a system-wide `libtdjson` install.

### Optional git submodule (preferred for air-gapped / review)

Instead of downloading the tarball into `target/`, you may vendor the same revision:

```bash
git submodule add https://github.com/tdlib/td.git third_party/td
cd third_party/td && git fetch --depth 1 origin b3ab664a18f8611f4dfcd3054717504271eeaa7a && git checkout b3ab664a18f8611f4dfcd3054717504271eeaa7a
```

If `third_party/td/CMakeLists.txt` exists, `build.rs` uses it and skips the download.

The pinned revision matches **upstream tag `v1.8.0`** (multiplex JSON API: `td_create_client_id`, `td_send`, `td_receive`).
