//! The Daemon from both Products' executables (ADRs 0024, 0030 and 0098):
//! `daemon start|stop|restart|status`, the hidden `daemon launched` that a
//! start runs from the pick's versioned copy, and the Client side every
//! command that reaches the Daemon shares.
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
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI reads the machine's files, starts the Daemon's process and builds its runtime before and outside any async worker (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{
    self, BROWSERS_PATH_VARIABLE, CacheLocation, HOME_FOLDER, MaxUnusedAge,
};
use goodfetch_engine::daemon::files::{DaemonFiles, FileLock, new_token};
use goodfetch_engine::daemon::{
    DaemonEnd, DaemonOptions, Holds, RefusedClient, RefusedExecutable, Succession, refusal_fix,
};
use goodfetch_engine::home::{Home, major_name};
use goodfetch_engine::installed::{InstalledList, Seen, fingerprint, pick};
use goodfetch_engine::transport::{self, Connection, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, MAX_UNUSED_AGE};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    HelloParams, NOT_IN_PRODUCT, PROTOCOL_VERSION_UNSUPPORTED, Surface,
};
use goodfetch_protocol::daemon::{
    DaemonRuntimeFile, DaemonStartOutput, DaemonStatus, DaemonStatusOutput, DaemonStopOutput,
    DaemonStopping, IDLE_TIMEOUT,
};
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable};
use goodfetch_protocol::jsonrpc::{ErrorObject, Message, Request, RequestId, Version};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, Lines};

use super::{browsers, output};

/// How many times a Client looks for the Daemon again when the one it
/// found went away, or handed over, before it gives up.
const ATTEMPTS: usize = 3;

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
}

/// Why reaching the Daemon failed.
pub(super) enum Failure {
    /// The Daemon refused this Client, with an Error code.
    Refused {
        code: String,
        message: String,
        fix: String,
    },
    /// The Daemon cannot be started or reached.
    Unavailable(String),
}

impl Failure {
    /// Logs it and answers its exit code.
    pub(super) fn report(&self) -> ExitCode {
        match self {
            Self::Refused { code, message, fix } => {
                tracing::error!(code = code.as_str(), "{message} {fix}");
                let exit = [&NOT_IN_PRODUCT, &PROTOCOL_VERSION_UNSUPPORTED]
                    .into_iter()
                    .find(|definition| definition.name == code)
                    .and_then(|definition| definition.exit_code)
                    .unwrap_or(1);
                ExitCode::from(exit)
            }
            Self::Unavailable(problem) => {
                tracing::error!("{problem}");
                ExitCode::FAILURE
            }
        }
    }
}

fn unavailable(what: &str) -> impl Fn(std::io::Error) -> Failure + '_ {
    move |error| Failure::Unavailable(format!("{what}: {error}"))
}

