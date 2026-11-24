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
mod browser_builds;
mod browser_pages;
mod catalog;
mod certificates;
mod challenges;
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
#[expect(
    unreachable_pub,
    dead_code,
    reason = "the same source is test support's public interface (`proving_ground_format`) and the conformance harness's reader of the observations' requests, which the Proving ground writes through `observation`"
)]
mod interface;
mod languages;
mod linger;
mod login;
mod markdown;
mod model;
mod network;
mod network_fixture;
mod observation;
mod online;
mod oracle;
mod output;
mod page_load;
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
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use clap::Parser as _;

use crate::browser_builds::BrowserBuilds;
use crate::command::{Command, Copy, NetworkProfile, Serve, Shaper, Tasks};
use crate::interface::ServeLine;
use crate::network::{Loss, Network, Profile};
use crate::network_fixture::NetworkFixtures;
use crate::online::Online;
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
    let runtime = goodfetch_runtime::multi_thread()
        .enable_all()
        .build()
        .context("building the Tokio runtime")?;
    let Serve {
        seed,
        copy,
        address,
        network_profile,
        loss,
        shaper,
        ref browser_builds,
        ref online_config,
    } = *serve;
    let builds = match browser_builds {
        Some(folder) => BrowserBuilds::read(folder)?,
        None => BrowserBuilds::default(),
    };
    let online = match (copy, online_config) {
        (Copy::Online, Some(path)) => Some(online(path)?),
        (Copy::Online, None) => bail!("the online copy needs --online-config"),
        (Copy::Local, Some(_)) => bail!("the local copy takes no --online-config"),
        (Copy::Local, None) => None,
    };
    runtime.block_on(shape_and_serve(
        seed,
        address,
        (network_profile, loss, shaper),
        builds,
        online,
    ))
}

/// The online copy of the configuration at `path`, each vendor's secret
/// read from the variable its section names.
fn online(path: &Path) -> Result<Online> {
    #[expect(
        clippy::disallowed_methods,
        reason = "`serve` reads its online configuration once, on its main thread, before its runtime starts"
    )]
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the online configuration {}", path.display()))?;
    let config = online::Config::parse(&text)?;
    #[expect(
        clippy::disallowed_methods,
        reason = "main is the one reader of the environment (ADR 0059): the vendors' secrets"
    )]
    let secret = |name: &str| std::env::var(name).ok();
    Online::new(&config, secret)
}

/// Listens for the stop signals, shapes and calibrates the network, binds
/// the server behind it, serves until stopped, then undoes the shaping. A
/// stop signal that arrives while it starts is acted on once it has
/// reported its address, so the shaping is undone then too.
async fn shape_and_serve(
    seed: u64,
    address: SocketAddr,
    (network_profile, loss, shaper): (NetworkProfile, Loss, Shaper),
    builds: BrowserBuilds,
    online: Option<Online>,
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
            Server::bind(address, None, seed, builds, online).await?,
        ),
        (Some(profile), Shaper::Proxy) => {
            let network = Network::proxy(profile, seed, address).await?;
            let reached_on = network.proxy_address().map(|proxy| proxy.port());
            let behind = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
            (
                network,
                Server::bind(behind, reached_on, seed, builds, online).await?,
            )
        }
        (Some(profile), Shaper::Netem) => {
            let server = Server::bind(address, None, seed, builds, online).await?;
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
    let addresses = fixtures.report()?;
    server.observations().set_network_fixtures(
        serde_json::to_value(&addresses).context("reporting the network fixtures")?,
    );
    let line = ServeLine {
        address: reached,
        version: version::VERSION.to_owned(),
        seed,
        hosts: server.site().host_names().map(str::to_owned).collect(),
        certificate_authority: server.site().certificate_authority().to_owned(),
        fixtures: addresses,
        network: serde_json::to_value(network.report()).context("reporting the network profile")?,
    };
    output::line(&line)?;
    tokio::select! {
        () = server.run(stop_signal) => {}
        () = network.forward(upstream) => {}
        () = fixtures.run() => {}
    }
    Ok(())
}
