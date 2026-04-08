//! TDLib JSON client (`tdjson`) via FFI: minimal initialization and `pingProxy`.
//!
//! ## Latency semantics
//!
//! `pingProxy` measures the time for traffic to go **through the proxy to Telegram and back**,
//! as reported by TDLib. It is **not** ICMP and **not** raw TCP connect time to the proxy.
//!
//! ## API
//!
//! With the `tdlib` Cargo feature enabled, this crate uses the multiplexed interface:
//! `td_create_client_id`, `td_send`, `td_receive`, and synchronous `td_execute` for log setup
//! (see TDLib `td_json_client.h`). All `td_receive` calls run on the invoking thread; the returned
//! C string must be copied before the next `td_receive` / `td_execute` on that thread.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TdlibCredentials {
    pub api_id: i32,
    pub api_hash: String,
}

#[derive(Debug, Clone)]
pub struct TdlibProbeSettings {
    pub timeout: Duration,
    pub verbose: bool,
}

#[cfg(feature = "tdlib")]
#[path = "tdjson_sys.rs"]
mod tdjson_sys;

#[cfg(feature = "tdlib")]
#[path = "tdlib_live.rs"]
mod tdlib_live;

#[cfg(feature = "tdlib")]
pub use tdlib_live::probe_proxy;

#[cfg(not(feature = "tdlib"))]
pub fn probe_proxy(
    proxy: &crate::proxy_link::ProxyConfig,
    creds: &TdlibCredentials,
    settings: &TdlibProbeSettings,
) -> Result<crate::output::ProbeReport, crate::error::ProbeError> {
    let _ = (proxy, creds, settings);
    Err(crate::error::ProbeError::TdlibInit(
        "Built without the `tdlib` Cargo feature (use default features or `cargo build --features tdlib`)."
            .into(),
    ))
}