impl Machine<'_> {
    fn files(&self) -> DaemonFiles {
        DaemonFiles::of(self.home, &self.identity.product_version)
    }

    fn major(&self) -> String {
        major_name(&self.identity.product_version)
    }

    fn list(&self) -> InstalledList {
        InstalledList::of(self.home)
    }

    /// The Browser cache the Daemon's copies go in: the one this executable
    /// uses, or `browsers` in Goodfetch home when that one cannot be
    /// written (ADR 0098's note under #259).
    fn cache(&self) -> PathBuf {
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
        .map(|settings| {
            MaxUnusedAge::new(
                settings
                    .value(MAX_UNUSED_AGE.name)
                    .as_u64()
                    .map(Duration::from_millis),
            )
        })
        .unwrap_or_default()
    }

    /// The major's pick now.
    fn pick(&self) -> Option<InstalledExecutable> {
        let entries = self.list().read().unwrap_or_default();
        let cache = self.cache();
        let seen: Vec<Seen<'_>> = entries
            .iter()
            .map(|entry| Seen::look(entry, &cache))
            .collect();
        pick(
            &seen,
            &self.identity.product_version,
            None,
            now(),
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

    /// The running Daemon's runtime file, or one this starts: the pick,
    /// under the start lock. `true` when this started it.
    fn running_or_started(&self, keep_alive: bool) -> Result<(DaemonRuntimeFile, bool), Failure> {
        let files = self.files();
        let start = files
            .start_lock()
            .map_err(unavailable("the Daemon's start lock cannot be taken"))?;
        if let Some(found) = files
            .running()
            .map_err(unavailable("the Daemon's runtime file cannot be read"))?
        {
            return Ok((found, false));
        }
        let started = self.start_pick(keep_alive)?;
        drop(start);
        Ok((started, true))
    }

    /// Starts the major's pick from its versioned copy and waits for its
    /// runtime file; whoever calls it holds the start lock.
    fn start_pick(&self, keep_alive: bool) -> Result<DaemonRuntimeFile, Failure> {
        let Some(picked) = self.pick() else {
            return Err(Failure::Unavailable(format!(
                "no installed executable can run the Daemon of major {}: install {} {} or newer",
                self.major(),
                self.identity.product.name(),
                self.identity.product_version,
            )));
        };
        let cache = self.cache();
        let copy = match &picked.copy {
            Some(copy) if browser_cache::entry_present(&cache, copy) => copy.clone(),
            _ => {
                let placed = browser_cache::place_executable_copy(
                    &cache,
                    Path::new(&picked.path),
                    picked.product,
                    &picked.product_version,
                    jiff_now(),
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
        launch(
            &executable,
            &copy.build,
            keep_alive,
            self.environment,
            self.home,
        )
    }

    /// A Connection to the Daemon that `hello` opened, this executable's
    /// Product needed: the running Daemon, or the pick this starts when
    /// `start` and none runs.
    pub(super) fn connect(&self, start: bool) -> Result<Option<Connected>, Failure> {
        let runtime = runtime();
        for _ in 0..ATTEMPTS {
            let found = if start {
                Some(self.running_or_started(false)?.0)
            } else {
                self.files()
                    .running()
                    .map_err(unavailable("the Daemon's runtime file cannot be read"))?
            };
            let Some(found) = found else {
                return Ok(None);
            };
            match runtime.block_on(self.open(&found)) {
                Ok(connection) => {
                    return Ok(Some(Connected {
                        runtime,
                        connection,
                        found,
                        next: 1,
                    }));
                }
                // It handed over, or went away in between: look again.
                Err(Opening::HandOver | Opening::Gone) => {}
                Err(Opening::Refused(error)) => return Err(self.refused(&error)),
            }
        }
        Err(Failure::Unavailable(format!(
            "the Daemon of major {} went away {ATTEMPTS} times while this Client connected",
            self.major()
        )))
    }

    async fn open(&self, found: &DaemonRuntimeFile) -> Result<Opened, Opening> {
        let connection = transport::connect_daemon(&found.endpoint)
            .await
            .map_err(|_| Opening::Gone)?;
        let mut opened = Opened::new(connection);
        let hello = HelloParams {
            protocol_version: PROTOCOL_VERSION,
            surface: Surface::Cli,
            token: Some(found.token.clone()),
            needs: Some(self.identity.product),
            product_version: self.identity.product_version.clone(),
        };
        match opened.call(0, "hello", &hello).await {
            Ok(_) => Ok(opened),
            Err(None) => Err(Opening::Gone),
            Err(Some(error)) => {
                let handover = error
                    .data
                    .details
                    .get("handover")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if handover {
                    Err(Opening::HandOver)
                } else {
                    Err(Opening::Refused(error))
                }
            }
        }
    }

    /// The refusal of this Client, its `fix` adding this executable's
    /// `daemon restart` (ADR 0098).
    fn refused(&self, error: &ErrorObject) -> Failure {
        let details = &error.data.details;
        let given = details
            .get("fix")
            .or_else(|| details.get("hint"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let executable = self.executable.clone().unwrap_or_default();
        let ours = refusal_fix(&RefusedClient {
            product: self.identity.product,
            version: &self.identity.product_version,
            executable: RefusedExecutable::Path(&executable),
            channel: self.identity.channel,
            would_be_pick: self.is_the_pick(),
        });
        Failure::Refused {
            code: error.data.code.clone(),
            message: error.message.clone(),
            fix: format!("{given} {ours}").trim().to_owned(),
        }
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
                last_seen: jiff_now(),
            })
        });
        if let Err(error) = recorded {
            tracing::warn!(%error, "this executable is not recorded in the list of installed executables");
        }
    }

    /// Asks the running Daemon to stop and waits until it has; `None` when
    /// none runs.
    fn stop(&self) -> Result<Option<DaemonStopping>, Failure> {
        let Some(mut connected) = self.connect(false)? else {
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

    /// Starts the Daemon unless it runs, and answers its status.
    fn start(&self, keep_alive: bool) -> Result<DaemonStartOutput, Failure> {
        let (_, started) = self.running_or_started(keep_alive)?;
        let mut connected = self.connect(true)?.ok_or_else(|| {
            Failure::Unavailable("the Daemon stopped as soon as it started".to_owned())
        })?;
        let daemon: DaemonStatus = connected.call("daemon.status", &serde_json::json!({}))?;
        connected.close();
        Ok(DaemonStartOutput { started, daemon })
    }
}

/// Why opening a Connection to a found Daemon failed.
enum Opening {
    /// It refused this Client and hands over to its major's pick.
    HandOver,
    /// It went away.
    Gone,
    /// It refused this Client.
    Refused(ErrorObject),
}

/// A Client's open Connection to the Daemon.
pub(super) struct Connected {
    runtime: tokio::runtime::Runtime,
    connection: Opened,
    found: DaemonRuntimeFile,
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
        let answered = self
            .runtime
            .block_on(self.connection.call(id, method, params));
        match answered {
            Ok(result) => serde_json::from_value(result).map_err(|error| {
                Failure::Unavailable(format!(
                    "the Daemon answered `{method}` with no result of it: {error}"
                ))
            }),
            Err(None) => Err(Failure::Unavailable(format!(
                "the Daemon at {} closed the Connection before it answered `{method}`",
                self.found.endpoint
            ))),
            Err(Some(error)) => Err(Failure::Refused {
                code: error.data.code.clone(),
                message: error.message,
                fix: error
                    .data
                    .details
                    .get("fix")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
        }
    }

    /// Closes the Connection.
    pub(super) fn close(mut self) {
        let _ = self.runtime.block_on(self.connection.writer.shutdown());
    }
}

/// A Connection's two ends, read line by line.
struct Opened {
    lines: Lines<tokio::io::BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl Opened {
    fn new(connection: Connection) -> Self {
        let (reader, writer) = connection.split();
        Self {
            lines: tokio::io::BufReader::new(reader).lines(),
            writer,
        }
    }

    /// Sends one call and reads its reply: its result, its error, or `None`
    /// once the Daemon closed the Connection.
    async fn call(
        &mut self,
        id: i64,
        method: &str,
        params: &impl Serialize,
    ) -> Result<Value, Option<ErrorObject>> {
        let request = Request {
            jsonrpc: Version,
            id: RequestId::Number(id),
            method: method.to_owned(),
            params: Some(serde_json::to_value(params).expect("invariant: params encode")),
        };
        let mut line = serde_json::to_vec(&request).expect("invariant: a request encodes");
        line.push(b'\n');
        if self.writer.write_all(&line).await.is_err() || self.writer.flush().await.is_err() {
            return Err(None);
        }
        loop {
            let Ok(Some(line)) = self.lines.next_line().await else {
                return Err(None);
            };
            match serde_json::from_str::<Message>(&line) {
                Ok(Message::Response(response)) if response.id == RequestId::Number(id) => {
                    return Ok(response.result);
                }
                Ok(Message::Error(failure)) if failure.id == Some(RequestId::Number(id)) => {
                    return Err(Some(failure.error));
                }
                // Events and others' replies are no answer to this call.
                _ => {}
            }
        }
    }
}

/// Starts the Daemon's process from `executable`, the versioned copy of
/// build `build`, and waits for the runtime file it prints once ready.
fn launch(
    executable: &Path,
    build: &str,
    keep_alive: bool,
    environment: &BTreeMap<OsString, OsString>,
    home: &Home,
) -> Result<DaemonRuntimeFile, Failure> {
    std::fs::create_dir_all(home.root()).map_err(unavailable("Goodfetch home cannot be made"))?;
    let mut command = std::process::Command::new(executable);
    command
        .args(["daemon", "launched", "--copy", build])
        .args(keep_alive.then_some("--keep-alive"))
        .env_clear()
        .envs(environment)
        .current_dir(home.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    detach(&mut command);
    let mut child = command
        .spawn()
        .map_err(unavailable("the Daemon's process cannot start"))?;
    let stdout = child
        .stdout
        .take()
        .expect("invariant: the Daemon's stdout is piped");
    let mut ready = String::new();
    let read = BufReader::new(stdout).read_line(&mut ready);
    // The Daemon outlives this process: it is never waited for here.
    drop(child);
    match read {
        Ok(read) if read > 0 => serde_json::from_str(&ready).map_err(|error| {
            Failure::Unavailable(format!(
                "the Daemon printed no runtime file once ready: {error}"
            ))
        }),
        _ => Err(Failure::Unavailable(format!(
            "the Daemon exited before it was ready; its log is in {}",
            home.root().join("logs").display()
        ))),
    }
}

/// Starts the Daemon's process outside this process's group, so a signal
/// to this command's terminal never reaches it.
#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut std::process::Command) {}

/// The Daemon's process, started by [`launch`] from the copy of build
/// `copy`: waits for the serving lock, listens, publishes its runtime file
/// and prints it, serves until it stops, and starts the pick when it hands
/// over.
pub(super) fn launched(
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    copy: Option<&str>,
    keep_alive: bool,
) -> ExitCode {
    let files = machine.files();
    let identity = engine.identity();
    let _copy = copy.and_then(|build| {
        let id = ArtifactId {
            kind: ArtifactKind::ExecutableCopy,
            name: format!("goodfetch-{}", identity.product.as_str()),
            build: build.to_owned(),
        };
        browser_cache::hold_copy(&machine.cache(), &id)
            .ok()
            .flatten()
    });
    let serving = match files.serving_lock() {
        Ok(serving) => serving,
        Err(error) => {
            return unavailable("the Daemon's serving lock cannot be taken")(error).report();
        }
    };
    let idle_timeout =
        browsers::machine_settings(machine.home, identity.product, &identity.product_version)
            .ok()
            .and_then(|settings| settings.value(IDLE_TIMEOUT.name).as_u64())
            .map_or(Duration::from_secs(600), Duration::from_millis);
    let (socket, _socket_folder) = match files.socket_place() {
        Ok(place) => place,
        Err(error) => return unavailable("the Daemon's socket has no folder")(error).report(),
    };
    let token = match new_token() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "the OS generator gave no token");
            return ExitCode::FAILURE;
        }
    };
    let pid = std::process::id();
    let runtime = multi_thread_runtime();
    let ended = runtime.block_on(async {
        let (listener, endpoint) = Network::Os
            .listen(&socket)
            .await
            .map_err(unavailable("the Daemon cannot listen"))?;
        let published = DaemonRuntimeFile {
            endpoint: endpoint.clone(),
            token: token.clone(),
            version: identity.product_version.clone(),
            pid,
        };
        files
            .publish(&published)
            .map_err(unavailable("the Daemon's runtime file cannot be written"))?;
        output::json(&published);
        tracing::info!(%endpoint, pid, "the Daemon serves");
        let options = DaemonOptions {
            token,
            endpoint,
            pid,
            idle_timeout,
            keep_alive,
            holds: Holds::new(),
            succession: Arc::new(OnTheMachine {
                files: files.clone(),
                home: machine.home.clone(),
                cache: machine.cache(),
                product: identity.product,
                version: identity.product_version.clone(),
                max_unused_age: machine.max_unused_age(),
                pid,
            }),
        };
        Ok::<_, Failure>(engine.run_daemon(listener, dependencies, options).await)
    });
    runtime.shutdown_background();
    let ended = match ended {
        Ok(ended) => ended,
        Err(failure) => return failure.report(),
    };
    if let Err(error) = files.unpublish(pid) {
        tracing::warn!(%error, "the Daemon's runtime file cannot be removed");
    }
    drop(serving);
    match ended {
        DaemonEnd::HandedOver(start_lock) => {
            let started = machine.start_pick(false);
            drop(start_lock);
            match started {
                Ok(_) => ExitCode::SUCCESS,
                Err(failure) => failure.report(),
            }
        }
        DaemonEnd::ListenerFailed => ExitCode::FAILURE,
        DaemonEnd::Idle | DaemonEnd::Stopped => ExitCode::SUCCESS,
    }
}

/// The pick and the handover on this machine (ADR 0098).
struct OnTheMachine {
    files: DaemonFiles,
    home: Home,
    cache: PathBuf,
    product: Product,
    version: goodfetch_protocol::connection::ProductVersion,
    max_unused_age: MaxUnusedAge,
    pid: u32,
}

impl Succession for OnTheMachine {
    fn is_pick(&self) -> bool {
        let entries = InstalledList::of(&self.home).read().unwrap_or_default();
        let seen: Vec<Seen<'_>> = entries
            .iter()
            .map(|entry| Seen::look(entry, &self.cache))
            .collect();
        pick(&seen, &self.version, None, now(), self.max_unused_age).is_none_or(|picked| {
            picked.product == self.product && picked.product_version == self.version
        })
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        let lock: Option<FileLock> = match self.files.start_lock() {
            Ok(lock) => Some(lock),
            Err(error) => {
                tracing::warn!(%error, "the Daemon hands over without the start lock");
                None
            }
        };
        if let Err(error) = self.files.unpublish(self.pid) {
            tracing::warn!(%error, "the Daemon's runtime file cannot be removed");
        }
        tracing::info!("the Daemon hands over to its major's pick");
        Box::new(lock)
    }
}

/// `daemon start`.
pub(super) fn start(machine: &Machine<'_>, keep_alive: bool, json: bool) -> ExitCode {
    match machine.start(keep_alive) {
        Ok(started) => {
            if json {
                output::json(&started);
            } else {
                let verb = if started.started {
                    "started"
                } else {
                    "already ran"
                };
                output::line(&format!(
                    "The Daemon of major {} {verb}: {}",
                    machine.major(),
                    describe(&started.daemon)
                ));
            }
            ExitCode::SUCCESS
        }
        Err(failure) => failure.report(),
    }
}

/// `daemon stop`.
pub(super) fn stop(machine: &Machine<'_>, json: bool) -> ExitCode {
    match machine.stop() {
        Ok(stopped) => {
            report_stop(machine, json, stopped.as_ref());
            ExitCode::SUCCESS
        }
        Err(failure) => failure.report(),
    }
}

fn report_stop(machine: &Machine<'_>, json: bool, stopped: Option<&DaemonStopping>) {
    if json {
        output::json(&DaemonStopOutput {
            stopped: stopped.cloned(),
        });
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
pub(super) fn restart(machine: &Machine<'_>, keep_alive: bool, json: bool) -> ExitCode {
    if let Err(failure) = machine.stop() {
        return failure.report();
    }
    start(machine, keep_alive, json)
}

/// `daemon status`.
pub(super) fn status(machine: &Machine<'_>, json: bool) -> ExitCode {
    let daemon = match machine.connect(false) {
        Ok(Some(mut connected)) => {
            let status = connected.call::<DaemonStatus>("daemon.status", &serde_json::json!({}));
            connected.close();
            match status {
                Ok(status) => Some(status),
                Err(failure) => return failure.report(),
            }
        }
        Ok(None) => None,
        Err(failure) => return failure.report(),
    };
    if json {
        output::json(&DaemonStatusOutput {
            runtime_file: machine.files().runtime_file().display().to_string(),
            daemon,
        });
    } else {
        match &daemon {
            Some(status) => output::line(&format!(
                "The Daemon of major {}: {}",
                machine.major(),
                describe(status)
            )),
            None => output::line(&format!("No Daemon of major {} runs", machine.major())),
        }
    }
    ExitCode::SUCCESS
}

fn describe(status: &DaemonStatus) -> String {
    let idle = if status.kept_alive {
        "kept alive".to_owned()
    } else {
        format!(
            "stops after {} s with no Client",
            status.idle_timeout / 1000
        )
    };
    format!(
        "{} {} (PID {}) at {}, {} Clients, {} live Runs, {idle}",
        status.product.name(),
        status.product_version,
        status.pid,
        status.endpoint,
        status.clients,
        status.live_runs,
    )
}

fn now() -> jiff::Timestamp {
    jiff_now()
}

fn jiff_now() -> jiff::Timestamp {
    let now = std::time::SystemTime::now();
    jiff::Timestamp::try_from(now).expect("invariant: the system clock reads a representable time")
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
