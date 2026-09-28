mod app;
mod browser_flavor;
mod browser_options;
mod chrome;
mod cli;
mod dom;
mod edge;
mod firefox;
mod inspect;
mod install;
mod live_access;
mod modal;
mod model;
mod overlay;
mod render;
mod runtime;
mod selector;
mod window;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

/// Longest a single command may run before it is abandoned.
///
/// Every call into the accessibility bus is a D-Bus round trip, and those calls
/// have no timeout of their own. When the bus or the registry is half-dead -- a
/// state that is easy to reach, because the registry is started lazily and an
/// earlier session can leave the environment pointing at one that is gone -- a
/// call can simply never return. The command then hangs forever with no output,
/// which is the worst outcome: a caller cannot tell a slow page from a dead bus.
///
/// This bounds that. The limit is well above the longest legitimate wait (a
/// launch or a `page wait` with an explicit timeout), so it only ever fires on a
/// genuine stall, and it says so instead of hanging. Override it with
/// `AXONBROWSER_TIMEOUT_SECS` when a workflow really needs longer.
const DEFAULT_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Commands::InstallDeps) {
        // Installing browsers downloads hundreds of megabytes, so it is the one
        // command that must not be bounded by the stall timeout.
        return app::run(cli).await;
    }

    runtime::bootstrap_headless_session()?;

    let timeout = std::env::var("AXONBROWSER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT);

    match tokio::time::timeout(timeout, app::run(cli)).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "gave up after {}s: the browser did not answer, which usually means the accessibility bus is not responding (restart the browser with `launch` to re-register it). Set AXONBROWSER_TIMEOUT_SECS to allow longer",
            timeout.as_secs()
        )),
    }
}
