//! TDLib multiplexed JSON client: `td_create_client_id` / `td_send` / `td_receive`.
//!
//! Current pinned TDLib uses **`addProxy`** with a nested **`proxy`** object, then **`pingProxy`**
//! with the same **`proxy`** shape (not `proxy_id`).
//!
//! Without `--auth-session`, the add/ping sequence starts once TDLib reports
//! `authorizationStateWaitPhoneNumber`, or right after a successful `checkDatabaseEncryptionKey`
//! `ok`. With `--auth-session`, `addProxy` runs after the DB encryption check so login traffic can
//! use the proxy; `pingProxy` runs only after `authorizationStateReady`.

use super::tdjson_sys;
use super::{TdlibCredentials, TdlibProbeSettings};
use crate::error::{ProbeError, ProbeTimeoutContext};
use crate::output::{
    success_interpretation, wall_ms, Interpretation, ProbeReport, SponsoredReport,
    SubscriptionReport,
};
use crate::proxy_link::{ProxyConfig, ProxyKind};
use serde_json::{json, Value};
use std::ffi::CStr;
use std::io::{self, Write};
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
    if let Some(s) = v
        .pointer("/authorization_state/@type")
        .and_then(|t| t.as_str())
    {
        return Some(s.to_string());
    }
    let inner = v.get("authorization_state").or_else(|| v.get("authorizationState"))?;
    inner
        .get("@type")
        .or_else(|| inner.get("type"))
        .and_then(|t| t.as_str())
        .map(String::from)
}

fn probe_timeout_err(
    _deadline: Instant,
    probe_start: Instant,
    probe_start_wall_ms: u128,
    auth_states_seen: &[String],
) -> ProbeError {
    ProbeError::Timeout(ProbeTimeoutContext {
        probe_start_wall_ms,
        probe_end_wall_ms: wall_ms(),
        wall_duration: probe_start.elapsed(),
        auth_states_seen: auth_states_seen.to_vec(),
        tdlib_log_lines: copy_log_lines(),
    })
}

fn ensure_before_auth_prompt(
    deadline: Instant,
    probe_start: Instant,
    probe_start_wall_ms: u128,
    auth_states_seen: &[String],
) -> Result<(), ProbeError> {
    if Instant::now() >= deadline {
        Err(probe_timeout_err(
            deadline,
            probe_start,
            probe_start_wall_ms,
            auth_states_seen,
        ))
    } else {
        Ok(())
    }
}

