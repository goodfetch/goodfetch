//! The Daemon from both Products' executables (ADRs 0024, 0030 and 0098):
//! `daemon start|stop|restart|status|health`, the hidden `daemon launched`
//! that a start runs from the pick's versioned copy, and the Client side
//! every command that reaches the Daemon shares. `daemon health` asks the
//! running Daemon of the major, or else the foreground Daemon at
//! `daemon.address` with the persistent token, what its ops listener's
//! probes check (ADR 0050), for `exec` probes and systemd.
//!
//! Each executable records itself in the list of installed executables
//! when it runs. Whatever starts the Daemon, under the start lock so two
//! starts start one, starts its major's pick from the pick's versioned copy
//! in the Browser cache and waits for the line the Daemon prints once its
//! runtime file is written. A Client finds the Daemon through that file,
//! replaces one whose Daemon died, and opens with `hello` and the file's
//! token; a refusal marked as a handover makes it wait for the pick, and
//! any other refusal fails with a `fix` naming this executable's
//! `daemon restart`. A Daemon that hands over starts the pick itself.
//!
//! Once `daemon install` registered the major's service with socket
//! activation (macOS and Linux), a Client that finds no running Daemon
//! connects to the service's socket with the persistent token, and that
//! connection starts the Daemon (ADR 0031). The service runs `daemon
//! launched --service`, which takes the place of the pick's copy when it
//! is not the pick, takes the socket its service manager hands over, and
//! leaves that socket to it; a Daemon a service activated hands over by
//! stopping, so the next connection starts the pick.
//!
//! The `daemon` commands need Collie, which carries `daemon.*`, so either
//! executable stops or asks a Daemon of either Product; a command that
//! runs the engine needs its executable's own Product (spec #213).
//!
//! Size: one concern, the executables' side of the Daemon: the `daemon`
//! commands and the start every Client shares, kept whole so finding,
//! starting and handing over a Daemon follow one start lock.
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI reads the machine's files, starts the Daemon's process and builds its runtime before and outside any async worker (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use goodfetch_engine::browser_cache::{
    self, BROWSERS_PATH_VARIABLE, CacheLocation, HOME_FOLDER, MaxUnusedAge,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::files::DaemonFiles;
use goodfetch_engine::daemon::{RefusedClient, RefusedExecutable, refusal_fix};
use goodfetch_engine::home::Home;
use goodfetch_engine::installed::{InstalledList, Seen, fingerprint, pick, stores_floor};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::{CallError, Identity};
use goodfetch_protocol::browser_cache::MAX_UNUSED_AGE;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{NOT_IN_PRODUCT, PROTOCOL_VERSION_UNSUPPORTED};
use goodfetch_protocol::console::{ConsoleFailure, ConsoleState};
use goodfetch_protocol::daemon::{
    DaemonEndpoint, DaemonHealth, DaemonHealthOutput, DaemonRuntimeFile, DaemonState, DaemonStatus,
    DaemonStopping, DaemonToken, HealthProbe, Unready,
};
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable};
use goodfetch_protocol::jsonrpc::ErrorObject;
use goodfetch_protocol::run::{LAYOUT_PER_INSTALL, RUN_STATE_LAYOUT};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::engine_choice::{Interrupts, Link, Opening, hello, unreachable};
use super::exit_codes;
use super::output::{self, Human};

use super::browsers;

mod engine_choice;
mod foreground;
mod launched;
#[cfg(target_os = "linux")]
mod reaper;

use launched::launch;
pub(super) use launched::{launched, start_console};

/// `daemon run`, the foreground Daemon (ADR 0049); on Linux, under its
/// reaper unless `reaped` says it runs under it already.
pub(super) fn run(
    engine: &goodfetch_engine::Engine,
    dependencies: goodfetch_engine::Dependencies,
    machine: &Machine<'_>,
    reaped: bool,
    json: bool,
) -> ExitCode {
    #[cfg(target_os = "linux")]
    if !reaped {
        let Some(executable) = machine.executable.as_deref() else {
            tracing::error!(
                "the executable's own path is unknown, so its reaper cannot start the Daemon"
            );
            return ExitCode::FAILURE;
        };
        return reaper::reap_around(executable, machine.environment, json);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = reaped;
    foreground::run(engine, dependencies, machine, json)
}

/// Whether the machine's `settings` choose the per-install layout
/// (`run_state.layout`), in which the Daemon alone writes run state.
pub(super) fn per_install(settings: Option<&Resolved>) -> bool {
    settings.is_some_and(|settings| {
        settings.value(&RUN_STATE_LAYOUT).as_str() == Some(LAYOUT_PER_INSTALL)
    })
}

/// How many times a Client looks for the Daemon again when the one it
/// found went away, or handed over, before it gives up.
const ATTEMPTS: usize = 3;

/// How long a Client waits for a Daemon that went away to stop before it
/// looks again: a stop checkpoints the Daemon's Runs, each within
/// `timeout.close`'s 30 seconds.
const STOPPING: Duration = Duration::from_secs(30);

/// How often a Client that waits for a stopping Daemon looks at its
/// runtime file.
const LOOK_AGAIN: Duration = Duration::from_millis(50);

/// What every `daemon` command and every Client of the Daemon works with.
pub(super) struct Machine<'a> {
    /// The executable's Product, version and channel.
    pub(super) identity: &'a Identity,
    /// Goodfetch home.
    pub(super) home: &'a Home,
    /// The environment `main` read once, which a started Daemon gets.
    pub(super) environment: &'a BTreeMap<OsString, OsString>,
    /// This executable's path, as the OS started it.
    pub(super) executable: Option<PathBuf>,
    /// Where it reads wall time: the engine's Clock.
    pub(super) clock: Clock,
}

/// Why reaching the Daemon failed.
pub(super) enum Failure {
    /// The Daemon, or this executable before it, refused or failed the
    /// call `operation` with an Error code; `None` when it failed before
    /// any call.
    Refused {
        error: ErrorObject,
        operation: Option<String>,
    },
    /// The Daemon refused this Client's `hello` by Product or by protocol
    /// version and cannot hand over, its `fix` naming this executable's
    /// `daemon restart`.
    DaemonRefused(ErrorObject),
    /// The Daemon cannot be started or reached: `daemon_unreachable`.
    Unavailable(String),
}

/// The `fix` of a Daemon that cannot be started or reached.
const UNAVAILABLE_FIX: &str = "Check that Goodfetch home can be read and written and `daemon status`, then run the command again.";

impl Failure {
    /// A failure with the Error code of `error`, before any call.
    pub(super) fn of(error: ErrorObject) -> Self {
        Self::Refused {
            error,
            operation: None,
        }
    }

    /// Logs it and answers its exit code, as `daemon launched` does, whose
    /// output is the Daemon's log.
    pub(super) fn report(&self) -> ExitCode {
        match self {
            Self::Refused { error, .. } => {
                tracing::error!(code = error.data.code.as_str(), "{}", self.text());
                ExitCode::from(exit_codes::of_error(
                    &error.data.code,
                    exit_codes::group(error),
                ))
            }
            Self::DaemonRefused(error) => {
                tracing::error!(code = error.data.code.as_str(), "{}", self.text());
                ExitCode::from(exit_codes::DAEMON_REFUSED)
            }
            Self::Unavailable(problem) => {
                tracing::error!("{problem}");
                ExitCode::FAILURE
            }
        }
    }

    /// Prints it as every command's failure prints (ADR 0069): under
    /// `json` the `CommandFailed` on stdout, else its message, `code`,
    /// `retryable` and `fix` on stderr; answers its exit code, its Error
    /// code's row, or `11` for a refusal of `hello` by Product or version.
    pub(super) fn print(self, json: bool, human: &Human) -> ExitCode {
        match self {
            Self::Refused { error, operation } => {
                output::fail(error, operation.as_deref(), json, human)
            }
            Self::DaemonRefused(error) => {
                output::fail_exiting(error, None, exit_codes::DAEMON_REFUSED, json, human)
            }
            Self::Unavailable(problem) => {
                output::fail(unreachable(problem, UNAVAILABLE_FIX), None, json, human)
            }
        }
    }

    /// Its error object, with its Error code.
    fn into_error(self) -> ErrorObject {
        match self {
            Self::Refused { error, .. } | Self::DaemonRefused(error) => error,
            Self::Unavailable(problem) => unreachable(problem, UNAVAILABLE_FIX),
        }
    }

    /// What it says, for people: its message and its `fix`.
    fn text(&self) -> String {
        match self {
            Self::Refused { error, .. } | Self::DaemonRefused(error) => {
                let fix = error.data.details.get("fix").and_then(Value::as_str);
                format!("{} {}", error.message, fix.unwrap_or_default())
                    .trim()
                    .to_owned()
            }
            Self::Unavailable(problem) => problem.clone(),
        }
    }
}

fn unavailable(what: &str) -> impl Fn(std::io::Error) -> Failure + '_ {
    move |error| Failure::Unavailable(format!("{what}: {error}"))
}

