//! `tg-proxy-check` — verify Telegram reachability through a proxy via TDLib `pingProxy`.

fn main() {
    let code = tg_proxy_check::run();
    std::process::exit(code.as_i32());
}
