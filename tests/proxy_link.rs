//! Integration tests for proxy link parsing (no TDLib).

use tg_proxy_check::proxy_link::{parse_proxy_link, ParseError, ProxyKind};

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