impl Machine<'_> {
    /// Wall time now, on the engine's Clock.
    pub(super) fn now(&self) -> jiff::Timestamp {
        now_of(self.clock)
    }

    pub(super) fn files(&self) -> DaemonFiles {
        DaemonFiles::of(self.home, &self.identity.product_version)
    }

    pub(super) fn major(&self) -> String {
        self.identity.product_version.line().to_string()
    }

    pub(super) fn list(&self) -> InstalledList {
        InstalledList::of(self.home)
    }

    /// The Browser cache the Daemon's copies go in: the one this executable
    /// uses, or `browsers` in Goodfetch home when that one cannot be
    /// written (ADR 0098's note under #259).
    pub(super) fn cache(&self) -> PathBuf {
        let location = CacheLocation::new(
            self.home.root(),
            self.environment
                .get(std::ffi::OsStr::new(BROWSERS_PATH_VARIABLE))
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        );
        let root = location.root().to_owned();
        if std::fs::create_dir_all(&root).is_ok()
            && tempfile::Builder::new()
                .prefix(".write-check")
                .tempfile_in(&root)
                .is_ok()
        {
            root
        } else {
            self.home.root().join(HOME_FOLDER)
        }
    }

    /// `browser_cache.max_unused_age` from the machine's settings.
    fn max_unused_age(&self) -> MaxUnusedAge {
        browsers::machine_settings(
            self.home,
            self.identity.product,
            &self.identity.product_version,
        )
        .map(|settings| MaxUnusedAge::new(settings.get(&MAX_UNUSED_AGE)))
        .unwrap_or_default()
    }

    /// The major's pick now.
    pub(super) fn pick(&self) -> Option<InstalledExecutable> {
        let entries = self.list().read().unwrap_or_default();
        let cache = self.cache();
        let seen: Vec<Seen<'_>> = entries
            .iter()
            .map(|entry| Seen::look(entry, &cache))
            .collect();
        pick(
            &seen,
            &self.identity.product_version,
            stores_floor(self.home, &self.identity.product_version),
            self.now(),
            self.max_unused_age(),
        )
        .cloned()
    }

    /// Whether this executable is its major's pick.
    fn is_the_pick(&self) -> bool {
        self.pick().is_none_or(|picked| {
            picked.product == self.identity.product
                && picked.product_version == self.identity.product_version
        })
    }

    /// The running Daemon, from its runtime file.
    fn running(&self) -> Result<Option<Reach>, Failure> {
        Ok(self
            .files()
            .running()
            .map_err(unavailable("the Daemon's runtime file cannot be read"))?
            .map(Reach::from))
    }

    /// The socket of the major's installed service and its persistent
    /// token, whose first connection starts the Daemon; `None` when no
    /// service with socket activation is installed.
    pub(super) fn activated_service(&self) -> Result<Option<Reach>, Failure> {
        let files = self.files();
        let Some(socket) = files
            .service()
            .map_err(unavailable("the installed service's record cannot be read"))?
            .and_then(|service| service.socket)
        else {
            return Ok(None);
        };
        let token = files
            .read_token()
            .map_err(unavailable("the Daemon's persistent token cannot be read"))?
            .ok_or_else(|| {
                Failure::Unavailable(format!(
                    "the service of major {} has no persistent token at {}: run `daemon install` again",
                    self.major(),
                    files.token_file().display()
                ))
            })?;
        Ok(Some(Reach {
            endpoint: DaemonEndpoint::Unix(socket),
            token: DaemonToken::new(token),
            service: true,
        }))
    }

    /// The running Daemon, the installed service's socket, which starts
    /// one at its first connection, or one this starts: the pick, under
    /// the start lock. `true` when no Daemon ran before.
    fn running_or_started(&self, keep_alive: bool) -> Result<(Reach, bool), Failure> {
        if let Some(found) = self.running()? {
            return Ok((found, false));
        }
        if let Some(service) = self.activated_service()? {
            return Ok((service, true));
        }
        let start = self
            .files()
            .start_lock()
            .map_err(unavailable("the Daemon's start lock cannot be taken"))?;
        if let Some(found) = self.running()? {
            return Ok((found, false));
        }
        let started = self.start_pick(keep_alive)?;
        drop(start);
        Ok((Reach::from(started), true))
    }

    /// The pick when it is not this executable, which a service's
    /// `daemon launched` gives its place to.
    #[cfg(unix)]
    pub(super) fn other_pick(&self) -> Option<InstalledExecutable> {
        self.pick().filter(|picked| {
            picked.product != self.identity.product
                || picked.product_version != self.identity.product_version
        })
    }

    /// The pick's versioned copy, placed in the Browser cache unless it is
    /// there: the executable to run and its build.
    pub(super) fn copy_of(
        &self,
        picked: &InstalledExecutable,
    ) -> Result<(PathBuf, String), Failure> {
        let cache = self.cache();
        let copy = match &picked.copy {
            Some(copy) if browser_cache::entry_present(&cache, copy) => copy.clone(),
            _ => {
                let placed = browser_cache::place_executable_copy(
                    &cache,
                    Path::new(&picked.path),
                    picked.product,
                    &picked.product_version,
                    self.now(),
                )
                .map_err(unavailable("the Daemon's versioned copy cannot be made"))?;
                let recorded = InstalledExecutable {
                    copy: Some(placed.id.clone()),
                    ..picked.clone()
                };
                if let Err(error) = self.list().record(&recorded) {
                    tracing::warn!(%error, "the versioned copy is not recorded in the list of installed executables");
                }
                placed.id
            }
        };
        let file_name = Path::new(&picked.path)
            .file_name()
            .map(ToOwned::to_owned)
            .unwrap_or_default();
        let executable = cache
            .join(copy.kind.as_str())
            .join(&copy.name)
            .join(&copy.build)
            .join("files")
            .join(file_name);
        Ok((executable, copy.build))
    }

    /// Starts the major's pick from its versioned copy and waits for its
    /// runtime file; whoever calls it holds the start lock.
    pub(super) fn start_pick(&self, keep_alive: bool) -> Result<DaemonRuntimeFile, Failure> {
        let Some(picked) = self.pick() else {
            let install = stores_floor(self.home, &self.identity.product_version).map_or_else(
                || self.identity.product_version.to_string(),
                |floor| floor.to_string(),
            );
            return Err(Failure::Unavailable(format!(
                "no installed executable can run the Daemon of major {}, whose shared stores need Goodfetch {install} or newer: install {} {install} or newer",
                self.major(),
                self.identity.product.name(),
            )));
        };
        let (executable, build) = self.copy_of(&picked)?;
        launch(&executable, &build, keep_alive, self.environment, self.home)
    }

    /// A Connection to the Daemon that `hello` opened, needing `needs`: the
    /// running Daemon, or the pick this starts when `start` and none runs.
    pub(super) fn connect(
        &self,
        start: bool,
        needs: Product,
    ) -> Result<Option<Connected>, Failure> {
        let runtime = runtime();
        for _ in 0..ATTEMPTS {
            let found = if start {
                Some(self.running_or_started(false)?.0)
            } else {
                self.running()?
            };
            let Some(found) = found else {
                return Ok(None);
            };
            match runtime.block_on(self.open(&found, needs)) {
                Ok(connection) => {
                    return Ok(Some(Connected {
                        runtime,
                        connection,
                        endpoint: found.endpoint,
                        next: 1,
                    }));
                }
                // The service's socket answers no connection: its service
                // manager does not listen there.
                Err(Opening::Unreachable) if found.service => {
                    return Err(Failure::Unavailable(format!(
                        "the service of major {} does not answer at {}: run `daemon install` again to repair it, or `daemon uninstall`",
                        self.major(),
                        found.endpoint
                    )));
                }
                // It hands over: once the start lock is free, the pick runs.
                Err(Opening::HandOver(_)) => drop(
                    self.files()
                        .start_lock()
                        .map_err(unavailable("the Daemon's start lock cannot be taken"))?,
                ),
                // It went away in between: once it stopped, look again. A
                // `daemon` command's Link listens for no interrupt.
                Err(Opening::Gone | Opening::Unreachable | Opening::Interrupted(_)) => {
                    self.stopped(&found.endpoint)?;
                }
                Err(Opening::Refused(error)) => return Err(self.refused(error)),
            }
        }
        Err(Failure::Unavailable(format!(
            "the Daemon of major {} went away {ATTEMPTS} times while this Client connected",
            self.major()
        )))
    }

    /// A Link to the Daemon `found`, opened with this Client's `hello`
    /// needing `needs`; it leaves the interrupts to their default.
    async fn open(&self, found: &Reach, needs: Product) -> Result<Link, Opening> {
        let hello = hello(self.identity, needs, Some(&found.token));
        Link::open(&found.endpoint, &hello, Interrupts::none).await
    }

    /// The refusal of this Client, its `fix` adding this executable's
    /// `daemon restart` (ADR 0098).
    fn refused(&self, mut error: ErrorObject) -> Failure {
        let given = error
            .data
            .details
            .get("fix")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let fix = self.fix_of(given);
        error
            .data
            .details
            .insert("fix".to_owned(), Value::from(fix));
        if [NOT_IN_PRODUCT.name, PROTOCOL_VERSION_UNSUPPORTED.name]
            .contains(&error.data.code.as_str())
        {
            Failure::DaemonRefused(error)
        } else {
            Failure::Refused {
                error,
                operation: None,
            }
        }
    }

    /// Waits, at most [`STOPPING`], until the Daemon of the major that
    /// listened at `endpoint` and went away has stopped: a stopping Daemon
    /// drops its listener first, then stops its Runs, and only then lets
    /// its runtime file and serving lock go (608-4).
    pub(super) fn stopped(&self, endpoint: &DaemonEndpoint) -> Result<(), Failure> {
        for _ in 0..STOPPING.as_millis() / LOOK_AGAIN.as_millis() {
            match self.running()? {
                Some(found) if &found.endpoint == endpoint => std::thread::sleep(LOOK_AGAIN),
                _ => return Ok(()),
            }
        }
        Ok(())
    }

    /// The `fix` of a refusal whose own is `given`, adding this
    /// executable's `daemon restart` by its path (ADR 0098).
    fn fix_of(&self, given: &str) -> String {
        let executable = self.executable.clone().unwrap_or_default();
        let ours = refusal_fix(&RefusedClient {
            product: self.identity.product,
            version: &self.identity.product_version,
            executable: RefusedExecutable::Path(&executable),
            channel: self.identity.channel,
            would_be_pick: self.is_the_pick(),
        });
        format!("{given} {ours}").trim().to_owned()
    }

    /// Records this executable in the list of installed executables.
    pub(super) fn record(&self) {
        let Some(path) = &self.executable else {
            return;
        };
        let recorded = fingerprint(path).and_then(|(size, modified_at)| {
            self.list().record(&InstalledExecutable {
                kind: ExecutableKind::ProductExecutable,
                product: self.identity.product,
                product_version: self.identity.product_version.clone(),
                path: path.display().to_string(),
                size,
                modified_at,
                pins: crate::pinned_artifacts()
                    .into_iter()
                    .map(|artifact| artifact.id)
                    .collect(),
                copy: None,
                last_seen: self.now(),
            })
        });
        if let Err(error) = recorded {
            tracing::warn!(%error, "this executable is not recorded in the list of installed executables");
        }
    }

    /// Asks the running Daemon to stop and waits until it has; `None` when
    /// none runs.
    pub(super) fn stop(&self) -> Result<Option<DaemonStopping>, Failure> {
        let Some(mut connected) = self.connect(false, Product::Collie)? else {
            return Ok(None);
        };
        let stopping: DaemonStopping = connected.call("daemon.stop", &serde_json::json!({}))?;
        connected.close();
        // The Daemon holds its serving lock until it has stopped.
        drop(
            self.files()
                .serving_lock()
                .map_err(unavailable("the Daemon's serving lock cannot be taken"))?,
        );
        Ok(Some(stopping))
    }

    /// Where the running Daemon is reached and the token its `hello`
    /// takes, starting the pick when none runs: what an Attached engine in
    /// the per-install layout relays its Client to (ADR 0002).
    pub(super) fn reach_for_relay(&self) -> Result<(DaemonEndpoint, DaemonToken), Failure> {
        let (found, _) = self.running_or_started(false)?;
        Ok((found.endpoint, found.token))
    }

    /// The Console's sign-in link, `<url>#token=<token>`, starting the
    /// major's pick unless a Daemon runs: its Console's URL, with the token
    /// in the fragment, which never reaches a server (ADR 0024).
    pub(super) fn console_link(&self) -> Result<String, Failure> {
        let (found, _) = self.running_or_started(false)?;
        let mut connected = self.connect(true, Product::Collie)?.ok_or_else(|| {
            Failure::Unavailable("the Daemon stopped as soon as it started".to_owned())
        })?;
        let status: DaemonState = connected.call("daemon.status", &serde_json::json!({}))?;
        connected.close();
        match status.console {
            ConsoleState::Serving(url) => Ok(format!("{url}#token={}", found.token.expose())),
            ConsoleState::Failed(failure) => Err(Failure::of(console_failed(failure))),
            ConsoleState::Absent => Err(Failure::Unavailable(format!(
                "the Daemon of major {} serves no Console: its executable carries no Console build",
                self.major()
            ))),
        }
    }

    /// What the running Daemon of the major, or else the foreground Daemon
    /// at `daemon.address`, answers `daemon.health`; `None` when neither
    /// answers.
    fn health(&self) -> Result<Option<DaemonHealth>, Failure> {
        if let Some(mut connected) = self.connect(false, Product::Collie)? {
            let health = connected.call::<DaemonHealth>("daemon.health", &serde_json::json!({}));
            connected.close();
            return health.map(Some);
        }
        let Some(found) = foreground::reach(self)? else {
            return Ok(None);
        };
        let Some(mut connected) = self.open_foreground(found)? else {
            return Ok(None);
        };
        let health = connected.call::<DaemonHealth>("daemon.health", &serde_json::json!({}));
        connected.close();
        health.map(Some)
    }

    /// A Connection to the foreground Daemon `found` names, opened with
    /// the persistent token; `None` when nothing answers there.
    fn open_foreground(&self, found: Reach) -> Result<Option<Connected>, Failure> {
        let runtime = runtime();
        let connection = match runtime.block_on(self.open(&found, Product::Collie)) {
            Ok(connection) => connection,
            Err(Opening::Refused(error)) => return Err(self.refused(error)),
            Err(
                Opening::HandOver(_)
                | Opening::Gone
                | Opening::Unreachable
                | Opening::Interrupted(_),
            ) => return Ok(None),
        };
        Ok(Some(Connected {
            runtime,
            connection,
            endpoint: found.endpoint,
            next: 1,
        }))
    }

    /// A Connection to the foreground Daemon at `daemon.address`, which
    /// takes the persistent token, opened with that token as it is now;
    /// `None` when none answers there, or when `daemon.address` is one no
    /// Daemon can listen at.
    pub(super) fn foreground(&self) -> Result<Option<Connected>, Failure> {
        match foreground::reach(self) {
            Ok(Some(found)) => self.open_foreground(found),
            Ok(None) | Err(Failure::Refused { .. }) => Ok(None),
            Err(failure) => Err(failure),
        }
    }

    /// Starts the Daemon unless it runs, and answers whether this call
    /// started it and its state.
    fn start(&self, keep_alive: bool) -> Result<(bool, DaemonState), Failure> {
        let (_, started) = self.running_or_started(keep_alive)?;
        let mut connected = self.connect(true, Product::Collie)?.ok_or_else(|| {
            Failure::Unavailable("the Daemon stopped as soon as it started".to_owned())
        })?;
        let daemon: DaemonState = connected.call("daemon.status", &serde_json::json!({}))?;
        connected.close();
        Ok((started, daemon))
    }

    /// The major's Daemon as `--json` prints it: `daemon`, or none, and the
    /// installed service.
    pub(super) fn status_of(&self, daemon: Option<&DaemonState>) -> DaemonStatus {
        DaemonStatus {
            pid: daemon.map(|daemon| daemon.pid),
            major: self.major(),
            product: daemon.map(|daemon| daemon.product),
            version: daemon.map(|daemon| daemon.product_version.clone()),
            address: daemon.map(|daemon| daemon.endpoint.to_string()),
            console_url: daemon.and_then(|daemon| match &daemon.console {
                ConsoleState::Serving(url) => Some(url.clone()),
                ConsoleState::Failed(_) | ConsoleState::Absent => None,
            }),
            console_error: daemon.and_then(|daemon| match &daemon.console {
                ConsoleState::Failed(failure) => Some(failure.clone()),
                ConsoleState::Serving(_) | ConsoleState::Absent => None,
            }),
            service: self.files().service().ok().flatten(),
        }
    }
}

