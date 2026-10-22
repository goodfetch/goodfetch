//! `daemon run`, the foreground Daemon (ADR 0049): the Product its
//! operator chose, with no pick (ADR 0098), under Docker, Kubernetes or
//! systemd. It writes no runtime file and never stops idle; it listens on
//! `daemon.address`, a loopback address until the Daemon terminates TLS
//! (area 4), and Clients open with the persistent token of the major's
//! `daemon.token`; it reads `daemon.address` and `daemon.drain_grace` from
//! user configuration and the Operator policy, which
//! `GOODFETCH_DAEMON_ADDRESS` and `GOODFETCH_DAEMON_DRAIN_GRACE` override
//! unless the policy locks them; and its logs are JSON lines on stdout,
//! which `main` set up. It listens for `SIGTERM` and `SIGINT` (Ctrl-C on
//! Windows) before it listens for Clients, and on either it drains: it
//! takes no run, checkpoints each live run as resumable and exits within
//! `daemon.drain_grace`. On Linux it runs under its own reaper (the
//! `reaper` module), which reaps every browser descendant.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::daemon::{DaemonConsole, DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::settings::{Resolved, SettingsError};
use goodfetch_engine::transport::Network;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::daemon::{ADDRESS, DRAIN_GRACE, IDLE_TIMEOUT};
use goodfetch_protocol::settings::{SETTING_INVALID, SETTING_LOCKED, SettingsLayer};
use tokio_util::sync::CancellationToken;

use super::{Machine, multi_thread_runtime};
use crate::executable::browsers;

/// The deployment variable that overrides `daemon.address` for `daemon run`.
const ADDRESS_VARIABLE: &str = "GOODFETCH_DAEMON_ADDRESS";

/// The deployment variable that overrides `daemon.drain_grace` for
/// `daemon run`, in milliseconds.
const DRAIN_GRACE_VARIABLE: &str = "GOODFETCH_DAEMON_DRAIN_GRACE";

/// What the foreground Daemon runs with, from the machine's settings and
/// its environment.
struct Configured {
    address: SocketAddr,
    drain_grace: Duration,
    idle_timeout: Duration,
    /// The machine's settings, which choose the run-state layout.
    settings: Resolved,
}

/// Why the foreground Daemon did not start: an Error code and its message.
struct Refused {
    code: &'static str,
    message: String,
}

impl From<SettingsError> for Refused {
    fn from(error: SettingsError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

/// Runs the foreground Daemon until it drains or `daemon.stop` stops it.
pub(in crate::executable) fn run(
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
) -> ExitCode {
    let identity = engine.identity();
    let configured = match configured(machine) {
        Ok(configured) => configured,
        Err(refused) => {
            tracing::error!(code = refused.code, "{}", refused.message);
            return ExitCode::FAILURE;
        }
    };
    let install =
        match super::launched::install_database(machine, &dependencies, Some(&configured.settings))
        {
            Ok(install) => install,
            Err(failure) => return failure.report(),
        };
    let files = machine.files();
    let token = match files.persistent_token() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "the Daemon's persistent token cannot be read or made");
            return ExitCode::FAILURE;
        }
    };
    let pid = std::process::id();
    let runtime = multi_thread_runtime();
    let ended = runtime.block_on(async {
        // Listening for the signals comes first, so one sent as soon as the
        // Daemon reports that it serves is acted on.
        let signalled = match Signals::listen() {
            Ok(signals) => signals,
            Err(error) => {
                tracing::error!(%error, "the Daemon cannot listen for SIGTERM");
                return Err(ExitCode::FAILURE);
            }
        };
        let (listener, endpoint) = match Network::Os.listen_at(configured.address).await {
            Ok(listening) => listening,
            Err(error) => {
                tracing::error!(address = %configured.address, %error, "the Daemon cannot listen at `daemon.address`");
                return Err(ExitCode::FAILURE);
            }
        };
        let asked = CancellationToken::new();
        let options = DaemonOptions {
            token,
            endpoint: endpoint.clone(),
            pid,
            idle_timeout: configured.idle_timeout,
            keep_alive: true,
            holds: Holds::new(),
            succession: Arc::new(TheOperatorsChoice),
            console: DaemonConsole::Absent,
            drain: Some(Drain {
                asked: asked.clone(),
                grace: configured.drain_grace,
            }),
            install,
            ops: None,
            browsers: None,
        };
        tracing::info!(
            %endpoint,
            pid,
            product = identity.product.as_str(),
            version = %identity.product_version,
            token_file = %files.token_file().display(),
            drain_grace = u64::try_from(configured.drain_grace.as_millis()).unwrap_or(u64::MAX),
            "the foreground Daemon serves"
        );
        let grace = configured.drain_grace;
        let deadline = async move {
            let signal = signalled.received().await;
            tracing::info!(signal, "the foreground Daemon was asked to drain");
            asked.cancel();
            // The engine's drain ends within the grace; this bounds what
            // follows it too.
            tokio::time::sleep(grace).await;
        };
        tokio::select! {
            ended = engine.run_daemon(listener, dependencies, options) => Ok(Some(ended)),
            () = deadline => Ok(None),
        }
    });
    runtime.shutdown_background();
    match ended {
        Ok(Some(DaemonEnd::ListenerFailed)) => ExitCode::FAILURE,
        Ok(Some(_)) => ExitCode::SUCCESS,
        Ok(None) => {
            tracing::warn!("the drain grace passed before the Daemon stopped: it exits now");
            ExitCode::FAILURE
        }
        Err(exit) => exit,
    }
}

