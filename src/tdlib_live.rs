//! TDLib multiplexed JSON client: `td_create_client_id` / `td_send` / `td_receive`.
//!
//! TDLib **v1.8.0** defines `pingProxy proxy_id:int32` only (no inline proxy). This module first
//! sends **`addProxy`** (`enable: true`), then **`pingProxy`** with the returned **`id`**.
//!
//! The add/ping sequence starts once TDLib reports `authorizationStateWaitPhoneNumber`, or
//! immediately after a successful `checkDatabaseEncryptionKey` response (`ok`).

use super::tdjson_sys;
use super::{TdlibCredentials, TdlibProbeSettings};
use crate::error::{ProbeError, ProbeTimeoutContext};
use crate::output::{success_interpretation, wall_ms, Interpretation, ProbeReport};
use crate::output::SponsoredReport;
use crate::proxy_link::{ProxyConfig, ProxyKind};
use serde_json::{json, Value};
use std::ffi::CStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static EXTRA_COUNTER: AtomicU64 = AtomicU64::new(1);
static TD_LOG_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn next_extra(prefix: &str) -> String {
    let n = EXTRA_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", prefix, wall_ms(), n)
}

/// Never block TDLib: if the main thread holds `TD_LOG_LINES` during a future refactor, `try_lock`
/// drops the line instead of risking a deadlock with `td_receive` on the same thread.
extern "C" fn td_log_cb(verbosity: libc::c_int, message: *const libc::c_char) {
    if message.is_null() {
        return;
    }
    unsafe {
        let Ok(s) = CStr::from_ptr(message).to_str() else {
            return;
        };
        if let Ok(mut g) = TD_LOG_LINES.try_lock() {
            const MAX: usize = 256;
            if g.len() >= MAX {
                g.remove(0);
            }
            g.push(format!("[v{}] {}", verbosity, s));
        }
    }
}

/// Respects `deadline`: returns `None` when already past, otherwise blocks up to `min(remaining, max_slice)` (≥1 ms).
fn receive_json_until(deadline: Instant, max_slice: Duration) -> Option<String> {
    let now = Instant::now();
    if now >= deadline {
        return None;
    }
    let remaining = deadline.saturating_duration_since(now);
    let slice = remaining.min(max_slice).max(Duration::from_millis(1));
    tdjson_sys::receive_line(slice)
}

fn json_type_name(v: &Value) -> Option<&str> {
    match v.get("@type")? {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    }
}