/// Where a Client reaches the Daemon, and the token its `hello` carries.
pub(super) struct Reach {
    endpoint: DaemonEndpoint,
    token: DaemonToken,
    /// Whether it is the installed service's socket, which starts the
    /// Daemon at its first connection.
    service: bool,
}

impl From<DaemonRuntimeFile> for Reach {
    fn from(found: DaemonRuntimeFile) -> Self {
        Self {
            endpoint: found.endpoint,
            token: found.token,
            service: false,
        }
    }
}

/// A Client's open Connection to the Daemon.
pub(super) struct Connected {
    runtime: tokio::runtime::Runtime,
    connection: Link,
    endpoint: DaemonEndpoint,
    next: i64,
}

impl Connected {
    /// Calls `method` with `params`, and reads its result.
    pub(super) fn call<R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &impl Serialize,
    ) -> Result<R, Failure> {
        let id = self.next;
        self.next += 1;
        let params = serde_json::to_value(params).expect("invariant: params encode");
        let answered = self
            .runtime
            .block_on(self.connection.request(id, method, params));
        let failed = |error| Failure::Refused {
            error,
            operation: Some(method.to_owned()),
        };
        match answered {
            Ok(result) => serde_json::from_value(result).map_err(|error| {
                failed(exit_codes::internal(format!(
                    "The Daemon answered `{method}` with no result of it: {error}."
                )))
            }),
            Err(None) => Err(failed(super::engine_choice::connection_lost(
                &self.endpoint,
                method,
            ))),
            Err(Some(error)) => Err(failed(error)),
        }
    }

    /// Closes the Connection.
    pub(super) fn close(mut self) {
        self.runtime.block_on(self.connection.close());
    }
}

