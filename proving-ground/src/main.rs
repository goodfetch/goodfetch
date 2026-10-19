//! The Proving ground (ADRs 0025, 0032 and 0063): the website Goodfetch owns
//! and generates from a seed, with its task list and its oracle. Its command
//! line and HTTP are the interface every caller sees: `serve` binds an
//! address, prints one JSON line, and serves each page family on its own host
//! name, the task list at `/tasks` and the oracle at `/oracle`. `tasks`
//! prints one seed's task list and exits.

mod authority;
mod catalog;
mod client_hello;
mod clock;
mod command;
mod digest;
mod generator;
mod observation;
mod oracle;
mod output;
mod server;
mod site;
mod sync;
mod tap;
mod task;
mod version;

use anyhow::{Context as _, Result};
use clap::Parser as _;

use crate::command::{Command, Serve, Tasks};
use crate::server::Server;
use crate::site::Site;
use crate::task::TaskList;

fn main() -> Result<()> {
    match Command::parse() {
        Command::Serve(serve) => run(serve),
        Command::Tasks(Tasks { seed }) => output::line(&TaskList::new(
            version::VERSION,
            seed,
            &Site::every_task(seed, None),
        )),
    }
}

/// Prints the one output line, then serves until stopped.
fn run(serve: Serve) -> Result<()> {
    #[expect(
        clippy::disallowed_methods,
        reason = "`main` builds the Proving ground's one runtime (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the Tokio runtime")?;
    let Serve { seed, address, .. } = serve;
    let server = runtime.block_on(Server::bind(address, seed))?;
    output::line(&serde_json::json!({
        "address": server.address().to_string(),
        "version": version::VERSION,
        "seed": seed,
    }))?;
    runtime.block_on(server.run())
}
