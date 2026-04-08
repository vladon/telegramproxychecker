//! Text and JSON output for probe results.
//!
//! ## Latency semantics
//!
//! The reported `latency_ms` value comes from TDLib **`pingProxy`**: it is the time for a
//! request to go **client → proxy → Telegram → back**, as measured by TDLib. It is **not** an
//! ICMP ping, and it is **not** the raw TCP connect time to the proxy alone.

use crate::proxy_link::{redact_sensitive_query_in_link, ProxyConfig, ProxyKind};
use serde::Serialize;
use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Human-facing interpretation of latency or failure mode (verbose).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interpretation {
    ReachableLowLatency,
    ReachableModerateLatency,
    ReachableHighLatency,
    ProxyReachableTelegramUnavailable,
    InvalidProxyLink,
    Timeout,
    TdlibInitializationFailure,
    InternalUnexpected,
}

impl Interpretation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ReachableLowLatency => "reachable, low latency",
            Self::ReachableModerateLatency => "reachable, moderate latency",
            Self::ReachableHighLatency => "reachable, high latency",
            Self::ProxyReachableTelegramUnavailable => "proxy reachable but Telegram unavailable",
            Self::InvalidProxyLink => "invalid proxy link",
            Self::Timeout => "timeout",
            Self::TdlibInitializationFailure => "tdlib initialization failure",
            Self::InternalUnexpected => "internal unexpected error",
        }
    }
}

/// Latency bands for interpretation (milliseconds).
pub const LATENCY_LOW_MAX_MS: u64 = 200;
pub const LATENCY_MODERATE_MAX_MS: u64 = 800;

fn interpret_latency(latency_ms: u64) -> Interpretation {
    if latency_ms < LATENCY_LOW_MAX_MS {
        Interpretation::ReachableLowLatency
    } else if latency_ms <= LATENCY_MODERATE_MAX_MS {
        Interpretation::ReachableModerateLatency
    } else {
        Interpretation::ReachableHighLatency
    }
}

/// Full probe report used for rendering.
#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error_message: Option<String>,
    pub interpretation: Interpretation,
    pub auth_states_seen: Vec<String>,
    pub tdlib_log_lines: Vec<String>,
    pub probe_start_wall_ms: u128,
    pub probe_end_wall_ms: u128,
    pub wall_duration: Duration,
    pub tdlib_reported_seconds: Option<f64>,
}

impl ProbeReport {
    pub fn from_probe_failure(err: &crate::error::ProbeError) -> Self {
        let now = wall_ms();
        match err {
            crate::error::ProbeError::Timeout(ctx) => ProbeReport {
                ok: false,
                latency_ms: None,
                error_message: Some("Timeout".to_string()),
                interpretation: Interpretation::Timeout,
                auth_states_seen: ctx.auth_states_seen.clone(),
                tdlib_log_lines: ctx.tdlib_log_lines.clone(),
                probe_start_wall_ms: ctx.probe_start_wall_ms,
                probe_end_wall_ms: ctx.probe_end_wall_ms,
                wall_duration: ctx.wall_duration,
                tdlib_reported_seconds: None,
            },
            crate::error::ProbeError::TdlibInit(s) => ProbeReport {
                ok: false,
                latency_ms: None,
                error_message: Some(s.clone()),
                interpretation: Interpretation::TdlibInitializationFailure,
                auth_states_seen: Vec::new(),
                tdlib_log_lines: Vec::new(),
                probe_start_wall_ms: now,
                probe_end_wall_ms: now,
                wall_duration: Duration::ZERO,
                tdlib_reported_seconds: None,
            },
            crate::error::ProbeError::Internal(s) => ProbeReport {
                ok: false,
                latency_ms: None,
                error_message: Some(s.clone()),
                interpretation: Interpretation::InternalUnexpected,
                auth_states_seen: Vec::new(),
                tdlib_log_lines: Vec::new(),
                probe_start_wall_ms: now,
                probe_end_wall_ms: now,
                wall_duration: Duration::ZERO,
                tdlib_reported_seconds: None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderOpts {
    pub verbose: bool,
    pub json: bool,
}

#[derive(Serialize)]
struct JsonOk<'a> {
    ok: bool,
    proxy_type: &'a str,
    server: &'a str,
    port: u16,
    latency_ms: u64,
    message: &'static str,
}

#[derive(Serialize)]
struct JsonFail<'a> {
    ok: bool,
    proxy_type: &'a str,
    server: &'a str,
    port: u16,
    error: &'a str,
    message: &'static str,
}

pub fn render(
    proxy: &ProxyConfig,
    report: &ProbeReport,
    opts: &RenderOpts,
) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    if opts.json {
        render_json(proxy, report, &mut stdout)?;
    } else if opts.verbose {
        render_verbose_text(proxy, report, &mut stdout)?;
    } else {
        render_default_text(proxy, report, &mut stdout)?;
    }
    Ok(())
}

fn proxy_type_str(kind: ProxyKind) -> &'static str {
    match kind {
        ProxyKind::Mtproto => "mtproto",
        ProxyKind::Socks5 => "socks5",
    }
}