/// `daemon start`.
pub(super) fn start(
    machine: &Machine<'_>,
    keep_alive: bool,
    json: bool,
    human: &Human,
) -> ExitCode {
    match machine.start(keep_alive) {
        Ok((started, daemon)) => {
            if json {
                output::json(&machine.status_of(Some(&daemon)));
            } else {
                let verb = if started { "started" } else { "already ran" };
                output::line(&format!(
                    "The Daemon of major {} {verb}: {}",
                    machine.major(),
                    describe(&daemon)
                ));
            }
            console_exit(&daemon.console, json, human)
        }
        Err(failure) => failure.print(json, human),
    }
}

/// The exit code of a started Daemon whose Console is `console`: its Error
/// code's when the Console did not start, which the Daemon's protocol
/// outlives, else success. Under `json` the status printed carries that
/// failure as `console_error`; otherwise it prints as a failure does.
fn console_exit(console: &ConsoleState, json: bool, human: &Human) -> ExitCode {
    let ConsoleState::Failed(failure) = console else {
        return ExitCode::SUCCESS;
    };
    let exit = exit_codes::of_code(&failure.code);
    if !json {
        output::failed(
            &output::command_failed(console_failed(failure.clone()), None),
            false,
            human,
        );
    }
    ExitCode::from(exit)
}

