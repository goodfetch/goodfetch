//! `browser-host`: runs a Browser host beside its browser (ADRs 0022, 0095,
//! 0096, 0129 and 0132; issue 753).
//!
//! Marked `collie` and carried by both Products' executables. Under `--listen`,
//! binds the requested address (refusing any non-loopback address without TLS),
//! prints [`BrowserHostReady`] as JSON on stdout once listening, serves one
//! Session in an ephemeral profile folder, wipes that folder once the Session
//! ends, and exits 0. Under `--connect`, connects out to an engine URL
//! (refusing non-loopback without TLS), authenticates with `--token-file`, and
//! serves Sessions until the link ends or a signal arrives.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::page::browser_host::{BrowserHostAddress, BrowserHostReady};
use tokio_util::sync::CancellationToken;

use super::daemon::Machine;
use super::output::{self, ColourFlag};
use crate::runtime;

/// Runs a Browser host beside its browser to serve a Session for an engine.
#[derive(Args)]
pub(super) struct Arguments {
    /// Address to listen on (`host:port`, `ws://...`, or `wss://...`).
    /// Serves one Session, wipes its profile, and exits.
    #[arg(
        long,
        value_name = "ADDRESS",
        conflicts_with = "connect",
        required_unless_present = "connect"
    )]
    pub(super) listen: Option<String>,
    /// Engine URL to connect out to (`ws://...` or `wss://...`).
    #[arg(
        long,
        value_name = "ENGINE_URL",
        conflicts_with = "listen",
        required_unless_present = "listen"
    )]
    pub(super) connect: Option<String>,
    /// Path to the file containing the shared authentication token.
    #[arg(long, value_name = "PATH")]
    pub(super) token_file: PathBuf,
    #[command(flatten)]
    pub(super) _colour: ColourFlag,
}