fn render_default_text(
    proxy: &ProxyConfig,
    report: &ProbeReport,
    w: &mut impl Write,
) -> io::Result<()> {
    let t = proxy_type_str(proxy.kind);
    if report.ok {
        let ms = report.latency_ms.unwrap_or(0);
        writeln!(
            w,
            "OK type={} server={} port={} latency_ms={}",
            t, proxy.server, proxy.port, ms
        )?;
    } else {
        let err = report
            .error_message
            .as_deref()
            .unwrap_or("Telegram unreachable through proxy");
        let escaped = escape_quotes(err);
        writeln!(
            w,
            "FAIL type={} server={} port={} error=\"{}\"",
            t, proxy.server, proxy.port, escaped
        )?;
    }
    Ok(())
}

fn escape_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// TDLib may echo JSON fragments; drop lines that look like they contain credentials.
fn scrub_tdlib_log_line(line: &str) -> String {
    let low = line.to_ascii_lowercase();
    if low.contains("password")
        || low.contains("secret")
        || low.contains("api_hash")
        || low.contains("proxytype")
    {
        "[omitted: possible credential substring in tdlib log]".to_string()
    } else {
        line.to_string()
    }
}

fn render_verbose_text(
    proxy: &ProxyConfig,
    report: &ProbeReport,
    w: &mut impl Write,
) -> io::Result<()> {
    writeln!(
        w,
        "input_link={}",
        redact_sensitive_query_in_link(&proxy.original_input)
    )?;
    writeln!(w, "detected_type={}", proxy_type_str(proxy.kind))?;
    writeln!(w, "server={}", proxy.server)?;
    writeln!(w, "port={}", proxy.port)?;
    match proxy.kind {
        ProxyKind::Mtproto => {
            let len = proxy.mtproto_secret.as_ref().map(|s| s.len()).unwrap_or(0);
            writeln!(w, "mtproto_secret_len={}", len)?;
        }
        ProxyKind::Socks5 => {
            writeln!(
                w,
                "socks5_username_present={}",
                proxy.socks_username.is_some()
            )?;
            writeln!(
                w,
                "socks5_password_present={}",
                proxy.socks_password.is_some()
            )?;
        }
    }
    writeln!(w, "probe_start_wall_ms={}", report.probe_start_wall_ms)?;
    writeln!(w, "probe_end_wall_ms={}", report.probe_end_wall_ms)?;
    writeln!(w, "wall_duration_ms={}", report.wall_duration.as_millis())?;
    if let Some(sec) = report.tdlib_reported_seconds {
        writeln!(w, "tdlib_seconds={}", sec)?;
    }
    if let Some(ms) = report.latency_ms {
        writeln!(w, "tdlib_latency_ms={}", ms)?;
    }
    writeln!(w, "authorization_states_seen:")?;
    for s in &report.auth_states_seen {
        writeln!(w, "  - {}", s)?;
    }
    if !report.tdlib_log_lines.is_empty() {
        writeln!(w, "tdlib_log:")?;
        for line in &report.tdlib_log_lines {
            writeln!(w, "  {}", scrub_tdlib_log_line(line))?;
        }
    }
    writeln!(w, "interpretation={}", report.interpretation.as_str())?;
    render_default_text(proxy, report, w)?;
    Ok(())
}

fn render_json(proxy: &ProxyConfig, report: &ProbeReport, w: &mut impl Write) -> io::Result<()> {
    let t = proxy_type_str(proxy.kind);
    let line = if report.ok {
        let ms = report.latency_ms.unwrap_or(0);
        serde_json::to_string(&JsonOk {
            ok: true,
            proxy_type: t,
            server: proxy.server.as_str(),
            port: proxy.port,
            latency_ms: ms,
            message: "Telegram reachable through proxy",
        })
    } else {
        let err = report
            .error_message
            .as_deref()
            .unwrap_or("Telegram unreachable through proxy");
        serde_json::to_string(&JsonFail {
            ok: false,
            proxy_type: t,
            server: proxy.server.as_str(),
            port: proxy.port,
            error: err,
            message: "Telegram unreachable through proxy",
        })
    }
    .map_err(io::Error::other)?;
    writeln!(w, "{}", line)?;
    Ok(())
}

pub fn wall_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Build interpretation for a successful probe from latency.
pub fn success_interpretation(latency_ms: u64) -> Interpretation {
    interpret_latency(latency_ms)
}
