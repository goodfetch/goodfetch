//! The Proving ground (ADRs 0025, 0032 and 0063): the website Goodfetch owns
//! and generates from a seed, with its task list and its oracle. Its command
//! line and HTTP are the interface every caller sees: `serve` binds an
//! address, prints one JSON line, and serves each page family on its own host
//! name, the task list at `/tasks` and the oracle at `/oracle`, beside its
//! network fixtures: forward proxies, a DNS fixture and a watched target.

mod authority;
mod catalog;
mod client_hello;
mod clock;
mod command;
mod connection_record;
mod digest;
mod dns;
mod forward_proxy;
mod generator;
mod network_fixture;
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

use crate::command::{Command, Serve};
use crate::network_fixture::NetworkFixtures;
use crate::server::Server;

fn main() -> Result<()> {
    let Command::Serve(serve) = Command::parse();
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
    let fixtures = runtime.block_on(NetworkFixtures::bind(
        server.address().ip(),
        seed,
        server.observations(),
    ))?;
    let mut line = fixtures.report()?;
    line.insert("address".to_owned(), server.address().to_string().into());
    line.insert("version".to_owned(), version::VERSION.into());
    line.insert("seed".to_owned(), seed.into());
    output::line(&line)?;
    runtime.block_on(async {
        tokio::select! {
            outcome = server.run() => outcome,
            () = fixtures.run() => Ok(()),
        }
    })
}