/// The failure of a Daemon's Console that did not start, `failure`, with
/// its Error code's group.
fn console_failed(failure: ConsoleFailure) -> ErrorObject {
    let mut details = serde_json::Map::new();
    details.insert("fix".to_owned(), Value::from(failure.fix));
    match goodfetch_protocol::entries::error_code(&failure.code) {
        Some(definition) => CallError::new(definition, failure.message, &details).to_error_object(),
        None => exit_codes::unknown(
            failure.code,
            goodfetch_protocol::catalog::ErrorGroup::Unavailable.json_rpc_class(),
            failure.message,
            details,
        ),
    }
}

/// `daemon stop`.
pub(super) fn stop(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    match machine.stop() {
        Ok(stopped) => {
            report_stop(machine, json, stopped.as_ref());
            ExitCode::SUCCESS
        }
        Err(failure) => failure.print(json, human),
    }
}

fn report_stop(machine: &Machine<'_>, json: bool, stopped: Option<&DaemonStopping>) {
    if json {
        output::json(&machine.status_of(None));
        return;
    }
    match stopped {
        Some(stopped) => output::line(&format!(
            "The Daemon of major {} stopped, and its {} live Runs with it",
            machine.major(),
            stopped.live_runs
        )),
        None => output::line(&format!("No Daemon of major {} runs", machine.major())),
    }
}

