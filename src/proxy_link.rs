//! Parse Telegram MTProto and SOCKS5 proxy links into a strongly typed config.
//!
//! Supported:
//! - MTProto: `tg://proxy?...`, `https://t.me/proxy?...`, `http://t.me/proxy?...`
//! - SOCKS5: `tg://socks?...`, `https://t.me/socks?...`, `http://t.me/socks?...`

use std::collections::HashMap;
use thiserror::Error;
use url::Url;

/// Upper bounds for query-derived strings (defensive; normal Telegram links are far smaller).
const MAX_SERVER_LEN: usize = 512;
const MAX_MTSECRET_LEN: usize = 8192;
const MAX_SOCKS_CRED_LEN: usize = 2048;
const MAX_PORT_QUERY_BYTES: usize = 16;

fn field_too_long(field: &'static str, max: usize) -> ParseError {
    ParseError::InvalidUrl(format!("{field} exceeds maximum length ({max} bytes)"))
}

fn reject_control_chars(s: &str, field: &'static str) -> Result<(), ParseError> {
    if s.chars().any(|c| c.is_control()) {
        return Err(ParseError::InvalidUrl(format!(
            "{field} contains control characters"
        )));
    }
    Ok(())
}

/// Kind of proxy encoded in the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyKind {
    Mtproto,
    Socks5,
}

/// Parsed proxy parameters plus the original input string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyConfig {
    pub original_input: String,
    pub kind: ProxyKind,
    pub server: String,
    pub port: u16,
    pub mtproto_secret: Option<String>,
    pub socks_username: Option<String>,
    pub socks_password: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid or unsupported proxy URL: {0}")]
    InvalidUrl(String),

    #[error("unsupported proxy link path or scheme")]
    UnsupportedFormat,

    #[error("missing or empty server parameter")]
    MissingServer,

    #[error("missing or empty port parameter")]
    MissingPort,

    #[error("invalid port (must be 1–65535)")]
    InvalidPort,

    #[error("MTProto proxy requires a non-empty secret")]
    MissingMtprotoSecret,
}

/// Parse a full proxy link string into [`ProxyConfig`].
pub fn parse_proxy_link(input: &str) -> Result<ProxyConfig, ParseError> {
    let original_input = input.to_string();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseError::InvalidUrl("empty input".into()));
    }

    let url = Url::parse(trimmed).map_err(|e| ParseError::InvalidUrl(e.to_string()))?;

    let kind = classify(&url)?;

    let pairs = query_pairs_map(&url);

    let server = pairs
        .get("server")
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or(ParseError::MissingServer)?;

    let port_str = match pairs.get("port").map(String::as_str) {
        None | Some("") => return Err(ParseError::MissingPort),
        Some(s) => s,
    };
    if port_str.len() > MAX_PORT_QUERY_BYTES {
        return Err(ParseError::InvalidPort);
    }
    let port: u16 = port_str.parse().map_err(|_| ParseError::InvalidPort)?;
    if port == 0 {
        return Err(ParseError::InvalidPort);
    }

    if server.len() > MAX_SERVER_LEN {
        return Err(field_too_long("server", MAX_SERVER_LEN));
    }
    reject_control_chars(&server, "server")?;

    let mtproto_secret = match kind {
        ProxyKind::Mtproto => {
            let secret = pairs
                .get("secret")
                .cloned()
                .filter(|s| !s.is_empty())
                .ok_or(ParseError::MissingMtprotoSecret)?;
            if secret.len() > MAX_MTSECRET_LEN {
                return Err(field_too_long("secret", MAX_MTSECRET_LEN));
            }
            reject_control_chars(&secret, "secret")?;
            Some(secret)
        }
        ProxyKind::Socks5 => None,
    };

    let socks_username = match kind {
        ProxyKind::Socks5 => pairs.get("user").cloned().filter(|s| !s.is_empty()),
        ProxyKind::Mtproto => None,
    };

    let socks_password = match kind {
        ProxyKind::Socks5 => pairs.get("pass").cloned().filter(|s| !s.is_empty()),
        ProxyKind::Mtproto => None,
    };

    if let Some(ref u) = socks_username {
        if u.len() > MAX_SOCKS_CRED_LEN {
            return Err(field_too_long("user", MAX_SOCKS_CRED_LEN));
        }
        reject_control_chars(u, "user")?;
    }
    if let Some(ref p) = socks_password {
        if p.len() > MAX_SOCKS_CRED_LEN {
            return Err(field_too_long("pass", MAX_SOCKS_CRED_LEN));
        }
        reject_control_chars(p, "pass")?;
    }

    Ok(ProxyConfig {
        original_input,
        kind,
        server,
        port,
        mtproto_secret,
        socks_username,
        socks_password,
    })
}

fn classify(url: &Url) -> Result<ProxyKind, ParseError> {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");
    let path = url.path().trim_matches('/');

    match scheme {
        // `tg://proxy?...` and `tg://socks?...` use the host for the kind.
        "tg" => {
            if host.eq_ignore_ascii_case("proxy") || path == "proxy" {
                return Ok(ProxyKind::Mtproto);
            }
            if host.eq_ignore_ascii_case("socks") || path == "socks" {
                return Ok(ProxyKind::Socks5);
            }
            Err(ParseError::UnsupportedFormat)
        }
        "http" | "https" => {
            if !host.eq_ignore_ascii_case("t.me") && !host.eq_ignore_ascii_case("telegram.me") {
                return Err(ParseError::UnsupportedFormat);
            }
            match path {
                "proxy" => Ok(ProxyKind::Mtproto),
                "socks" => Ok(ProxyKind::Socks5),
                _ => Err(ParseError::UnsupportedFormat),
            }
        }
        _ => Err(ParseError::UnsupportedFormat),
    }
}

/// First occurrence wins; keys are ASCII-lowercased so `Secret` / `PORT` match Telegram’s usual params.
fn query_pairs_map(url: &Url) -> HashMap<String, String> {
    let mut acc = HashMap::new();
    for (k, v) in url.query_pairs().into_owned() {
        let kl = k.to_ascii_lowercase();
        acc.entry(kl).or_insert(v);
    }
    acc
}

/// Rebuilds the link query with `secret` and `pass` values replaced so verbose logs never echo raw credentials.
/// The full secret remains only in memory inside [`ProxyConfig`] for TDLib requests (not printed).
pub fn redact_sensitive_query_in_link(input: &str) -> String {
    let Ok(mut url) = Url::parse(input.trim()) else {
        return "<could not parse link for display>".to_string();
    };
    // Strip userinfo so `https://user:pass@host/...` cannot leak via `url.to_string()`.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    if url.query().is_none() {
        return url.to_string();
    }
    let mut ser = url.query_pairs().fold(
        url::form_urlencoded::Serializer::new(String::new()),
        |mut ser, (k, v)| {
            let redact = k.eq_ignore_ascii_case("secret")
                || k.eq_ignore_ascii_case("pass")
                || k.eq_ignore_ascii_case("password")
                || k.eq_ignore_ascii_case("token");
            let v_out = if redact { "<redacted>" } else { v.as_ref() };
            ser.append_pair(k.as_ref(), v_out);
            ser
        },
    );
    let q = ser.finish();
    url.set_query(Some(&q));
    url.to_string()
}
