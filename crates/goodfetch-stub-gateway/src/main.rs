//! The Stub gateway, `goodfetch-stub-gateway` (ADRs 0068, 0096 and 0125):
//! a test-only stand-in of Goodfetch Cloud's gateway, never shipped. It
//! serves no file: a Client opens a WebSocket at `/protocol` and signs in
//! with the configured token, presented in its `hello`. Behind it, each run gets an Attached engine of the executable of the
//! Product the routing rule picks (`goodfetch_protocol::routing`), both
//! executables taken by path from the same commit, so it names no Foxhound
//! crate. It answers `run.list` from its own index and forwards every other
//! call to the engine that owns the run, to which its `hello` says it is the
//! gateway and announces that engine's own Product (ADR 0095).
//!
//! Its listener binds loopback port 0 through `goodfetch-engine`'s
//! `transport` (ADRs 0059 and 0060). Once it serves, it writes its
//! address to the `--ready` file as `{"address": …}`; it
//! writes nothing to stdout, and logs through `tracing` to stderr. It runs
//! until it is interrupted (`SIGINT`), when it kills its engines and exits
//! once each has been reaped, or until it is killed, when its engines die
//! with it.

mod engines;
mod index;
mod relay;
mod site;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport;

use crate::engines::{Engines, Executables};
use crate::relay::Gateway;
use crate::site::Site;

/// The gateway's command line.
#[derive(Parser)]
#[command(name = "goodfetch-stub-gateway", version)]
struct Arguments {
    /// Collie's executable, which runs each run that gets Collie.
    #[arg(long)]
    collie: PathBuf,
    /// Foxhound's executable, which runs each run that gets Foxhound;
    /// without it, the gateway refuses a Foxhound Client at `hello` with
    /// `not_in_product`.
    #[arg(long)]
    foxhound: Option<PathBuf>,
    /// The file holding the token a Client's `hello` presents.
    #[arg(long)]
    token_file: PathBuf,
    /// The folder of the gateway's state: each engine's Goodfetch home.
    #[arg(long)]
    state: PathBuf,
    /// Where to write the gateway's address once it serves.
    #[arg(long)]
    ready: PathBuf,
}

#[expect(
    clippy::disallowed_methods,
    reason = "main builds the runtime, reads the environment and gives the tracing writer stderr (ADRs 0059 and 0060)"
)]
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let arguments = Arguments::parse();
    let environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "the gateway cannot build its runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(arguments, environment)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the gateway stopped");
            ExitCode::FAILURE
        }
    }
}

/// Serves until the process is killed or interrupted; interrupted, it
/// kills and reaps its engines before it returns.
async fn serve(arguments: Arguments, environment: Vec<(OsString, OsString)>) -> Result<(), String> {
    let token = tokio::fs::read_to_string(&arguments.token_file)
        .await
        .map_err(|error| format!("reading the token file: {error}"))?
        .trim()
        .to_owned();
    let mut entropy =
        Entropy::from_os().map_err(|error| format!("the OS gave no seed: {error}"))?;
    let loopback = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = transport::listen_gateway(loopback)
        .await
        .map_err(|error| format!("listening for Clients: {error}"))?;
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let engines = Engines::new(
        Executables {
            collie: arguments.collie,
            foxhound: arguments.foxhound,
        },
        environment,
        arguments.state,
        Arc::clone(&owner),
    )
    .map_err(|error| format!("preparing the engines: {error}"))?;
    let gateway = Arc::new(Gateway::new(engines, token, &mut entropy));
    let site = Site::new(Arc::clone(&gateway), &listener)
        .map_err(|error| format!("reading the listener's address: {error}"))?;
    let ready = serde_json::json!({ "address": site.address() });
    tokio::fs::write(&arguments.ready, ready.to_string())
        .await
        .map_err(|error| format!("writing the ready file: {error}"))?;
    owner.spawn(Arc::new(site).serve(listener, Arc::clone(&owner)));
    let _ = tokio::signal::ctrl_c().await;
    gateway.stop().await;
    owner.cancel();
    Ok(())
}
