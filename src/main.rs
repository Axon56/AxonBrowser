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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if !matches!(cli.command, Commands::InstallDeps) {
        runtime::bootstrap_headless_session()?;
    }

    app::run(cli).await
}