/// `daemon restart`: stops the running Daemon, then starts the pick.
pub(super) fn restart(
    machine: &Machine<'_>,
    keep_alive: bool,
    json: bool,
    human: &Human,
) -> ExitCode {
    if let Err(failure) = machine.stop() {
        return failure.print(json, human);
    }
    start(machine, keep_alive, json, human)
}

/// `daemon status`.
pub(super) fn status(machine: &Machine<'_>, json: bool, human: &Human) -> ExitCode {
    let daemon = match machine.connect(false, Product::Collie) {
        Ok(Some(mut connected)) => {
            let status = connected.call::<DaemonState>("daemon.status", &serde_json::json!({}));
            connected.close();
            match status {
                Ok(status) => Some(status),
                Err(failure) => return failure.print(json, human),
            }
        }
        Ok(None) => None,
        Err(failure) => return failure.print(json, human),
    };
    let service = machine.files().service().ok().flatten();
    if json {
        output::json(&machine.status_of(daemon.as_ref()));
    } else {
        match &daemon {
            Some(status) => output::line(&format!(
                "The Daemon of major {}: {}",
                machine.major(),
                describe(status)
            )),
            None => output::line(&format!("No Daemon of major {} runs", machine.major())),
        }
        if let Some(service) = &service {
            output::line(&super::service::describe(service));
        }
    }
    ExitCode::SUCCESS
}

