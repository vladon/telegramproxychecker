//! Command-line interface (clap).

use crate::error::CliError;
use clap::Parser;
use std::env;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "tg-proxy-check",
    version,
    about = "Check Telegram reachability through a proxy using TDLib pingProxy"
)]
pub struct Cli {
    /// Proxy link (positional). Mutually exclusive with --proxy-link.
    #[arg(value_name = "PROXY_LINK")]
    pub positional_proxy: Option<String>,

    /// Full Telegram proxy link (alternative to positional).
    #[arg(long = "proxy-link", value_name = "URL")]
    pub proxy_link_flag: Option<String>,

    #[arg(long)]
    pub verbose: bool,

    #[arg(long)]
    pub json: bool,

    /// Overall probe timeout in seconds.
    #[arg(long, value_name = "SECONDS", default_value_t = 60)]
    pub timeout: u64,

    #[arg(long, value_name = "ID")]
    pub api_id: Option<i32>,

    #[arg(long, value_name = "HASH")]
    pub api_hash: Option<String>,
}

/// Fully resolved CLI options after env fallback and validation.
#[derive(Debug, Clone)]
pub struct ResolvedCli {
    pub proxy_link: String,
    pub verbose: bool,
    pub json: bool,
    pub timeout: Duration,
    pub api_id: i32,
    pub api_hash: String,
}

impl ResolvedCli {
    pub fn from_env() -> Result<Self, CliError> {
        let cli = Cli::parse();

        let proxy_link = match (&cli.positional_proxy, &cli.proxy_link_flag) {
            (Some(_), Some(_)) => return Err(CliError::AmbiguousProxyLink),
            (Some(s), None) => s.clone(),
            (None, Some(s)) => s.clone(),
            (None, None) => return Err(CliError::MissingProxyLink),
        };

        if cli.timeout == 0 {
            return Err(CliError::InvalidTimeout);
        }

        let api_id = match cli.api_id {
            Some(id) => id,
            None => parse_api_id_env()?,
        };

        let api_hash = match &cli.api_hash {
            Some(h) if !h.is_empty() => h.clone(),
            Some(_) => return Err(CliError::InvalidApiHash("empty string".into())),
            None => read_api_hash_env()?,
        };

        Ok(ResolvedCli {
            proxy_link,
            verbose: cli.verbose,
            json: cli.json,
            timeout: Duration::from_secs(cli.timeout),
            api_id,
            api_hash,
        })
    }
}

fn parse_api_id_env() -> Result<i32, CliError> {
    let raw = env::var("TG_API_ID").map_err(|_| CliError::MissingApiId)?;
    raw.trim()
        .parse::<i32>()
        .map_err(|_| CliError::InvalidApiId(raw))
}

fn read_api_hash_env() -> Result<String, CliError> {
    let raw = env::var("TG_API_HASH").map_err(|_| CliError::MissingApiHash)?;
    let t = raw.trim();
    if t.is_empty() {
        return Err(CliError::InvalidApiHash("empty string".into()));
    }
    Ok(t.to_string())
}
