//! Text and JSON output for probe results.
//!
//! ## Latency semantics
//!
//! The reported `latency_ms` value comes from TDLib **`pingProxy`**: it is the time for a
//! request to go **client → proxy → Telegram → back**, as measured by TDLib. It is **not** an
//! ICMP ping, and it is **not** the raw TCP connect time to the proxy alone.

use crate::error::ProbeError;
use crate::proxy_link::{redact_sensitive_query_in_link, ProxyConfig, ProxyKind};
use serde::Serialize;
use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Sponsored-channel info from TDLib `chatSourceMtprotoProxy` chat positions (JSON always includes this object).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SponsoredStatus {
    Yes,
    No,
    Unknown,
}

impl SponsoredStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SponsoredReport {
    pub status: SponsoredStatus,
    pub channel_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SubscriptionReport {
    pub checked: bool,
    pub joined: Option<bool>,
}

impl SponsoredReport {
    pub fn unknown_unchecked() -> Self {
        Self {
            status: SponsoredStatus::Unknown,
            channel_id: None,
        }
    }

    pub fn no_promo() -> Self {
        Self {
            status: SponsoredStatus::No,
            channel_id: None,
        }
    }

    pub fn yes_with_peer_id(peer_id: i64) -> Self {
        Self {
            status: SponsoredStatus::Yes,
            channel_id: Some(peer_id),
        }
    }
}

impl SubscriptionReport {
    pub const fn unchecked() -> Self {
        Self {
            checked: false,
            joined: None,
        }
    }

    pub const fn checked_no_join_info() -> Self {
        Self {
            checked: true,
            joined: None,
        }
    }

    pub const fn checked_joined(j: bool) -> Self {
        Self {
            checked: true,
            joined: Some(j),
        }
    }
}

fn default_promo_on_probe_failure() -> (SponsoredReport, SubscriptionReport) {
    (
        SponsoredReport::unknown_unchecked(),
        SubscriptionReport::unchecked(),
    )
}

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
    pub sponsored: SponsoredReport,
    pub subscription: SubscriptionReport,
}

