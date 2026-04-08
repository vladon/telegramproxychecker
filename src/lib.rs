//! Library surface for `tg-proxy-check` (CLI binary and integration tests).

pub mod cli;
pub mod error;
pub mod output;
pub mod proxy_link;
pub mod tdlib_client;

pub use error::{CliError, ExitCode, ProbeError, ProbeTimeoutContext, RunError};

use crate::cli::ResolvedCli;
use crate::output::{ProbeReport, RenderOpts};
use crate::proxy_link::parse_proxy_link;
use crate::tdlib_client::{probe_proxy, TdlibCredentials, TdlibProbeSettings};

/// Run the full flow: CLI → parse link → TDLib probe → printed output.
pub fn run() -> ExitCode {
    let resolved = match ResolvedCli::from_env() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::InvalidInput;
        }
    };

    let proxy = match parse_proxy_link(&resolved.proxy_link) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::InvalidInput;
        }
    };

    let creds = TdlibCredentials {
        api_id: resolved.api_id,
        api_hash: resolved.api_hash,
    };

    let td_settings = TdlibProbeSettings {
        timeout: resolved.timeout,
        verbose: resolved.verbose,
    };

    let report = match probe_proxy(&proxy, &creds, &td_settings) {
        Ok(r) => r,
        Err(e) => {
            let partial = ProbeReport::from_probe_failure(&e);
            let exit: ExitCode = RunError::Probe(e).into();
            let opts = RenderOpts {
                verbose: resolved.verbose,
                json: resolved.json,
                probe_timeout_sec: resolved.timeout.as_secs(),
            };
            let _ = output::render(&proxy, &partial, &opts);
            return exit;
        }
    };

    let opts = RenderOpts {
        verbose: resolved.verbose,
        json: resolved.json,
        probe_timeout_sec: resolved.timeout.as_secs(),
    };

    if let Err(e) = output::render(&proxy, &report, &opts) {
        eprintln!("{}", e);
        return ExitCode::Internal;
    }

    if report.ok {
        ExitCode::Success
    } else {
        ExitCode::Unreachable
    }
}
