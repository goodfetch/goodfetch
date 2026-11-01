//! The Proving ground (ADRs 0025, 0032 and 0063): the website Goodfetch owns
//! and generates from a seed, with its task list and its oracle. Its command
//! line and HTTP are the interface every caller sees: `serve` binds an
//! address, prints one JSON line, and serves each page family on its own host
//! name, the task list at `/tasks` and the oracle at `/oracle`, beside its
//! network fixtures: forward proxies, a DNS fixture and a watched target.
//! `tasks` prints one seed's task list and exits.

mod access;
mod adversarial;
mod agent;
mod authority;
mod blackhole;
mod browser_pages;
mod catalog;
mod certificates;
mod clean_up;
mod client_hello;
mod clock;
mod command;
mod connection_record;
mod consent;
mod crawl;
mod digest;
mod dns;
mod encodings;
mod exchange;
mod extraction;
mod files;
mod forward_proxy;
mod generator;
mod handmade;
mod hostile;
mod html;
mod languages;
mod login;
mod markdown;
mod model;
mod network;
mod network_fixture;
mod observation;
mod oracle;
mod output;
mod portal;
mod protection;
mod push;
mod raw;
mod raw_site;
mod rendered;
mod replay;
mod robots;
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
use serde_json::Value;

use crate::command::{Command, NetworkProfile, Serve, Shaper, Tasks};
use crate::network::{Loss, Network, Profile};
use crate::network_fixture::NetworkFixtures;
use crate::server::{Server, StopSignal};
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

/// Listens for the stop signals, shapes and calibrates the network, binds
/// the server behind it, serves until stopped, then undoes the shaping. A
/// stop signal that arrives while it starts is acted on once it has
/// reported its address, so the shaping is undone then too.
async fn shape_and_serve(
    seed: u64,
    address: SocketAddr,
    network_profile: NetworkProfile,
    loss: Loss,
    shaper: Shaper,
) -> Result<()> {
    let stop_signal = StopSignal::listen()?;
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
    let outcome = serve(server, &network, seed, stop_signal).await;
    let stopped = network.stop().await;
    outcome.and(stopped)
}

/// Binds the network fixtures beside the server, prints the output line
/// (with every host name the site serves and its certificate authority, so a
/// caller maps each name to the address and trusts the authority with no
/// further request), and serves until `stop_signal` is received.
async fn serve(
    server: Server,
    network: &Network,
    seed: u64,
    stop_signal: StopSignal,
) -> Result<()> {
    let upstream = server.address();
    let reached = network.proxy_address().unwrap_or(upstream);
    let fixtures = NetworkFixtures::bind(reached.ip(), seed, server.observations()).await?;
    let mut line = fixtures.report()?;
    server
        .observations()
        .set_network_fixtures(Value::Object(line.clone()));
    line.insert("address".to_owned(), reached.to_string().into());
    line.insert("version".to_owned(), version::VERSION.into());
    line.insert("seed".to_owned(), seed.into());
    let hosts: Vec<_> = server.site().host_names().map(Value::from).collect();
    line.insert("hosts".to_owned(), hosts.into());
    line.insert(
        "certificate_authority".to_owned(),
        server.site().certificate_authority().into(),
    );
    line.insert(
        "network".to_owned(),
        serde_json::to_value(network.report()).context("reporting the network profile")?,
    );
    output::line(&line)?;
    tokio::select! {
        () = server.run(stop_signal) => {}
        () = network.forward(upstream) => {}
        () = fixtures.run() => {}
    }
    Ok(())
}
