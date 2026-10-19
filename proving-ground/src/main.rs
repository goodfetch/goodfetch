//! The Proving ground (ADRs 0025, 0032 and 0063): the website Goodfetch owns
//! and generates from a seed, with its task list and its oracle. Its command
//! line and HTTP are the interface every caller sees: `serve` binds an
//! address, prints one JSON line, and serves each page family on its own host
//! name, the task list at `/tasks` and the oracle at `/oracle`. `tasks`
//! prints one seed's task list and exits.

mod agent;
mod authority;
mod catalog;
mod client_hello;
mod clock;
mod command;
mod digest;
mod encodings;
mod exchange;
mod extraction;
mod files;
mod generator;
mod handmade;
mod languages;
mod login;
mod model;
mod network;
mod observation;
mod oracle;
mod output;
mod portal;
mod rendered;
mod replay;
mod schedule;
mod server;
mod site;
mod sync;
mod tap;
mod task;
mod totp;
mod version;

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::{Context as _, Result, bail};
use clap::Parser as _;

use crate::command::{Command, NetworkProfile, Serve, Shaper, Tasks};
use crate::network::{Loss, Network, Profile};
use crate::server::Server;
use crate::site::Site;
use crate::task::TaskList;

fn main() -> Result<()> {
    match Command::parse() {
        Command::Serve(serve) => run(&serve),
        Command::Tasks(Tasks { seed }) => output::line(&TaskList::new(
            version::VERSION,
            seed,
            &Site::every_task(seed, None),
        )),
    }
}

/// Builds the runtime `serve` runs on, then serves until stopped.
fn run(serve: &Serve) -> Result<()> {
    #[expect(
        clippy::disallowed_methods,
        reason = "`serve` builds the Proving ground's one runtime here (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the Tokio runtime")?;
    let &Serve {
        seed,
        address,
        network_profile,
        loss,
        shaper,
        ..
    } = serve;
    runtime.block_on(shape_and_serve(
        seed,
        address,
        network_profile,
        loss,
        shaper,
    ))
}

/// Shapes and calibrates the network, binds the server behind it, prints
/// the output line, and serves until stopped; then undoes the shaping.
async fn shape_and_serve(
    seed: u64,
    address: SocketAddr,
    network_profile: NetworkProfile,
    loss: Loss,
    shaper: Shaper,
) -> Result<()> {
    let profile = match network_profile.round_trip() {
        Some((name, round_trip)) => Some(Profile::new(name, round_trip, loss)),
        None if loss.is_some() => bail!("a loss needs a shaped network profile, such as `rtt-0`"),
        None => None,
    };
    let (network, server) = match (profile, shaper) {
        (None, _) => (
            Network::unshaped(),
            Server::bind(address, None, seed).await?,
        ),
        (Some(profile), Shaper::Proxy) => {
            let network = Network::proxy(profile, seed, address).await?;
            let reached_on = network.proxy_address().map(|proxy| proxy.port());
            let behind = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
            (network, Server::bind(behind, reached_on, seed).await?)
        }
        (Some(profile), Shaper::Netem) => {
            let server = Server::bind(address, None, seed).await?;
            (
                Network::netem(profile, server.address().port()).await?,
                server,
            )
        }
    };
    let upstream = server.address();
    let printed = output::line(&serde_json::json!({
        "address": network.proxy_address().unwrap_or(upstream).to_string(),
        "version": version::VERSION,
        "seed": seed,
        "network": network.report(),
    }));
    let outcome = match printed {
        Ok(()) => tokio::select! {
            outcome = server.run() => outcome,
            () = network.forward(upstream) => Ok(()),
        },
        Err(error) => Err(error),
    };
    let stopped = network.stop().await;
    outcome.and(stopped)
}