/// Runs `browser-host` in `--listen` or `--connect` mode and wipes its
/// ephemeral profile folder before returning.
pub(super) fn run(
    arguments: &Arguments,
    engine: &Engine,
    mut dependencies: Dependencies,
    machine: &Machine<'_>,
) -> ExitCode {
    let token = match read_token_file(&arguments.token_file) {
        Ok(token) => token,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let mode = match validate_mode(arguments.listen.as_deref(), arguments.connect.as_deref()) {
        Ok(mode) => mode,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let (ephemeral, ephemeral_home) = match create_ephemeral_home(machine) {
        Ok(pair) => pair,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };
    dependencies.home = ephemeral_home;
    dependencies.run_state_location = RunStateLocation::PerMajor;

    let threads = runtime::Threads::of(machine.settings.as_ref().ok());
    let runtime = runtime::multi_thread(threads);
    let exit = runtime.block_on(async move {
        let shutdown = CancellationToken::new();
        let signal_cancel = shutdown.clone();
        match mode {
            Mode::Listen(socket_addr) => {
                let (listener, endpoint) = match Network::Os.listen_at(socket_addr).await {
                    Ok(bound) => bound,
                    Err(error) => {
                        tracing::error!(address = %socket_addr, %error, "the Browser host cannot listen on the requested address");
                        return ExitCode::FAILURE;
                    }
                };
                let bound_address = match &endpoint {
                    DaemonEndpoint::Tcp(addr) => addr.to_string(),
                    DaemonEndpoint::Unix(path) => path.display().to_string(),
                };
                let ready = BrowserHostReady {
                    address: bound_address,
                    pid: std::process::id(),
                    product: Product::Collie,
                    version: engine.identity().product_version.clone(),
                };
                output::json(&ready);
                tokio::select! {
                    result = engine.run_browser_host_listen(listener, token, dependencies, shutdown) => {
                        match result {
                            Ok(()) => ExitCode::SUCCESS,
                            Err(error) => {
                                tracing::error!(%error, "the Browser host listener failed");
                                ExitCode::FAILURE
                            }
                        }
                    }
                    () = wait_for_shutdown_signal() => {
                        signal_cancel.cancel();
                        ExitCode::SUCCESS
                    }
                }
            }
            Mode::Connect(socket_addr) => {
                let connection = match transport::connect_daemon(&DaemonEndpoint::Tcp(socket_addr)).await {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::error!(address = %socket_addr, %error, "the Browser host could not connect to the engine");
                        return ExitCode::FAILURE;
                    }
                };
                tokio::select! {
                    result = engine.run_browser_host_connect(connection, token, dependencies, shutdown) => {
                        match result {
                            Ok(()) => ExitCode::SUCCESS,
                            Err(message) => {
                                tracing::error!("{message}");
                                ExitCode::FAILURE
                            }
                        }
                    }
                    () = wait_for_shutdown_signal() => {
                        signal_cancel.cancel();
                        ExitCode::SUCCESS
                    }
                }
            }
        }
    });
    runtime.shutdown_background();
    // Explicitly wipe the ephemeral profile directory before exiting (ADR 0022).
    if let Err(error) = ephemeral.close() {
        tracing::warn!(%error, "could not remove the Browser host ephemeral profile folder");
    }
    exit
}

enum Mode {
    Listen(std::net::SocketAddr),
    Connect(std::net::SocketAddr),
}

fn validate_mode(listen: Option<&str>, connect: Option<&str>) -> Result<Mode, String> {
    if let Some(raw) = listen {
        let parsed = BrowserHostAddress::parse(raw)?;
        if parsed.tls {
            return Err(
                "TLS serving on `browser-host --listen` (`wss://`) is not built yet; use loopback until area 4 brings TLS"
                    .to_owned(),
            );
        }
        let addr = parsed.loopback_socket_addr().ok_or_else(|| {
            format!("refusing non-loopback `--listen` address `{raw}` without TLS")
        })?;
        return Ok(Mode::Listen(addr));
    }
    if let Some(raw) = connect {
        let parsed = BrowserHostAddress::parse(raw)?;
        if parsed.tls {
            return Err(
                "TLS client connections on `browser-host --connect` (`wss://`) are not built yet; use loopback until area 4 brings TLS"
                    .to_owned(),
            );
        }
        let addr = parsed
            .loopback_socket_addr()
            .ok_or_else(|| format!("refusing non-loopback `--connect` URL `{raw}` without TLS"))?;
        return Ok(Mode::Connect(addr));
    }
    Err("pass either `--listen <ADDRESS>` or `--connect <ENGINE_URL>`".to_owned())
}

fn read_token_file(path: &Path) -> Result<String, String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "reading the Browser host token file once at startup before the runtime starts (ADR 0059)"
    )]
    let raw = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "could not read `--token-file` `{}`: {error}",
            path.display()
        )
    })?;
    let trimmed = raw.trim().to_owned();
    if trimmed.is_empty() {
        return Err(format!(
            "`--token-file` `{}` is empty; provide a non-empty shared token",
            path.display()
        ));
    }
    Ok(trimmed)
}

fn create_ephemeral_home(machine: &Machine<'_>) -> Result<(tempfile::TempDir, Home), String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "creating the Browser host ephemeral profile folder once at startup before the runtime starts (ADR 0059)"
    )]
    std::fs::create_dir_all(machine.home.root())
        .map_err(|error| format!("could not prepare Goodfetch home: {error}"))?;
    let ephemeral = tempfile::Builder::new()
        .prefix("browser-host-")
        .tempdir_in(machine.home.root())
        .map_err(|error| format!("could not create ephemeral Browser host folder: {error}"))?;

    let browsers_path = machine.cache_location().root().to_owned();
    let ephemeral_home =
        Home::new(ephemeral.path().to_path_buf()).with_browsers_path(Some(browsers_path));

    // Copy per-major machine configuration files (if present) so settings like
    // `egress.private_ranges` apply inside the ephemeral home without touching
    // the real Run index.
    let src_major = machine.home.per_major(&machine.identity.product_version);
    let dst_major = ephemeral_home.per_major(&machine.identity.product_version);
    copy_if_exists(
        &src_major.join("configuration.toml"),
        &dst_major.join("configuration.toml"),
    );
    copy_if_exists(
        &src_major.join("policy.toml"),
        &dst_major.join("policy.toml"),
    );

    Ok((ephemeral, ephemeral_home))
}

fn copy_if_exists(src: &Path, dst: &Path) {
    #[expect(
        clippy::disallowed_methods,
        reason = "copying optional machine configuration into the Browser host ephemeral folder once at startup"
    )]
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::copy(src, dst);
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let term = signal(SignalKind::terminate());
        let int = signal(SignalKind::interrupt());
        match (term, int) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                }
            }
            _ => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
