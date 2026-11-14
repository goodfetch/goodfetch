//! `daemon run`, the foreground Daemon (ADR 0049): the Product its
//! operator chose, with no pick (ADR 0098), under Docker, Kubernetes or
//! systemd. It writes no runtime file and never stops idle; it listens on
//! `daemon.address`, a loopback address until the Daemon terminates TLS
//! (area 4), with newline-delimited JSON-RPC and the protocol over
//! WebSocket, whose handshake must name that address or `localhost` with
//! its port as its `Host` and `Origin`; Clients open with the persistent
//! token of the major's `daemon.token`; it reads `daemon.address` and `daemon.drain_grace` from
//! user configuration and the Operator policy, which
//! `GOODFETCH_DAEMON_ADDRESS` and `GOODFETCH_DAEMON_DRAIN_GRACE` override
//! unless the policy locks them; and its logs are JSON lines on stdout,
//! which `main` set up; under `--json` its `DaemonStatus` comes first,
//! once it listens. It listens for `SIGTERM` and `SIGINT` (Ctrl-C on
//! Windows) before it listens for Clients, and on either it drains: it
//! takes no run, checkpoints each live run as resumable and exits within
//! `daemon.drain_grace`. On Linux it runs under its own reaper (the
//! `reaper` module), which reaps every browser descendant.
//!
//! With `ops.address` set, or `GOODFETCH_OPS_ADDRESS` unless the policy
//! locks it, it serves its ops listener there (ADR 0050): the probes,
//! ready while the pinned browser is in the Browser cache, and `/metrics`
//! behind the ops token of the major's `ops.token`; with
//! `otlp.metrics_endpoint` set, it exports its metrics there.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use crate::executable::output;
use goodfetch_engine::browser_cache::{
    BrowserChoice, BrowserSource, CacheLocation, Pins, Platform, browser_source, entry_present,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::files::DaemonFiles;
use goodfetch_engine::daemon::{DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::endpoint::Authorities;
use goodfetch_engine::ops::{ConfiguredBrowsers, Ops, OpsOptions};
use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{Resolved, SettingsError};
use goodfetch_engine::transport::{Network, listen_ops};
use goodfetch_engine::{Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::daemon::{
    ADDRESS, DRAIN_GRACE, DaemonEndpoint, DaemonStatus, DaemonToken, IDLE_TIMEOUT,
};
use goodfetch_protocol::ops::{ADDRESS as OPS_ADDRESS, OTLP_METRICS_ENDPOINT};
use goodfetch_protocol::settings::{
    DAEMON_ADDRESS_VARIABLE as ADDRESS_VARIABLE,
    DAEMON_DRAIN_GRACE_VARIABLE as DRAIN_GRACE_VARIABLE, OPS_ADDRESS_VARIABLE, SETTING_INVALID,
    SettingLimit, SettingsLayer,
};
use tokio_util::sync::CancellationToken;

use super::super::runtime;
use super::{Failure, Machine, Reach};
use crate::executable::metrics::Export;
use crate::executable::otlp::ToCollector;

/// What the foreground Daemon runs with, from the machine's settings and
/// its environment.
struct Configured {
    address: SocketAddr,
    drain_grace: Duration,
    idle_timeout: Duration,
    /// Where its ops listener listens, if anywhere.
    ops: Option<SocketAddr>,
    /// Where its metrics are exported over OTLP, if anywhere.
    otlp_metrics: Option<String>,
    /// The machine's settings, which choose the run-state layout and the
    /// runtime's threads.
    settings: Resolved,
}

/// Why the foreground Daemon did not start: a setting, or a deployment
/// variable, it cannot take.
struct Refused(SettingsError);

impl From<SettingsError> for Refused {
    fn from(error: SettingsError) -> Self {
        Self(error)
    }
}

/// Where a value the foreground Daemon reads came from: a setting's key
/// and the layer that set it, or a deployment variable, of no layer.
struct Source {
    key: String,
    layer: Option<SettingsLayer>,
}

impl Source {
    fn variable(variable: &str) -> Self {
        Self {
            key: variable.to_owned(),
            layer: None,
        }
    }

    fn setting(settings: &Resolved, setting: &Setting) -> Self {
        Self {
            key: setting.name.to_owned(),
            layer: Some(
                settings
                    .effective(setting)
                    .map_or(SettingsLayer::Default, |effective| effective.layer),
            ),
        }
    }

    /// Its value is not valid: `reason`.
    fn invalid(&self, reason: String) -> Refused {
        Refused(SettingsError::Invalid {
            key: self.key.clone(),
            layer: self.layer,
            reason,
        })
    }
}

/// Runs the foreground Daemon until it drains or `daemon.stop` stops it.
pub(in crate::executable) fn run(
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    json: bool,
) -> ExitCode {
    let identity = engine.identity();
    let configured = match configured(machine) {
        Ok(configured) => configured,
        Err(Refused(error)) => {
            tracing::error!(code = error.code(), "{error}");
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
    let Tokens { token, ops_token } = match tokens(&configured, &files) {
        Ok(tokens) => tokens,
        Err(exit) => return exit,
    };
    let export = match export(&configured, identity, dependencies.clock) {
        Ok(export) => export,
        Err(exit) => return exit,
    };
    let engine = engine.clone().with_metrics(export.metrics());
    let browsers = Arc::new(PinnedBrowser::of(identity, machine));
    let pid = std::process::id();
    let runtime = runtime::multi_thread(runtime::Threads::of(Some(&configured.settings)));
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
        let ops = ops_listener(configured.ops, ops_token, &export, &files).await?;
        let asked = CancellationToken::new();
        let options = DaemonOptions {
            token,
            endpoint: endpoint.clone(),
            pid,
            idle_timeout: configured.idle_timeout,
            keep_alive: true,
            holds: Holds::new(),
            succession: Arc::new(TheOperatorsChoice),
            drain: Drain {
                asked: Some(asked.clone()),
                grace: configured.drain_grace,
            },
            install,
            ops: Ops {
                listener: ops,
                browsers: Some(browsers),
            },
            websocket: match &endpoint {
                DaemonEndpoint::Tcp(bound) => Some(Authorities::loopback(*bound)),
                DaemonEndpoint::Unix(_) => None,
            },
        };
        announce(json, machine, identity, pid, &endpoint);
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
    // Its last export, before the process exits.
    drop(export);
    match ended {
        Ok(Some(DaemonEnd::ListenerFailed | DaemonEnd::Panicked)) => ExitCode::FAILURE,
        Ok(Some(_)) => ExitCode::SUCCESS,
        Ok(None) => {
            tracing::warn!("the drain grace passed before the Daemon stopped: it exits now");
            ExitCode::FAILURE
        }
        Err(exit) => exit,
    }
}

/// Under `--json`, prints the Daemon's `DaemonStatus` once it listens at
/// `endpoint`, before its first log line, as a container's tooling reads
/// it: no service, which a foreground Daemon never has.
fn announce(
    json: bool,
    machine: &Machine<'_>,
    identity: &Identity,
    pid: u32,
    endpoint: &DaemonEndpoint,
) {
    if !json {
        return;
    }
    output::json(&DaemonStatus {
        pid: Some(pid),
        major: machine.major(),
        product: Some(identity.product),
        version: Some(identity.product_version.clone()),
        address: Some(endpoint.to_string()),
        service: None,
    });
}

/// The tokens the foreground Daemon takes: its Clients' persistent token,
/// and the ops token when it opens an ops listener.
struct Tokens {
    token: String,
    ops_token: Option<String>,
}

/// The foreground Daemon's tokens, read or made in `files`.
fn tokens(configured: &Configured, files: &DaemonFiles) -> Result<Tokens, ExitCode> {
    let token = files.persistent_token().map_err(|error| {
        tracing::error!(%error, "the Daemon's persistent token cannot be read or made");
        ExitCode::FAILURE
    })?;
    let ops_token = configured
        .ops
        .map(|_| files.ops_token())
        .transpose()
        .map_err(|error| {
            tracing::error!(%error, "the ops token cannot be read or made");
            ExitCode::FAILURE
        })?;
    Ok(Tokens { token, ops_token })
}

/// The metrics' export `configured` asks for.
fn export(configured: &Configured, identity: &Identity, clock: Clock) -> Result<Export, ExitCode> {
    Export::start(
        configured.ops.is_some(),
        configured
            .otlp_metrics
            .as_deref()
            .map(|endpoint| (endpoint, ToCollector::new(&configured.settings, clock))),
        identity.product,
    )
    .map_err(|error| {
        tracing::error!(code = SETTING_INVALID.name, %error, "the metrics' export cannot start");
        ExitCode::FAILURE
    })
}

/// The ops listener at `address`, if any, serving `/metrics` to
/// `ops_token` from `export`.
async fn ops_listener(
    address: Option<SocketAddr>,
    ops_token: Option<String>,
    export: &Export,
    files: &DaemonFiles,
) -> Result<Option<OpsOptions>, ExitCode> {
    let Some(address) = address else {
        return Ok(None);
    };
    let listener = listen_ops(address).await.map_err(|error| {
        tracing::error!(%address, %error, "the Daemon cannot listen at `ops.address`");
        ExitCode::FAILURE
    })?;
    tracing::info!(
        address = %listener.local_address().map_or_else(|_| String::new(), |address| address.to_string()),
        token_file = %files.ops_token_file().display(),
        "the ops listener serves"
    );
    Ok(ops_token
        .zip(export.scrape())
        .map(|(token, scrape)| OpsOptions {
            listener,
            token,
            scrape: Arc::new(scrape),
        }))
}

/// `daemon.address`, `daemon.drain_grace` and `daemon.idle_timeout` from
/// the machine's settings, the two deployment variables overriding the
/// first two unless the Operator policy locks them; the address must be a
/// loopback one.
fn configured(machine: &Machine<'_>) -> Result<Configured, Refused> {
    let settings = machine.settings.clone()?;
    let (address, source) =
        match overridden(&settings, machine.environment, ADDRESS_VARIABLE, &ADDRESS)? {
            Some(address) => (address, Source::variable(ADDRESS_VARIABLE)),
            None => (
                settings.get::<&str>(&ADDRESS).to_owned(),
                Source::setting(&settings, &ADDRESS),
            ),
        };
    let address = parse_address(&address, &source)?;
    let drain_grace = match overridden(
        &settings,
        machine.environment,
        DRAIN_GRACE_VARIABLE,
        &DRAIN_GRACE,
    )? {
        Some(milliseconds) => milliseconds
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|_| {
                Source::variable(DRAIN_GRACE_VARIABLE).invalid(format!(
                    "`{milliseconds}` is no whole number of milliseconds"
                ))
            })?,
        None => settings.get(&DRAIN_GRACE),
    };
    let idle_timeout = settings.get(&IDLE_TIMEOUT);
    let ops = match overridden(
        &settings,
        machine.environment,
        OPS_ADDRESS_VARIABLE,
        &OPS_ADDRESS,
    )? {
        Some(address) => Some((address, Source::variable(OPS_ADDRESS_VARIABLE))),
        None => settings
            .get::<Option<&str>>(&OPS_ADDRESS)
            .map(|address| (address.to_owned(), Source::setting(&settings, &OPS_ADDRESS))),
    }
    .map(|(address, source)| parse_ops_address(&address, &source))
    .transpose()?;
    let otlp_metrics = settings
        .get::<Option<&str>>(&OTLP_METRICS_ENDPOINT)
        .map(str::to_owned);
    Ok(Configured {
        address,
        drain_grace,
        idle_timeout,
        ops,
        otlp_metrics,
        settings,
    })
}

/// Where the foreground Daemon of the machine's settings is reached, and
/// the persistent token its Clients open with; `None` when no persistent
/// token was ever made, so none ever ran.
///
/// # Errors
///
/// When `daemon.address` is invalid, or the token cannot be read.
pub(super) fn reach(machine: &Machine<'_>) -> Result<Option<Reach>, Failure> {
    let configured = configured(machine).map_err(|Refused(error)| Failure::Refused {
        error: local_settings_error(&error, machine.identity).to_error_object(),
        operation: None,
    })?;
    let token = machine.files().read_token().map_err(|error| {
        Failure::Unavailable(format!(
            "the Daemon's persistent token cannot be read: {error}"
        ))
    })?;
    Ok(token.map(|token| Reach {
        endpoint: DaemonEndpoint::Tcp(configured.address),
        token: DaemonToken::new(token),
        service: false,
        remote: false,
    }))
}

/// `address` as the ops listener listens at it: any `host:port`, so a
/// cluster's probes and scrapers reach it.
fn parse_ops_address(address: &str, source: &Source) -> Result<SocketAddr, Refused> {
    address.parse::<SocketAddr>().map_err(|_| {
        source.invalid(format!(
            "`{address}` is no `host:port`; set an address and port such as `0.0.0.0:9464`"
        ))
    })
}

/// The browser `daemon run` launches, as a Run's page does: the build the
/// Product version pins for this platform, in the Browser cache, or the
/// distribution's Chromium where no build is made.
struct PinnedBrowser {
    pins: Pins,
    cache: PathBuf,
    environment: BTreeMap<OsString, OsString>,
}

impl PinnedBrowser {
    fn of(identity: &Identity, machine: &Machine<'_>) -> Self {
        Self {
            pins: Pins {
                pinned_by: PinnedBy {
                    product: identity.product,
                    product_version: identity.product_version.clone(),
                },
                artifacts: crate::pinned_artifacts(),
            },
            cache: CacheLocation::new(
                machine.home.root(),
                machine.home.browsers_path().map(Path::to_path_buf),
            )
            .root()
            .to_owned(),
            environment: machine.environment.clone(),
        }
    }
}

impl ConfiguredBrowsers for PinnedBrowser {
    fn installed(&self) -> bool {
        match browser_source(
            Platform::current(),
            &BrowserChoice::Default,
            &self.environment,
        ) {
            BrowserSource::Pinned { name, platform, .. } => self
                .pins
                .browser(name, platform)
                .is_some_and(|artifact| entry_present(&self.cache, &artifact.id)),
            BrowserSource::Installed { candidates, .. } => {
                candidates.iter().any(|candidate| candidate.is_file())
            }
        }
    }
}

/// The value of the deployment variable `variable`, which overrides the
/// setting `key` unless the Operator policy locks it.
fn overridden(
    settings: &Resolved,
    environment: &BTreeMap<OsString, OsString>,
    variable: &str,
    setting: &Setting,
) -> Result<Option<String>, Refused> {
    let key = setting.name;
    let Some(value) = goodfetch_engine::settings::deployment_variable(environment, variable) else {
        return Ok(None);
    };
    if let Some(effective) = settings
        .effective(setting)
        .filter(|effective| effective.locked_by == Some(SettingsLayer::OperatorPolicy))
    {
        // The variable stands in for the machine's settings, the user
        // configuration, whose value the lock refuses.
        return Err(Refused(SettingsError::Locked {
            key: key.to_owned(),
            layer: SettingsLayer::UserConfiguration,
            limit: SettingLimit {
                key: key.to_owned(),
                value: effective.value.clone(),
                layer: SettingsLayer::OperatorPolicy,
            },
            fix: format!("`{variable}` sets it: unset `{variable}`, or change the Operator policy"),
        }));
    }
    Ok(Some(text(value)))
}

fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

/// `address` as the foreground Daemon listens at it: a loopback `host:port`,
/// until the Daemon terminates TLS (area 4).
fn parse_address(address: &str, source: &Source) -> Result<SocketAddr, Refused> {
    let parsed = address.parse::<SocketAddr>().map_err(|_| {
        source.invalid(format!(
            "`{address}` is no `host:port`; set an address and port such as `127.0.0.1:7900`"
        ))
    })?;
    if !parsed.ip().is_loopback() {
        return Err(source.invalid(format!(
            "`{address}` is not a loopback address: `daemon run` listens only on loopback until the Daemon terminates TLS; put a TLS-terminating proxy in front of a loopback address"
        )));
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
    #[expect(
        clippy::unnecessary_wraps,
        reason = "listening for Unix signals can fail, for Ctrl-C never: one signature serves both"
    )]
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
    use super::{Source, parse_address};

    #[test]
    fn a_loopback_address_is_taken_and_any_other_refused() {
        let source = Source::variable(super::ADDRESS_VARIABLE);
        let codes = [
            "127.0.0.1:0",
            "[::1]:7900",
            "0.0.0.0:7900",
            "192.0.2.1:7900",
            "nowhere",
        ]
        .map(|address| {
            parse_address(address, &source).map_or_else(|refused| refused.0.code(), |_| "ok")
        });

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