/// Prompt on stderr, read one line from stdin (trimmed). Used only when `--auth-session` is set.
fn read_auth_line(
    prompt: &str,
    deadline: Instant,
    probe_start: Instant,
    probe_start_wall_ms: u128,
    auth_states_seen: &[String],
) -> Result<String, ProbeError> {
    ensure_before_auth_prompt(
        deadline,
        probe_start,
        probe_start_wall_ms,
        auth_states_seen,
    )?;
    eprint!("{}", prompt);
    io::stderr().flush().map_err(|e| {
        ProbeError::Internal(format!("flush stderr before auth prompt: {e}"))
    })?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| ProbeError::Internal(format!("read stdin: {e}")))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(ProbeError::Internal(
            "empty input (authentication cancelled)".into(),
        ));
    }
    Ok(trimmed.to_string())
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

    let (_temp_holder, db_dir, files_dir) = if let Some(root) = settings.auth_session.as_ref() {
        let files_dir = root.join("tg-proxy-check-files");
        std::fs::create_dir_all(root)
            .map_err(|e| ProbeError::Internal(format!("create auth_session database dir: {e}")))?;
        std::fs::create_dir_all(&files_dir).map_err(|e| {
            ProbeError::Internal(format!("create auth_session files directory: {e}"))
        })?;
        (None::<tempfile::TempDir>, root.clone(), files_dir)
    } else {
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
        (Some(temp), db_dir, files_dir)
    };

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
    let mut add_proxy_extra: Option<String> = None;
    let mut ping_extra: Option<String> = None;
    let mut ping_result: Option<Result<f64, String>> = None;
    let mut authorization_state: Option<String> = None;
    // `--auth-session`: addProxy after `setTdlibParameters` ok; ping only after Ready.
    let mut interactive_proxy_registered = false;
    let interactive_auth = settings.auth_session.is_some();

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

        note_auth_from_value(&v, &mut authorization_state);

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
                        interactive_auth,
                        deadline,
                        start_instant,
                        start_wall,
                        &auth_states_seen,
                        &mut set_params_sent,
                        &mut add_proxy_extra,
                        &mut ping_extra,
                        &mut ping_result,
                        &mut interactive_proxy_registered,
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
                    interactive_auth,
                    deadline,
                    start_instant,
                    start_wall,
                    &auth_states_seen,
                    &mut set_params_sent,
                    &mut add_proxy_extra,
                    &mut ping_extra,
                    &mut ping_result,
                    &mut interactive_proxy_registered,
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
                if ex.starts_with("setAuthenticationPhoneNumber-")
                    || ex.starts_with("checkAuthenticationCode-")
                    || ex.starts_with("checkAuthenticationPassword-")
                    || ex.starts_with("setAuthenticationEmailAddress-")
                    || ex.starts_with("checkAuthenticationEmailCode-")
                {
                    td_shutdown_session(client_id);
                    return Err(ProbeError::TdlibInit(msg));
                }
            }
            "ok" => {
                let ex = extra_for_match(&v).unwrap_or_default();
                if ex.starts_with("setTdlibParameters-") && interactive_auth {
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
            "addedProxy" | "proxy" => {
                let ex = extra_for_match(&v).unwrap_or_default();
                if Some(ex.as_str()) == add_proxy_extra.as_deref() {
                    if let Err(e) = on_add_proxy_response(
                        client_id,
                        proxy,
                        interactive_auth,
                        &authorization_state,
                        &mut interactive_proxy_registered,
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

    let (sponsored, subscription) = promo_and_subscription_after_ping(
        client_id,
        deadline,
        settings,
        proxy,
        &mut authorization_state,
    );

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
                subscription,
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
            subscription,
        }),
    }
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if v.last().map(|x| x.as_str()) != Some(s) {
        v.push(s.to_string());
    }
}

fn note_auth_from_value(v: &Value, auth: &mut Option<String>) {
    if let Some(st) = auth_state_from_update(v) {
        *auth = Some(st);
        return;
    }
    if let Some(typ) = json_type_name(v) {
        if typ.starts_with("authorizationState") {
            *auth = Some(typ.to_string());
        }
    }
}

/// MTProto + `--auth-session`: wait for `authorizationStateReady`, then `getPromoData` (TDLib JSON name).
fn promo_and_subscription_after_ping(
    client_id: i32,
    deadline: Instant,
    settings: &TdlibProbeSettings,
    proxy: &ProxyConfig,
    authorization_state: &mut Option<String>,
) -> (SponsoredReport, SubscriptionReport) {
    if settings.auth_session.is_none() || proxy.kind != ProxyKind::Mtproto {
        return (
            SponsoredReport::unknown_unchecked(),
            SubscriptionReport::unchecked(),
        );
    }
    if !wait_for_authorization_ready(client_id, deadline, authorization_state) {
        return (
            SponsoredReport::unknown_unchecked(),
            SubscriptionReport::unchecked(),
        );
    }
    run_get_promo_flow(client_id, deadline)
}

fn wait_for_authorization_ready(
    client_id: i32,
    deadline: Instant,
    auth: &mut Option<String>,
) -> bool {
    if auth.as_deref() == Some("authorizationStateReady") {
        return true;
    }
    let extra = next_extra("getAuthorizationState");
    let req = match serde_json::to_string(&json!({
        "@type": "getAuthorizationState",
        "@extra": &extra,
    })) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if send_raw(client_id, &req).is_err() {
        return false;
    }
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
        note_auth_from_value(&v, auth);
        if auth.as_deref() == Some("authorizationStateReady") {
            return true;
        }
        if json_type_name(&v) == Some("error") && extra_for_match(&v).as_deref() == Some(extra.as_str())
        {
            return false;
        }
        if let Some(t) = json_type_name(&v) {
            if extra_for_match(&v).as_deref() == Some(extra.as_str())
                && t.starts_with("authorizationState")
            {
                *auth = Some(t.to_string());
                if t == "authorizationStateReady" {
                    return true;
                }
            }
        }
    }
    auth.as_deref() == Some("authorizationStateReady")
}

