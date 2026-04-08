//! Integration tests for proxy link parsing (no TDLib).

use tg_proxy_check::proxy_link::{
    parse_proxy_link, redact_sensitive_query_in_link, ParseError, ProxyKind,
};

#[test]
fn valid_mtproto_tg_proxy() {
    let c = parse_proxy_link("tg://proxy?server=1.2.3.4&port=443&secret=deadbeef").unwrap();
    assert_eq!(c.kind, ProxyKind::Mtproto);
    assert_eq!(c.server, "1.2.3.4");
    assert_eq!(c.port, 443);
    assert_eq!(c.mtproto_secret.as_deref(), Some("deadbeef"));
    assert_eq!(
        c.original_input,
        "tg://proxy?server=1.2.3.4&port=443&secret=deadbeef"
    );
}

#[test]
fn valid_mtproto_https_t_me() {
    let s = "https://t.me/proxy?server=example.com&port=443&secret=0123456789abcdef";
    let c = parse_proxy_link(s).unwrap();
    assert_eq!(c.kind, ProxyKind::Mtproto);
    assert_eq!(c.server, "example.com");
    assert_eq!(c.port, 443);
    assert_eq!(c.mtproto_secret.as_deref(), Some("0123456789abcdef"));
    assert_eq!(c.original_input, s);
}

#[test]
fn valid_socks5_tg_no_auth() {
    let c = parse_proxy_link("tg://socks?server=10.0.0.1&port=1080").unwrap();
    assert_eq!(c.kind, ProxyKind::Socks5);
    assert_eq!(c.server, "10.0.0.1");
    assert_eq!(c.port, 1080);
    assert!(c.socks_username.is_none());
    assert!(c.socks_password.is_none());
}

#[test]
fn valid_socks5_tg_with_auth() {
    let c = parse_proxy_link("tg://socks?server=1.1.1.1&port=1080&user=foo&pass=bar").unwrap();
    assert_eq!(c.kind, ProxyKind::Socks5);
    assert_eq!(c.server, "1.1.1.1");
    assert_eq!(c.port, 1080);
    assert_eq!(c.socks_username.as_deref(), Some("foo"));
    assert_eq!(c.socks_password.as_deref(), Some("bar"));
}

#[test]
fn missing_port_parameter() {
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&secret=ab");
    assert_eq!(r, Err(ParseError::MissingPort));
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&port=&secret=ab");
    assert_eq!(r, Err(ParseError::MissingPort));
}

#[test]
fn invalid_port() {
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&port=0&secret=ab");
    assert_eq!(r, Err(ParseError::InvalidPort));
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&port=99999&secret=ab");
    assert_eq!(r, Err(ParseError::InvalidPort));
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&port=abc&secret=ab");
    assert_eq!(r, Err(ParseError::InvalidPort));
}

#[test]
fn missing_server() {
    let r = parse_proxy_link("tg://proxy?port=443&secret=ab");
    assert_eq!(r, Err(ParseError::MissingServer));
    let r = parse_proxy_link("tg://proxy?server=&port=443&secret=ab");
    assert_eq!(r, Err(ParseError::MissingServer));
}

#[test]
fn missing_mtproto_secret() {
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&port=443");
    assert_eq!(r, Err(ParseError::MissingMtprotoSecret));
    let r = parse_proxy_link("tg://proxy?server=1.2.3.4&port=443&secret=");
    assert_eq!(r, Err(ParseError::MissingMtprotoSecret));
}

#[test]
fn url_decoding_secret_user_pass() {
    let c = parse_proxy_link("tg://socks?server=h&port=1080&user=u%40x&pass=p%3Ay").unwrap();
    assert_eq!(c.socks_username.as_deref(), Some("u@x"));
    assert_eq!(c.socks_password.as_deref(), Some("p:y"));

    let c = parse_proxy_link("tg://proxy?server=h&port=443&secret=dd%2Bcc").unwrap();
    assert_eq!(c.mtproto_secret.as_deref(), Some("dd+cc"));
}

#[test]
fn http_t_me_socks() {
    let s = "http://t.me/socks?server=z&port=1080";
    let c = parse_proxy_link(s).unwrap();
    assert_eq!(c.kind, ProxyKind::Socks5);
    assert_eq!(c.server, "z");
}

#[test]
fn telegram_me_host_mtproto() {
    let s = "https://telegram.me/proxy?server=a.b.c&port=443&secret=abc";
    let c = parse_proxy_link(s).unwrap();
    assert_eq!(c.kind, ProxyKind::Mtproto);
    assert_eq!(c.server, "a.b.c");
}

#[test]
fn query_keys_case_insensitive() {
    let c = parse_proxy_link("tg://proxy?server=x&port=443&Secret=first&secret=second").unwrap();
    assert_eq!(c.mtproto_secret.as_deref(), Some("first"));
}

#[test]
fn duplicate_query_first_wins() {
    let c = parse_proxy_link("tg://proxy?server=x&port=443&port=1080&secret=ab").unwrap();
    assert_eq!(c.port, 443);
}

#[test]
fn redact_strips_userinfo_and_password_param() {
    let r = redact_sensitive_query_in_link("https://u:p@t.me/proxy?server=x&port=443&secret=z&password=zz");
    assert!(!r.contains("u:p@"));
    assert!(!r.contains("zz"));
    assert!(r.to_ascii_lowercase().contains("redacted"));
}

#[test]
fn redact_sensitive_query_for_verbose_display() {
    let raw = "tg://socks?server=x&port=1080&user=u&pass=secretpass";
    let redacted = redact_sensitive_query_in_link(raw);
    assert!(!redacted.contains("secretpass"));
    assert!(redacted.to_ascii_lowercase().contains("redacted"));

    let raw_mt = "tg://proxy?server=x&port=443&secret=supersecret";
    let red_mt = redact_sensitive_query_in_link(raw_mt);
    assert!(!red_mt.contains("supersecret"));
    assert!(red_mt.to_ascii_lowercase().contains("redacted"));
}