fn parse_tdlib_seconds(v: &Value) -> Option<f64> {
    let sec = v.get("seconds")?;
    match sec {
        Value::Number(n) => n
            .as_f64()
            .or_else(|| n.as_i64().map(|i| i as f64))
            .or_else(|| n.as_u64().map(|u| u as f64)),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn tdlib_error_message(v: &Value) -> String {
    v.pointer("/message")
        .and_then(|m| m.as_str())
        .or_else(|| v.get("message").and_then(|m| m.as_str()))
        .unwrap_or("unknown TDLib error")
        .to_string()
}

/// Normalize `@extra` for comparisons (TDLib normally uses a string; be defensive).
fn extra_for_match(v: &Value) -> Option<String> {
    let e = v.get("@extra")?;
    Some(match e {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => serde_json::to_string(e).unwrap_or_default(),
    })
}

fn send_raw(client_id: i32, json: &str) -> Result<(), ProbeError> {
    tdjson_sys::send_json(client_id, json).map_err(|e| {
        ProbeError::Internal(format!("request contains interior nul: {e}"))
    })
}


fn client_matches(v: &Value, client_id: i32) -> bool {
    match v.get("@client_id") {
        // TDLib 1.8+ tags updates; if absent (single-client / older build), accept the update.
        None => true,
        Some(id) => {
            if let Some(n) = id.as_i64() {
                n == i64::from(client_id)
            } else if let Some(n) = id.as_u64() {
                n == client_id as u64
            } else {
                false
            }
        }
    }
}

fn auth_state_from_update(v: &Value) -> Option<String> {
    v.pointer("/authorization_state/@type")
        .and_then(|t| t.as_str())
        .map(String::from)
}

struct LogCallbackGuard {
    /// If true, `td_set_log_message_callback(0, None)` on drop (verbose path installed `td_log_cb`).
    clear_on_drop: bool,
}

impl Drop for LogCallbackGuard {
    fn drop(&mut self) {
        if self.clear_on_drop {
            tdjson_sys::set_log_callback(0, None);
        }
    }
}

/// Run TDLib `pingProxy` for the given proxy configuration.
pub fn probe_proxy(
    proxy: &ProxyConfig,
    creds: &TdlibCredentials,
    settings: &TdlibProbeSettings,
) -> Result<ProbeReport, ProbeError> {
    let start_wall = wall_ms();
    let start_instant = Instant::now();
    let deadline = start_instant + settings.timeout;

    // TDLib defaults to a stderr log stream; clearing the Rust callback alone does not silence it.
    // `logStreamEmpty` stops default file/stderr output; verbosity controls what reaches the callback.
    tdjson_sys::execute_sync(r#"{"@type":"setLogStream","log_stream":{"@type":"logStreamEmpty"}}"#)
        .map_err(|e| ProbeError::Internal(format!("td_execute setLogStream: {e}")))?;
    let verbosity = if settings.verbose { 3 } else { 0 };
    tdjson_sys::execute_sync(&format!(
        r#"{{"@type":"setLogVerbosityLevel","new_verbosity_level":{verbosity}}}"#
    ))
    .map_err(|e| ProbeError::Internal(format!("td_execute setLogVerbosityLevel: {e}")))?;

    let mut log_callback_guard = LogCallbackGuard {
        clear_on_drop: false,
    };

    if settings.verbose {
        if let Ok(mut g) = TD_LOG_LINES.try_lock() {
            g.clear();
        }
        // Match or exceed typical TDLib diagnostic lines (often tagged as verbosity 3).
        tdjson_sys::set_log_callback(4, Some(td_log_cb));
        log_callback_guard.clear_on_drop = true;
    } else {
        tdjson_sys::set_log_callback(0, None);
    }

    let temp = tempfile::Builder::new()
        .prefix("tg-proxy-check-tdlib-")
        .tempdir()
        .map_err(|e| ProbeError::Internal(format!("temp dir: {e}")))?;
    let db_dir = temp.path().join("db");
    let files_dir = temp.path().join("files");
    std::fs::create_dir_all(&db_dir)
        .map_err(|e| ProbeError::Internal(format!("create database_directory: {e}")))?;
    std::fs::create_dir_all(&files_dir)
        .map_err(|e| ProbeError::Internal(format!("create files_directory: {e}")))?;

    let db_path = path_to_tdlib_string(&db_dir);
    let files_path = path_to_tdlib_string(&files_dir);

    let client_id = tdjson_sys::create_client_id();
    if client_id <= 0 {
        return Err(ProbeError::TdlibInit(format!(
            "td_create_client_id returned invalid id {client_id}"
        )));
    }

    let mut auth_states_seen: Vec<String> = Vec::new();
    let mut set_params_sent = false;
    let mut check_key_sent = false;
    let mut add_proxy_extra: Option<String> = None;
    let mut ping_extra: Option<String> = None;
    let mut ping_result: Option<Result<f64, String>> = None;

    let extra_auth = next_extra("getAuthorizationState");
    let get_auth = json!({
        "@type": "getAuthorizationState",
        "@extra": extra_auth.clone(),
    });
    let get_auth_s = match serde_json::to_string(&get_auth) {
        Ok(s) => s,
        Err(e) => {
            td_shutdown_session(client_id);
            return Err(ProbeError::Internal(format!(
                "serialize getAuthorizationState: {e}"
            )));
        }
    };
    if let Err(e) = send_raw(client_id, &get_auth_s) {
        td_shutdown_session(client_id);
        return Err(e);
    }

    while Instant::now() < deadline && ping_result.is_none() {
        let Some(line) = receive_json_until(deadline, Duration::from_millis(500)) else {
            continue;
        };
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                td_shutdown_session(client_id);
                return Err(ProbeError::Internal(format!(
                    "invalid JSON from td_receive: {e} (utf8_line_bytes={})",
                    line.len()
                )));
            }
        };

        if !client_matches(&v, client_id) {
            continue;
        }

        let Some(typ) = json_type_name(&v) else {
            continue;
        };

        match typ {
            "updateAuthorizationState" => {
                if let Some(st) = auth_state_from_update(&v) {
                    push_unique(&mut auth_states_seen, &st);
                    if let Err(e) = handle_auth_state(
                        client_id,
                        proxy,
                        creds,
                        &db_path,
                        &files_path,
                        &st,
                        &mut set_params_sent,
                        &mut check_key_sent,
                        &mut add_proxy_extra,
                        &mut ping_extra,
                        &mut ping_result,
                    ) {
                        td_shutdown_session(client_id);
                        return Err(e);
                    }
                }
            }
            t if t.starts_with("authorizationState") => {
                push_unique(&mut auth_states_seen, t);
                if let Err(e) = handle_auth_state(
                    client_id,
                    proxy,
                    creds,
                    &db_path,
                    &files_path,
                    t,
                    &mut set_params_sent,
                    &mut check_key_sent,
                    &mut add_proxy_extra,
                    &mut ping_extra,
                    &mut ping_result,
                ) {
                    td_shutdown_session(client_id);
                    return Err(e);
                }
            }
            "error" => {
                let ex = extra_for_match(&v).unwrap_or_default();
                let msg = tdlib_error_message(&v);
                if ex == extra_auth {
                    td_shutdown_session(client_id);
                    return Err(ProbeError::TdlibInit(msg));
                }
                if let Some(ae) = add_proxy_extra.as_deref() {
                    if ex == ae {
                        td_shutdown_session(client_id);
                        return Err(ProbeError::TdlibInit(format!(
                            "addProxy failed: {msg}"
                        )));
                    }
                }
                if let Some(pe) = ping_extra.as_deref() {
                    if ex == pe {
                        ping_result = Some(Err(msg));
                        break;
                    }
                }
                if ex.starts_with("setTdlibParameters-") {
                    td_shutdown_session(client_id);
                    return Err(ProbeError::TdlibInit(msg));
                }
                if ex.starts_with("checkDatabaseEncryptionKey-") {
                    td_shutdown_session(client_id);
                    return Err(ProbeError::TdlibInit(msg));
                }
            }
            "ok" => {
                let ex = extra_for_match(&v).unwrap_or_default();
                if ex.starts_with("checkDatabaseEncryptionKey-") {
                    if let Err(e) = try_start_proxy_ping(
                        client_id,
                        proxy,
                        &mut add_proxy_extra,
                        &mut ping_extra,
                        &mut ping_result,
                    ) {
                        td_shutdown_session(client_id);
                        return Err(e);
                    }
                }
            }
            "proxy" => {
                let ex = extra_for_match(&v).unwrap_or_default();
                if Some(ex.as_str()) == add_proxy_extra.as_deref() {
                    if let Err(e) = on_add_proxy_response(
                        client_id,
                        &v,
                        &mut add_proxy_extra,
                        &mut ping_extra,
                        &mut ping_result,
                    ) {
                        td_shutdown_session(client_id);
                        return Err(e);
                    }
                }
            }
            "seconds" => {
                let ex = extra_for_match(&v).unwrap_or_default();
                if Some(ex.as_str()) == ping_extra.as_deref() {
                    let Some(sec) = parse_tdlib_seconds(&v) else {
                        td_shutdown_session(client_id);
                        return Err(ProbeError::Internal(
                            "pingProxy response missing or unparsable seconds".into(),
                        ));
                    };
                    ping_result = Some(Ok(sec));
                    break;
                }
            }
            _ => {}
        }
    }

    let end_wall = wall_ms();
    let wall_duration = start_instant.elapsed();
    let tdlib_log = copy_log_lines();

    let Some(pr) = ping_result else {
        td_shutdown_session(client_id);
        return Err(ProbeError::Timeout(ProbeTimeoutContext {
            probe_start_wall_ms: start_wall,
            probe_end_wall_ms: end_wall,
            wall_duration,
            auth_states_seen,
            tdlib_log_lines: tdlib_log,
        }));
    };

    let sponsored = match (&pr, proxy.kind) {
        (Ok(_), ProxyKind::Socks5) => SponsoredReport::socks5_na(),
        (Ok(_), ProxyKind::Mtproto) => detect_mtproto_sponsored_channel(client_id, deadline),
        (Err(_), ProxyKind::Socks5) => SponsoredReport::socks5_na(),
        (Err(_), ProxyKind::Mtproto) => {
            SponsoredReport::unknown_none("pingProxy did not succeed")
        }
    };

    td_shutdown_session(client_id);

    match pr {
        Ok(sec) => {
            let latency_ms = (sec * 1000.0).round().max(0.0) as u64;
            let interpretation = success_interpretation(latency_ms);
            Ok(ProbeReport {
                ok: true,
                latency_ms: Some(latency_ms),
                error_message: None,
                interpretation,
                auth_states_seen,
                tdlib_log_lines: tdlib_log,
                probe_start_wall_ms: start_wall,
                probe_end_wall_ms: end_wall,
                wall_duration,
                tdlib_reported_seconds: Some(sec),
                sponsored,
            })
        }
        Err(msg) => Ok(ProbeReport {
            ok: false,
            latency_ms: None,
            error_message: Some(msg),
            interpretation: Interpretation::ProxyReachableTelegramUnavailable,
            auth_states_seen,
            tdlib_log_lines: tdlib_log,
            probe_start_wall_ms: start_wall,
            probe_end_wall_ms: end_wall,
            wall_duration,
            tdlib_reported_seconds: None,
            sponsored,
        }),
    }
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if v.last().map(|x| x.as_str()) != Some(s) {
        v.push(s.to_string());
    }
}

/// After a successful `pingProxy`, ask TDLib to load the main chat list and look for
/// [`chatSourceMtprotoProxy`](https://core.telegram.org/tdlib/docs/td__api_8h.html) on a chat
/// position (TDLib marks promo channels inserted by an MTProto proxy this way). No login is
/// performed; if nothing is observed before the probe deadline, status stays **unknown**.
fn detect_mtproto_sponsored_channel(client_id: i32, deadline: Instant) -> SponsoredReport {
    if Instant::now() >= deadline {
        return SponsoredReport::unknown_tdlib(
            "probe deadline already reached before loadChats",
        );
    }
    let extra = next_extra("loadChats");
    let req = json!({
        "@type": "loadChats",
        "chat_list": {"@type": "chatListMain"},
        "limit": 100,
        "@extra": extra,
    });
    let req_s = match serde_json::to_string(&req) {
        Ok(s) => s,
        Err(_) => return SponsoredReport::unknown_tdlib("serialize loadChats failed"),
    };
    if send_raw(client_id, &req_s).is_err() {
        return SponsoredReport::unknown_tdlib("send loadChats failed");
    }

    let mut saw_load_response = false;

    while Instant::now() < deadline {
        let Some(line) = receive_json_until(deadline, Duration::from_millis(500)) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !client_matches(&v, client_id) {
            continue;
        }

        let Some(typ) = json_type_name(&v) else {
            continue;
        };

        if typ == "error" {
            if extra_for_match(&v).as_deref() != Some(extra.as_str()) {
                continue;
            }
            let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            // TDLib uses 404 when there is nothing left to load for `loadChats`.
            if code == 404 {
                saw_load_response = true;
                continue;
            }
            let msg = tdlib_error_message(&v);
            return SponsoredReport::unknown_tdlib(format!("loadChats: {msg} (code {code})"));
        }

        if typ == "ok" && extra_for_match(&v).as_deref() == Some(extra.as_str()) {
            saw_load_response = true;
            continue;
        }

        if typ == "updateNewChat" {
            if let Some(id) = chat_id_if_mtproto_sponsored_from_new_chat(&v) {
                return SponsoredReport::yes_tdlib(id, "chatSourceMtprotoProxy in updateNewChat");
            }
        }

        if typ == "updateChatPosition" {
            if let Some(id) = chat_id_if_mtproto_sponsored_from_position_update(&v) {
                return SponsoredReport::yes_tdlib(
                    id,
                    "chatSourceMtprotoProxy in updateChatPosition",
                );
            }
        }
    }

    if saw_load_response {
        SponsoredReport::unknown_tdlib(
            "no chat with chatSourceMtprotoProxy observed before probe deadline",
        )
    } else {
        SponsoredReport::unknown_tdlib("loadChats did not complete before probe deadline")
    }
}

fn position_source_is_mtproto_proxy(pos: &Value) -> bool {
    pos.pointer("/source/@type")
        .and_then(|t| t.as_str())
        == Some("chatSourceMtprotoProxy")
}

fn chat_id_if_mtproto_sponsored_from_new_chat(v: &Value) -> Option<i64> {
    let chat = v.get("chat")?;
    let arr = chat.get("positions")?.as_array()?;
    for p in arr {
        if position_source_is_mtproto_proxy(p) {
            return chat.get("id").and_then(|x| x.as_i64());
        }
    }
    None
}

fn chat_id_if_mtproto_sponsored_from_position_update(v: &Value) -> Option<i64> {
    let pos = v.get("position")?;
    if !position_source_is_mtproto_proxy(pos) {
        return None;
    }
    v.get("chat_id").and_then(|x| x.as_i64())
}

#[allow(clippy::too_many_arguments)]
fn handle_auth_state(
    client_id: i32,
    proxy: &ProxyConfig,
    creds: &TdlibCredentials,
    db_path: &str,
    files_path: &str,
    state: &str,
    set_params_sent: &mut bool,
    check_key_sent: &mut bool,
    add_proxy_extra: &mut Option<String>,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
) -> Result<(), ProbeError> {
    match state {
        "authorizationStateWaitTdlibParameters" if !*set_params_sent => {
            let extra = next_extra("setTdlibParameters");
            let req = build_set_tdlib_parameters(extra.as_str(), db_path, files_path, creds);
            let s = serde_json::to_string(&req)
                .map_err(|e| ProbeError::Internal(format!("serialize setTdlibParameters: {e}")))?;
            send_raw(client_id, &s)?;
            *set_params_sent = true;
        }
        "authorizationStateWaitEncryptionKey" if !*check_key_sent => {
            let extra = next_extra("checkDatabaseEncryptionKey");
            let req = json!({
                "@type": "checkDatabaseEncryptionKey",
                "encryption_key": "",
                "@extra": extra,
            });
            let s = serde_json::to_string(&req).map_err(|e| {
                ProbeError::Internal(format!("serialize checkDatabaseEncryptionKey: {e}"))
            })?;
            send_raw(client_id, &s)?;
            *check_key_sent = true;
        }
        "authorizationStateWaitPhoneNumber" => {
            try_start_proxy_ping(
                client_id,
                proxy,
                add_proxy_extra,
                ping_extra,
                ping_result,
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// Register the proxy with TDLib (`addProxy`), then `pingProxy(proxy_id)` when the `proxy` object returns.
fn try_start_proxy_ping(
    client_id: i32,
    proxy: &ProxyConfig,
    add_proxy_extra: &mut Option<String>,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
) -> Result<(), ProbeError> {
    if ping_result.is_some() || add_proxy_extra.is_some() || ping_extra.is_some() {
        return Ok(());
    }
    let extra = next_extra("addProxy");
    *add_proxy_extra = Some(extra.clone());
    let req = build_add_proxy(&extra, proxy)?;
    let s = serde_json::to_string(&req)
        .map_err(|e| ProbeError::Internal(format!("serialize addProxy: {e}")))?;
    send_raw(client_id, &s)?;
    Ok(())
}

fn on_add_proxy_response(
    client_id: i32,
    v: &Value,
    add_proxy_extra: &mut Option<String>,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
) -> Result<(), ProbeError> {
    let id = v
        .get("id")
        .and_then(|x| x.as_i64())
        .and_then(|i| i32::try_from(i).ok())
        .ok_or_else(|| ProbeError::Internal("addProxy response missing id".into()))?;
    *add_proxy_extra = None;
    send_ping_with_proxy_id(client_id, id, ping_extra, ping_result)
}

fn send_ping_with_proxy_id(
    client_id: i32,
    proxy_id: i32,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
) -> Result<(), ProbeError> {
    if ping_result.is_some() || ping_extra.is_some() {
        return Ok(());
    }
    let extra = next_extra("pingProxy");
    *ping_extra = Some(extra.clone());
    let req = json!({
        "@type": "pingProxy",
        "proxy_id": proxy_id,
        "@extra": extra,
    });
    let s = serde_json::to_string(&req)
        .map_err(|e| ProbeError::Internal(format!("serialize pingProxy: {e}")))?;
    send_raw(client_id, &s)?;
    Ok(())
}

fn copy_log_lines() -> Vec<String> {
    TD_LOG_LINES
        .try_lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn send_close(client_id: i32) -> Result<(), ProbeError> {
    let extra = next_extra("close");
    let req = json!({ "@type": "close", "@extra": extra });
    let s = serde_json::to_string(&req)
        .map_err(|e| ProbeError::Internal(format!("serialize close: {e}")))?;
    send_raw(client_id, &s)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let Some(line) = receive_json_until(deadline, Duration::from_millis(200)) else {
            continue;
        };
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if !client_matches(&v, client_id) {
                continue;
            }
            let typ = json_type_name(&v).unwrap_or("");
            if typ == "updateAuthorizationState" {
                if let Some(st) = auth_state_from_update(&v) {
                    if st == "authorizationStateClosed" {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Clear the log callback before `close` so TDLib cannot invoke our callback mid-teardown.
fn td_shutdown_session(client_id: i32) {
    tdjson_sys::set_log_callback(0, None);
    let _ = send_close(client_id);
}

fn path_to_tdlib_string(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn build_set_tdlib_parameters(
    extra: &str,
    database_directory: &str,
    files_directory: &str,
    creds: &TdlibCredentials,
) -> Value {
    // TDLib 1.8+ (pinned v1.8.0): `setTdlibParameters` takes a single `parameters` object
    // (`tdlibParameters` in td_api.tl), not flat fields on the request.
    json!({
        "@type": "setTdlibParameters",
        "@extra": extra,
        "parameters": {
            "@type": "tdlibParameters",
            "use_test_dc": false,
            "database_directory": database_directory,
            "files_directory": files_directory,
            "use_file_database": false,
            "use_chat_info_database": false,
            "use_message_database": false,
            "use_secret_chats": false,
            "api_id": creds.api_id,
            "api_hash": creds.api_hash,
            "system_language_code": "en",
            "device_model": "tg-proxy-check",
            "system_version": "",
            "application_version": env!("CARGO_PKG_VERSION"),
            "enable_storage_optimizer": false,
            "ignore_file_names": false,
        }
    })
}

fn build_add_proxy(extra: &str, proxy: &ProxyConfig) -> Result<Value, ProbeError> {
    let proxy_type = proxy_type_json(proxy)?;
    Ok(json!({
        "@type": "addProxy",
        "@extra": extra,
        "server": proxy.server,
        "port": i32::from(proxy.port),
        "enable": true,
        "type": proxy_type,
    }))
}

fn proxy_type_json(proxy: &ProxyConfig) -> Result<Value, ProbeError> {
    match proxy.kind {
        ProxyKind::Mtproto => {
            let secret = proxy.mtproto_secret.as_ref().ok_or_else(|| {
                ProbeError::Internal("MTProto secret missing in ProxyConfig".into())
            })?;
            Ok(json!({
                "@type": "proxyTypeMtproto",
                "secret": secret,
            }))
        }
        ProxyKind::Socks5 => {
            let mut obj = serde_json::Map::new();
            obj.insert("@type".into(), json!("proxyTypeSocks5"));
            if let Some(u) = &proxy.socks_username {
                obj.insert("username".into(), json!(u));
            }
            if let Some(p) = &proxy.socks_password {
                obj.insert("password".into(), json!(p));
            }
            Ok(Value::Object(obj))
        }
    }
}