fn run_get_promo_flow(client_id: i32, deadline: Instant) -> (SponsoredReport, SubscriptionReport) {
    let extra = next_extra("getPromoData");
    let req = match serde_json::to_string(&json!({
        "@type": "getPromoData",
        "@extra": &extra,
    })) {
        Ok(s) => s,
        Err(_) => {
            return (
                SponsoredReport::unknown_unchecked(),
                SubscriptionReport::checked_no_join_info(),
            );
        }
    };
    if send_raw(client_id, &req).is_err() {
        return (
            SponsoredReport::unknown_unchecked(),
            SubscriptionReport::checked_no_join_info(),
        );
    }
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
        if json_type_name(&v) == Some("error") && extra_for_match(&v).as_deref() == Some(extra.as_str())
        {
            return (
                SponsoredReport::unknown_unchecked(),
                SubscriptionReport::checked_no_join_info(),
            );
        }
        if extra_for_match(&v).as_deref() == Some(extra.as_str()) {
            return parse_get_promo_response(&v, client_id, deadline);
        }
    }
    (
        SponsoredReport::unknown_unchecked(),
        SubscriptionReport::checked_no_join_info(),
    )
}

fn parse_get_promo_response(
    v: &Value,
    client_id: i32,
    deadline: Instant,
) -> (SponsoredReport, SubscriptionReport) {
    match v.get("@type").and_then(|t| t.as_str()) {
        Some("promoDataEmpty") => (
            SponsoredReport::no_promo(),
            SubscriptionReport::checked_no_join_info(),
        ),
        Some("promoData") => {
            let Some(peer) = v.get("peer").filter(|p| !p.is_null()) else {
                return (
                    SponsoredReport::unknown_unchecked(),
                    SubscriptionReport::checked_no_join_info(),
                );
            };
            let Some(display_id) = peer_display_id(peer) else {
                return (
                    SponsoredReport::unknown_unchecked(),
                    SubscriptionReport::checked_no_join_info(),
                );
            };
            let sponsored = SponsoredReport::yes_with_peer_id(display_id);
            let Some(chat_id) = tdlib_chat_id_for_peer(peer) else {
                return (
                    sponsored,
                    SubscriptionReport::checked_no_join_info(),
                );
            };
            let Some(user_id) = fetch_my_user_id(client_id, deadline) else {
                return (
                    sponsored,
                    SubscriptionReport::checked_no_join_info(),
                );
            };
            let joined = fetch_am_i_member(client_id, deadline, chat_id, user_id);
            let sub = SubscriptionReport {
                checked: true,
                joined,
            };
            (sponsored, sub)
        }
        _ => (
            SponsoredReport::unknown_unchecked(),
            SubscriptionReport::checked_no_join_info(),
        ),
    }
}

fn peer_display_id(peer: &Value) -> Option<i64> {
    let t = peer.get("@type")?.as_str()?;
    match t {
        "peerChannel" => peer.get("channel_id").and_then(|x| x.as_i64()),
        "peerChat" => peer.get("chat_id").and_then(|x| x.as_i64()),
        "peerUser" => peer.get("user_id").and_then(|x| x.as_i64()),
        _ => None,
    }
}

fn tdlib_chat_id_for_peer(peer: &Value) -> Option<i64> {
    let t = peer.get("@type")?.as_str()?;
    match t {
        "peerChannel" => {
            let cid = peer.get("channel_id").and_then(|x| x.as_i64())?;
            Some(-(1_000_000_000_000_i64 + cid))
        }
        "peerChat" => peer.get("chat_id").and_then(|x| x.as_i64()),
        "peerUser" => peer.get("user_id").and_then(|x| x.as_i64()),
        _ => None,
    }
}

fn fetch_my_user_id(client_id: i32, deadline: Instant) -> Option<i64> {
    let extra = next_extra("getMe");
    let req = serde_json::to_string(&json!({ "@type": "getMe", "@extra": &extra })).ok()?;
    send_raw(client_id, &req).ok()?;
    while Instant::now() < deadline {
        let line = receive_json_until(deadline, Duration::from_millis(500))?;
        let v: Value = serde_json::from_str(&line).ok()?;
        if !client_matches(&v, client_id) {
            continue;
        }
        if json_type_name(&v) == Some("error") && extra_for_match(&v).as_deref() == Some(extra.as_str())
        {
            return None;
        }
        if extra_for_match(&v).as_deref() == Some(extra.as_str()) && json_type_name(&v) == Some("user")
        {
            return v.get("id").and_then(|x| x.as_i64());
        }
    }
    None
}

