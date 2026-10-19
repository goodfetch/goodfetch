//! The Proving ground (ADRs 0025, 0032 and 0063): the website Goodfetch owns
//! and generates from a seed, with its task list and its oracle. Its command
//! line and HTTP are the interface every caller sees: `serve` binds an
//! address, prints one JSON line, and serves each page family on its own host
//! name, the task list at `/tasks` and the oracle at `/oracle`.

mod authority;
mod catalog;
mod client_hello;
mod clock;
mod command;
mod digest;
mod generator;
mod network;
mod observation;
mod oracle;
mod output;
mod server;
mod site;
mod sync;
mod tap;
mod task;
mod version;

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::{Context as _, Result, bail};
use clap::Parser as _;

use crate::command::{Command, NetworkProfile, Serve, Shaper};
use crate::network::{Loss, Network, Profile};
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
    let Serve {
        seed,
        address,
        network_profile,
        loss,
        shaper,
        ..
    } = serve;
    runtime.block_on(run(seed, address, network_profile, loss, shaper))
}

/// Shapes and calibrates the network, binds the server behind it, prints
/// the output line, and serves until stopped; then undoes the shaping.
async fn run(
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