/// `daemon.address`, `daemon.drain_grace` and `daemon.idle_timeout` from
/// the machine's settings, the two deployment variables overriding the
/// first two unless the Operator policy locks them; the address must be a
/// loopback one.
fn configured(machine: &Machine<'_>) -> Result<Configured, Refused> {
    let settings = browsers::machine_settings(
        machine.home,
        machine.identity.product,
        &machine.identity.product_version,
    )?;
    let address = overridden(
        &settings,
        machine.environment,
        ADDRESS_VARIABLE,
        ADDRESS.name,
    )?
    .unwrap_or_else(|| {
        settings
            .value(ADDRESS.name)
            .as_str()
            .unwrap_or_default()
            .to_owned()
    });
    let address = parse_address(&address)?;
    let drain_grace = match overridden(
        &settings,
        machine.environment,
        DRAIN_GRACE_VARIABLE,
        DRAIN_GRACE.name,
    )? {
        Some(milliseconds) => milliseconds
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|_| Refused {
                code: SETTING_INVALID.name,
                message: format!(
                    "`{DRAIN_GRACE_VARIABLE}` is `{milliseconds}`, which is no whole number of milliseconds."
                ),
            })?,
        None => settings
            .duration(DRAIN_GRACE.name)
            .unwrap_or(Duration::from_secs(110)),
    };
    let idle_timeout = settings
        .duration(IDLE_TIMEOUT.name)
        .unwrap_or(Duration::from_secs(600));
    Ok(Configured {
        address,
        drain_grace,
        idle_timeout,
        settings,
    })
}

/// The value of the deployment variable `variable`, which overrides the
/// setting `key` unless the Operator policy locks it.
fn overridden(
    settings: &Resolved,
    environment: &BTreeMap<OsString, OsString>,
    variable: &str,
    key: &str,
) -> Result<Option<String>, Refused> {
    let Some(value) = goodfetch_engine::settings::deployment_variable(environment, variable) else {
        return Ok(None);
    };
    if settings
        .get(key)
        .is_some_and(|effective| effective.locked_by == Some(SettingsLayer::OperatorPolicy))
    {
        return Err(Refused {
            code: SETTING_LOCKED.name,
            message: format!(
                "`{variable}` sets `{key}`, which the Operator policy locks: unset `{variable}`, or change the Operator policy."
            ),
        });
    }
    Ok(Some(text(value)))
}

fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

/// `address` as the foreground Daemon listens at it: a loopback `host:port`,
/// until the Daemon terminates TLS (area 4).
fn parse_address(address: &str) -> Result<SocketAddr, Refused> {
    let parsed = address.parse::<SocketAddr>().map_err(|_| Refused {
        code: SETTING_INVALID.name,
        message: format!(
            "`daemon.address` is `{address}`, which is no `host:port`; set an address and port such as `127.0.0.1:7900`."
        ),
    })?;
    if !parsed.ip().is_loopback() {
        return Err(Refused {
            code: SETTING_INVALID.name,
            message: format!(
                "`daemon.address` is `{address}`, which is not a loopback address: `daemon run` listens only on loopback until the Daemon terminates TLS; put a TLS-terminating proxy in front of a loopback address."
            ),
        });
    }
    Ok(parsed)
}

/// The foreground Daemon runs the Product its operator chose: it is its own
/// pick, and never hands over (ADR 0098).
struct TheOperatorsChoice;

impl Succession for TheOperatorsChoice {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

/// The signals the foreground Daemon drains on, listened for from its
/// start.
struct Signals {
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
}

impl Signals {
    #[cfg(unix)]
    fn listen() -> std::io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    #[cfg(not(unix))]
    fn listen() -> std::io::Result<Self> {
        Ok(Self {})
    }

    /// The first one received, by name.
    #[cfg(unix)]
    async fn received(mut self) -> &'static str {
        tokio::select! {
            _ = self.terminate.recv() => "SIGTERM",
            _ = self.interrupt.recv() => "SIGINT",
        }
    }

    #[cfg(not(unix))]
    async fn received(self) -> &'static str {
        let _ = tokio::signal::ctrl_c().await;
        "Ctrl-C"
    }
}

#[cfg(test)]
mod tests {
    use super::parse_address;

    #[test]
    fn a_loopback_address_is_taken_and_any_other_refused() {
        let codes = [
            "127.0.0.1:0",
            "[::1]:7900",
            "0.0.0.0:7900",
            "192.0.2.1:7900",
            "nowhere",
        ]
        .map(|address| parse_address(address).map_or_else(|refused| refused.code, |_| "ok"));

        assert_eq!(
            codes,
            [
                "ok",
                "ok",
                "setting_invalid",
                "setting_invalid",
                "setting_invalid"
            ]
        );
    }
}