fn fetch_am_i_member(client_id: i32, deadline: Instant, chat_id: i64, user_id: i64) -> Option<bool> {
    let extra = next_extra("getChatMember");
    let req = serde_json::to_string(&json!({
        "@type": "getChatMember",
        "chat_id": chat_id,
        "member_id": { "@type": "messageSenderUser", "user_id": user_id },
        "@extra": &extra,
    }))
    .ok()?;
    send_raw(client_id, &req).ok()?;
    while Instant::now() < deadline {
        let line = receive_json_until(deadline, Duration::from_millis(500))?;
        let v: Value = serde_json::from_str(&line).ok()?;
        if !client_matches(&v, client_id) {
            continue;
        }
        if json_type_name(&v) == Some("error") && extra_for_match(&v).as_deref() == Some(extra.as_str())
        {
            return None;
        }
        if extra_for_match(&v).as_deref() == Some(extra.as_str())
            && json_type_name(&v) == Some("chatMember")
        {
            return chat_member_is_joined(&v);
        }
    }
    None
}

fn chat_member_is_joined(v: &Value) -> Option<bool> {
    let st = v.pointer("/status/@type")?.as_str()?;
    Some(match st {
        "chatMemberStatusLeft" | "chatMemberStatusBanned" => false,
        "chatMemberStatusMember"
        | "chatMemberStatusAdministrator"
        | "chatMemberStatusRestricted"
        | "chatMemberStatusCreator" => true,
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn handle_auth_state(
    client_id: i32,
    proxy: &ProxyConfig,
    creds: &TdlibCredentials,
    db_path: &str,
    files_path: &str,
    state: &str,
    interactive_auth: bool,
    deadline: Instant,
    probe_start: Instant,
    probe_start_wall_ms: u128,
    auth_states_seen: &[String],
    set_params_sent: &mut bool,
    add_proxy_extra: &mut Option<String>,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
    interactive_proxy_registered: &mut bool,
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
        "authorizationStateWaitPhoneNumber" => {
            if interactive_auth {
                let phone = read_auth_line(
                    "Enter phone number:\n",
                    deadline,
                    probe_start,
                    probe_start_wall_ms,
                    auth_states_seen,
                )?;
                let extra = next_extra("setAuthenticationPhoneNumber");
                let req = json!({
                    "@type": "setAuthenticationPhoneNumber",
                    "phone_number": phone,
                    "@extra": extra,
                });
                let s = serde_json::to_string(&req).map_err(|e| {
                    ProbeError::Internal(format!("serialize setAuthenticationPhoneNumber: {e}"))
                })?;
                send_raw(client_id, &s)?;
            } else {
                try_start_proxy_ping(
                    client_id,
                    proxy,
                    add_proxy_extra,
                    ping_extra,
                    ping_result,
                )?;
            }
        }
        "authorizationStateWaitCode" if interactive_auth => {
            let code = read_auth_line(
                "Enter code:\n",
                deadline,
                probe_start,
                probe_start_wall_ms,
                auth_states_seen,
            )?;
            let extra = next_extra("checkAuthenticationCode");
            let req = json!({
                "@type": "checkAuthenticationCode",
                "code": code,
                "@extra": extra,
            });
            let s = serde_json::to_string(&req)
                .map_err(|e| ProbeError::Internal(format!("serialize checkAuthenticationCode: {e}")))?;
            send_raw(client_id, &s)?;
        }
        "authorizationStateWaitEmailAddress" if interactive_auth => {
            let email = read_auth_line(
                "Enter email address:\n",
                deadline,
                probe_start,
                probe_start_wall_ms,
                auth_states_seen,
            )?;
            let extra = next_extra("setAuthenticationEmailAddress");
            let req = json!({
                "@type": "setAuthenticationEmailAddress",
                "email_address": email,
                "@extra": extra,
            });
            let s = serde_json::to_string(&req).map_err(|e| {
                ProbeError::Internal(format!("serialize setAuthenticationEmailAddress: {e}"))
            })?;
            send_raw(client_id, &s)?;
        }
        "authorizationStateWaitEmailCode" if interactive_auth => {
            let code = read_auth_line(
                "Enter email code:\n",
                deadline,
                probe_start,
                probe_start_wall_ms,
                auth_states_seen,
            )?;
            let extra = next_extra("checkAuthenticationEmailCode");
            let req = json!({
                "@type": "checkAuthenticationEmailCode",
                "code": {
                    "@type": "emailAddressAuthenticationCode",
                    "code": code,
                },
                "@extra": extra,
            });
            let s = serde_json::to_string(&req).map_err(|e| {
                ProbeError::Internal(format!("serialize checkAuthenticationEmailCode: {e}"))
            })?;
            send_raw(client_id, &s)?;
        }
        "authorizationStateWaitPassword" if interactive_auth => {
            let password = read_auth_line(
                "Enter 2FA password:\n",
                deadline,
                probe_start,
                probe_start_wall_ms,
                auth_states_seen,
            )?;
            let extra = next_extra("checkAuthenticationPassword");
            let req = json!({
                "@type": "checkAuthenticationPassword",
                "password": password,
                "@extra": extra,
            });
            let s = serde_json::to_string(&req).map_err(|e| {
                ProbeError::Internal(format!("serialize checkAuthenticationPassword: {e}"))
            })?;
            send_raw(client_id, &s)?;
        }
        "authorizationStateReady" if interactive_auth => {
            if *interactive_proxy_registered {
                send_ping_with_proxy(client_id, proxy, ping_extra, ping_result)?;
            } else if add_proxy_extra.is_none() && ping_extra.is_none() {
                try_start_proxy_ping(
                    client_id,
                    proxy,
                    add_proxy_extra,
                    ping_extra,
                    ping_result,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Register the proxy with TDLib (`addProxy`), then `pingProxy` with the same `proxy` object when `addProxy` returns.
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
    proxy: &ProxyConfig,
    interactive_auth: bool,
    authorization_state: &Option<String>,
    interactive_proxy_registered: &mut bool,
    add_proxy_extra: &mut Option<String>,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
) -> Result<(), ProbeError> {
    *add_proxy_extra = None;
    if interactive_auth {
        *interactive_proxy_registered = true;
        if authorization_state.as_deref() == Some("authorizationStateReady") {
            send_ping_with_proxy(client_id, proxy, ping_extra, ping_result)?;
        }
    } else {
        send_ping_with_proxy(client_id, proxy, ping_extra, ping_result)?;
    }
    Ok(())
}

fn send_ping_with_proxy(
    client_id: i32,
    proxy: &ProxyConfig,
    ping_extra: &mut Option<String>,
    ping_result: &mut Option<Result<f64, String>>,
) -> Result<(), ProbeError> {
    if ping_result.is_some() || ping_extra.is_some() {
        return Ok(());
    }
    let extra = next_extra("pingProxy");
    *ping_extra = Some(extra.clone());
    let proxy_body = build_td_proxy_object(proxy)?;
    let req = json!({
        "@type": "pingProxy",
        "proxy": proxy_body,
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
    let application_version = std::env::var("TG_APPLICATION_VERSION")
        .unwrap_or_else(|_| "5.13.1".to_string());
    json!({
        "@type": "setTdlibParameters",
        "@extra": extra,
        "use_test_dc": false,
        "database_directory": database_directory,
        "files_directory": files_directory,
        "database_encryption_key": "",
        "use_file_database": false,
        "use_chat_info_database": false,
        "use_message_database": false,
        "use_secret_chats": false,
        "api_id": creds.api_id,
        "api_hash": creds.api_hash,
        "system_language_code": "en",
        "device_model": "Desktop",
        "system_version": "",
        "application_version": application_version,
    })
}

fn build_td_proxy_object(proxy: &ProxyConfig) -> Result<Value, ProbeError> {
    let proxy_type = proxy_type_json(proxy)?;
    Ok(json!({
        "@type": "proxy",
        "server": proxy.server,
        "port": i32::from(proxy.port),
        "type": proxy_type,
    }))
}

fn build_add_proxy(extra: &str, proxy: &ProxyConfig) -> Result<Value, ProbeError> {
    let p = build_td_proxy_object(proxy)?;
    Ok(json!({
        "@type": "addProxy",
        "@extra": extra,
        "proxy": p,
        "enable": true,
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
