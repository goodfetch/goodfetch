//! The TLA+ models' model checking (ADR 0060): `fetch` keeps the pinned pair
//! of TLA+ tools, each checked against its SHA-256, before the outbound-traffic
//! deny; `run` checks models with TLC on one of their configurations. The
//! models' list, with the modules each one traces, is what `xtask affected`
//! reads to select the models a change reaches.

mod models;
mod run;
mod tools;

use std::process::ExitCode;

use anyhow::Result;

use crate::check;
use crate::environment::Environment;
use crate::github;

pub(crate) use models::{MODELS_FOLDER, read as models};
pub(crate) use tools::TOOLS_FILE;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Downloads each tool `test-support/models/tools.toml` pins into
    /// `target/tla-tools/`, unless the kept file already has its pinned
    /// SHA-256, and fails on a download whose SHA-256 differs from its pin.
    /// A tool pinned as a release asset is read through GitHub's API with
    /// `GITHUB_TOKEN`, which a private repository's release needs.
    Fetch {
        /// GitHub's API, or a stand-in's address.
        #[arg(long, default_value = github::API_URL)]
        api_url: String,
    },
    /// Checks models with TLC on the fetched tools; fails on any outcome but
    /// the expected one.
    Run(run::Arguments),
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Fetch { api_url } => Ok(check::report(&tools::fetch(
            &environment.root,
            github::Api {
                url: &api_url,
                token: environment.token(),
            },
        )?)),
        Command::Run(arguments) => Ok(check::report(&run::run(&arguments, &environment.root)?)),
    }
}
