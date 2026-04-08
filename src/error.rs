//! Application errors and exit code mapping.

use crate::proxy_link::ParseError;
use thiserror::Error;

/// CLI configuration and usage errors (exit code 2).
#[derive(Debug, Error)]
pub enum CliError {
    #[error("provide either a positional proxy link or --proxy-link, not both")]
    AmbiguousProxyLink,

    #[error("proxy link is required (positional argument or --proxy-link)")]
    MissingProxyLink,

    #[error("timeout must be greater than zero")]
    InvalidTimeout,

    #[error(
        "TG_API_ID environment variable must be set when --api-id is omitted, or pass --api-id"
    )]
    MissingApiId,

    #[error("TG_API_HASH environment variable must be set when --api-hash is omitted, or pass --api-hash")]
    MissingApiHash,

    #[error("invalid API id: {0}")]
    InvalidApiId(String),

    #[error("invalid API hash: {0}")]
    InvalidApiHash(String),

    #[error("--auth-session must be an existing directory ({0})")]
    InvalidAuthSession(String),
}

/// Snapshot when the overall probe deadline is reached (accurate verbose output / debugging).
#[derive(Debug, Clone)]
pub struct ProbeTimeoutContext {
    pub probe_start_wall_ms: u128,
    pub probe_end_wall_ms: u128,
    pub wall_duration: std::time::Duration,
    pub auth_states_seen: Vec<String>,
    pub tdlib_log_lines: Vec<String>,
}

/// TDLib / probe failures after a proxy link was parsed successfully.
#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("probe timed out")]
    Timeout(ProbeTimeoutContext),

    #[error("TDLib initialization failed: {0}")]
    TdlibInit(String),

    #[error("internal error: {0}")]
    Internal(String),
}

/// Top-level run error for mapping to exit codes.
#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Cli(#[from] CliError),

    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error(transparent)]
    Probe(#[from] ProbeError),
}

/// Exit codes per specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success = 0,
    Unreachable = 1,
    InvalidInput = 2,
    Timeout = 3,
    TdlibInit = 4,
    Internal = 5,
}

impl ExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

impl From<RunError> for ExitCode {
    fn from(e: RunError) -> Self {
        match e {
            RunError::Cli(_) | RunError::Parse(_) => ExitCode::InvalidInput,
            RunError::Probe(p) => match p {
                ProbeError::Timeout(_) => ExitCode::Timeout,
                ProbeError::TdlibInit(_) => ExitCode::TdlibInit,
                ProbeError::Internal(_) => ExitCode::Internal,
            },
        }
    }
}