impl ProbeReport {
    pub fn from_probe_failure(err: &ProbeError, _proxy: &ProxyConfig) -> Self {
        let now = wall_ms();
        let (sponsored, subscription) = default_promo_on_probe_failure();
        match err {
            ProbeError::Timeout(ctx) => ProbeReport {
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
                sponsored,
                subscription,
            },
            ProbeError::TdlibInit(s) => ProbeReport {
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
                sponsored,
                subscription,
            },
            ProbeError::Internal(s) => ProbeReport {
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
                sponsored,
                subscription,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderOpts {
    pub verbose: bool,
    pub json: bool,
    /// Wall-clock probe budget from CLI (verbose diagnostics only).
    pub probe_timeout_sec: u64,
}

#[derive(Serialize)]
struct JsonOk<'a> {
    ok: bool,
    proxy_type: &'a str,
    server: &'a str,
    port: u16,
    latency_ms: u64,
    message: &'static str,
    sponsored: &'a SponsoredReport,
    subscription: &'a SubscriptionReport,
}

#[derive(Serialize)]
struct JsonFail<'a> {
    ok: bool,
    proxy_type: &'a str,
    server: &'a str,
    port: u16,
    error: &'a str,
    message: &'a str,
    sponsored: &'a SponsoredReport,
    subscription: &'a SubscriptionReport,
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
        render_verbose_text(proxy, report, opts, &mut stdout)?;
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
        || low.contains("token")
    {
        "[omitted: possible credential substring in tdlib log]".to_string()
    } else {
        line.to_string()
    }
}

fn render_verbose_text(
    proxy: &ProxyConfig,
    report: &ProbeReport,
    opts: &RenderOpts,
    w: &mut impl Write,
) -> io::Result<()> {
    writeln!(
        w,
        "input_link={}",
        redact_sensitive_query_in_link(&proxy.original_input)
    )?;
    writeln!(w, "probe_timeout_sec={}", opts.probe_timeout_sec)?;
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
    writeln!(
        w,
        "sponsored_status={} sponsored_channel_id={}",
        report.sponsored.status.as_str(),
        report
            .sponsored
            .channel_id
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".into())
    )?;
    writeln!(
        w,
        "subscription_checked={} subscription_joined={}",
        report.subscription.checked,
        report
            .subscription
            .joined
            .map(|b| b.to_string())
            .unwrap_or_else(|| "null".into())
    )?;
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
            sponsored: &report.sponsored,
            subscription: &report.subscription,
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
            message: err,
            sponsored: &report.sponsored,
            subscription: &report.subscription,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy_link::{ProxyConfig, ProxyKind};

    #[test]
    fn escape_quotes_handles_backslash_and_double_quote() {
        assert_eq!(escape_quotes(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape_quotes(r"a\b"), r"a\\b");
    }

    #[test]
    fn scrub_tdlib_log_line_omits_sensitive_substrings() {
        assert_eq!(
            scrub_tdlib_log_line("[v2] connecting"),
            "[v2] connecting"
        );
        assert_eq!(
            scrub_tdlib_log_line("password=foo"),
            "[omitted: possible credential substring in tdlib log]"
        );
        assert_eq!(
            scrub_tdlib_log_line("Bearer token xyz"),
            "[omitted: possible credential substring in tdlib log]"
        );
    }

    #[test]
    fn default_text_ok_and_fail_lines() {
        let proxy = ProxyConfig {
            original_input: String::new(),
            kind: ProxyKind::Socks5,
            server: "1.2.3.4".into(),
            port: 1080,
            mtproto_secret: None,
            socks_username: None,
            socks_password: None,
        };
        let ok_rep = ProbeReport {
            ok: true,
            latency_ms: Some(42),
            error_message: None,
            interpretation: Interpretation::ReachableLowLatency,
            auth_states_seen: vec![],
            tdlib_log_lines: vec![],
            probe_start_wall_ms: 0,
            probe_end_wall_ms: 0,
            wall_duration: Duration::ZERO,
            tdlib_reported_seconds: Some(0.042),
            sponsored: SponsoredReport::unknown_unchecked(),
            subscription: SubscriptionReport::unchecked(),
        };
        let mut buf = Vec::new();
        render_default_text(&proxy, &ok_rep, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(
            s.trim_end(),
            "OK type=socks5 server=1.2.3.4 port=1080 latency_ms=42"
        );

        let fail_rep = ProbeReport {
            ok: false,
            latency_ms: None,
            error_message: Some(r#"bad"msg"#.into()),
            interpretation: Interpretation::Timeout,
            auth_states_seen: vec![],
            tdlib_log_lines: vec![],
            probe_start_wall_ms: 0,
            probe_end_wall_ms: 0,
            wall_duration: Duration::ZERO,
            tdlib_reported_seconds: None,
            sponsored: SponsoredReport::unknown_unchecked(),
            subscription: SubscriptionReport::unchecked(),
        };
        let mut buf = Vec::new();
        render_default_text(&proxy, &fail_rep, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("FAIL type=socks5 server=1.2.3.4 port=1080 error=\""));
        assert!(s.contains(r#"bad\"m"#));
    }

    #[test]
    fn json_includes_sponsored_and_subscription() {
        let proxy = ProxyConfig {
            original_input: String::new(),
            kind: ProxyKind::Mtproto,
            server: "1.2.3.4".into(),
            port: 443,
            mtproto_secret: Some("ab".into()),
            socks_username: None,
            socks_password: None,
        };
        let rep = ProbeReport {
            ok: true,
            latency_ms: Some(320),
            error_message: None,
            interpretation: Interpretation::ReachableModerateLatency,
            auth_states_seen: vec![],
            tdlib_log_lines: vec![],
            probe_start_wall_ms: 0,
            probe_end_wall_ms: 0,
            wall_duration: Duration::ZERO,
            tdlib_reported_seconds: Some(0.32),
            sponsored: SponsoredReport::yes_with_peer_id(123456789_i64),
            subscription: SubscriptionReport::checked_joined(true),
        };
        let mut buf = Vec::new();
        render_json(
            &proxy,
            &rep,
            &mut buf,
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["sponsored"]["status"], "yes");
        assert_eq!(v["sponsored"]["channel_id"], 123456789);
        assert_eq!(v["subscription"]["checked"], true);
        assert_eq!(v["subscription"]["joined"], true);
    }

    #[test]
    fn verbose_includes_timeout_and_redacted_link_line() {
        let proxy = ProxyConfig {
            original_input: "tg://socks?server=h&port=1080&pass=secret1".into(),
            kind: ProxyKind::Socks5,
            server: "h".into(),
            port: 1080,
            mtproto_secret: None,
            socks_username: None,
            socks_password: Some("secret1".into()),
        };
        let rep = ProbeReport {
            ok: true,
            latency_ms: Some(1),
            error_message: None,
            interpretation: Interpretation::ReachableLowLatency,
            auth_states_seen: vec!["authorizationStateWaitPhoneNumber".into()],
            tdlib_log_lines: vec!["[v2] harmless line".into()],
            probe_start_wall_ms: 10,
            probe_end_wall_ms: 20,
            wall_duration: Duration::from_millis(5),
            tdlib_reported_seconds: Some(0.001),
            sponsored: SponsoredReport::unknown_unchecked(),
            subscription: SubscriptionReport::unchecked(),
        };
        let opts = RenderOpts {
            verbose: true,
            json: false,
            probe_timeout_sec: 99,
        };
        let mut buf = Vec::new();
        render_verbose_text(&proxy, &rep, &opts, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("probe_timeout_sec=99"));
        assert!(!s.contains("secret1"));
        assert!(s.to_ascii_lowercase().contains("redacted"));
        assert!(s.contains("authorizationStateWaitPhoneNumber"));
        assert!(s.contains("harmless line"));
    }
}