/// `daemon health`: whether the Daemon is ready, or live when `live`.
pub(super) fn health(machine: &Machine<'_>, live: bool, json: bool, human: &Human) -> ExitCode {
    let daemon = match machine.health() {
        Ok(daemon) => daemon,
        Err(failure) => return failure.print(json, human),
    };
    let probe = if live {
        HealthProbe::Live
    } else {
        HealthProbe::Ready
    };
    let healthy = daemon.as_ref().is_some_and(|daemon| match probe {
        HealthProbe::Live => daemon.live,
        HealthProbe::Ready => daemon.ready,
    });
    if json {
        output::json(&DaemonHealthOutput {
            probe,
            healthy,
            daemon,
        });
    } else {
        let major = machine.major();
        output::line(&match (&daemon, probe) {
            (None, _) => format!("No Daemon of major {major} answers"),
            (Some(_), HealthProbe::Live) if healthy => {
                format!("The Daemon of major {major} is live")
            }
            (Some(_), HealthProbe::Live) => format!("The Daemon of major {major} is not live"),
            (Some(_), HealthProbe::Ready) if healthy => {
                format!("The Daemon of major {major} is ready to take a run")
            }
            (Some(daemon), HealthProbe::Ready) => format!(
                "The Daemon of major {major} is not ready to take a run: {}",
                daemon
                    .unready
                    .iter()
                    .map(|reason| match reason {
                        Unready::Draining => "it drains",
                        Unready::RunStateNotWritable => "its run state cannot be written",
                        Unready::BrowsersMissing => "a browser it uses is not installed",
                        Unready::GovernorLastResort => {
                            "its memory stayed critical at the governor's floor"
                        }
                        Unready::StorageCritical => {
                            "the filesystem holding its run state is at the critical storage level"
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if healthy {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn describe(status: &DaemonState) -> String {
    let idle = if status.kept_alive {
        "kept alive".to_owned()
    } else {
        format!(
            "stops after {} s with no Client",
            status.idle_timeout / 1000
        )
    };
    let console = match &status.console {
        ConsoleState::Serving(url) => format!("its Console at {url}"),
        ConsoleState::Failed(failure) => format!("no Console ({})", failure.code),
        ConsoleState::Absent => "no Console".to_owned(),
    };
    format!(
        "{} {} (PID {}) at {}, {} Clients, {} live Runs, {idle}, {console}",
        status.product.name(),
        status.product_version,
        status.pid,
        status.endpoint,
        status.clients,
        status.live_runs,
    )
}

/// Wall time now on `clock`.
pub(super) fn now_of(clock: Clock) -> jiff::Timestamp {
    clock.timestamp()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its thread")
}

fn multi_thread_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its threads")
}
