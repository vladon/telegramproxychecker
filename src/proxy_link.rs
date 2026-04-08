//! Parse Telegram MTProto and SOCKS5 proxy links into a strongly typed config.
//!
//! Supported:
//! - MTProto: `tg://proxy?...`, `https://t.me/proxy?...`, `http://t.me/proxy?...`
//! - SOCKS5: `tg://socks?...`, `https://t.me/socks?...`, `http://t.me/socks?...`

use std::collections::HashMap;
use thiserror::Error;
use url::Url;

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

    #[error("missing or invalid port (must be 1–65535)")]
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

    let port_str = pairs.get("port").map(String::as_str).unwrap_or("");
    let port: u16 = port_str.parse().map_err(|_| ParseError::InvalidPort)?;
    if port == 0 {
        return Err(ParseError::InvalidPort);
    }

    let mtproto_secret = match kind {
        ProxyKind::Mtproto => {
            let secret = pairs
                .get("secret")
                .cloned()
                .filter(|s| !s.is_empty())
                .ok_or(ParseError::MissingMtprotoSecret)?;
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

fn query_pairs_map(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .into_owned()
        .fold(HashMap::new(), |mut acc, (k, v)| {
            acc.insert(k, v);
            acc
        })
}
