//! The `interrupt-reaches-only-the-host` client conformance case (ADRs 0078
//! and 0093; issue 3063): Ctrl+C, `SIGINT` sent to the host program's
//! process group, reaches the host alone. The SDK installs no handler, so
//! the host's own gets the signal and, as the SDK's docs advise, closes the
//! Run, which stops it with a Checkpoint, then the Connection. Linked, the
//! engine runs inside the host and outlives the signal, as the Run's resume
//! on a later Connection shows; attached, the Attached engine runs in a
//! process group of its own, so the Scripted engine playing it still gets
//! the Run's `run.abort` after the signal and plays its script to its end.
//! The host is an ignored test of this binary, started in a group of its
//! own as a terminal starts the program it runs. TypeScript's
//! `interrupt.test.ts` and Python's `test_forks_and_interrupts.py` are its
//! twins.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use goodfetch_collie::{Collie, ConnectOptions, EngineChoice, RunId, StartedAs};
use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime};
use goodfetch_protocol::run::Tags;
use goodfetch_test_support::{ATTACHED_RECORD, TestEnv, real_time_runtime};
use tokio::signal::unix::{SignalKind, signal};

use super::options;
use super::scripted::{attached_hello, client_methods, install_script};

/// The variable naming the file the host writes the signal its handler
/// got into, once it closed.
const SIGNAL_FILE: &str = "INTERRUPTED_HOST_SIGNAL_FILE";

/// The variable naming the file the host creates once its Run runs.
const RUNNING_FILE: &str = "INTERRUPTED_HOST_RUNNING_FILE";

/// The variable naming the Attached engine's executable.
const ENGINE_FILE: &str = "INTERRUPTED_HOST_ENGINE";

/// The Run the host starts.
const NIGHTLY: &str = "nightly";

/// How long the host may take to start its Run, then to exit once
/// interrupted.
const PATIENCE: Duration = Duration::from_secs(60);

/// What the interrupted host left behind.
#[derive(Debug, PartialEq, Eq)]
struct Interrupted {
    /// Whether it exited 0.
    succeeded: bool,
    /// The signal its handler got, as it wrote it.
    signal: String,
    /// What it wrote to stderr, which the SDK writes nothing to.
    stderr: String,
}

/// The `interrupt-reaches-only-the-host` case, linked: the host's handler
/// gets `SIGINT` and closes its Run, which a later Connection resumes, so
/// the engine inside the host outlived the signal and stopped the Run with
/// a Checkpoint.
/// Covers: story:11-22
#[test]
fn ctrl_c_reaches_the_host_alone_whose_handler_closes_the_run_linked() {
    let env = TestEnv::new();
    let nightly = RunId::parse(NIGHTLY).unwrap();

    let interrupted = interrupt(&env, "interrupted_host_linked", &[]);

    let resumed = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().run_id(&nightly).await.unwrap();
        let started_as = run.started_as();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        started_as
    });
    assert_eq!(interrupted, handled());
    assert_eq!(resumed, StartedAs::Resumed);
}

/// The `interrupt-reaches-only-the-host` case, attached: the host's handler
/// gets `SIGINT` and closes its Run, and the Attached engine, in a group of
/// its own, outlived the signal: the Scripted engine got the Run's close,
/// `run.abort`, after it, and played its script to its end.
/// Covers: story:11-22
#[test]
fn ctrl_c_reaches_the_host_alone_whose_handler_closes_the_run_attached() {
    let env = TestEnv::new();
    let engine = install_script(&env, "run/closed-while-running.json");

    let interrupted = interrupt(
        &env,
        "interrupted_host_attached",
        &[(ENGINE_FILE, engine.into_os_string())],
    );

    assert_eq!(interrupted, handled());
    assert_eq!(
        client_methods(&env.home().join(ATTACHED_RECORD)),
        ["hello", "run.start", "run.abort"]
    );
}

/// The host program, linked: it connects to the engine linked into it,
/// starts the Run `nightly` and waits for its own handler's signal, then
/// closes the Run and the Connection.
#[test]
#[ignore = "run as a child by sdk::interrupt::ctrl_c_reaches_the_host_alone_whose_handler_closes_the_run_linked"]
fn interrupted_host_linked() {
    let nightly = RunId::parse(NIGHTLY).unwrap();
    let environment = host_environment();
    let signal = real_time_runtime().block_on(async {
        let handler = Handler::install();
        let options = ConnectOptions::default()
            .engine(EngineChoice::Linked)
            .environment(environment.clone());
        let collie = Collie::connect(options).await.unwrap();
        let run = collie.start().run_id(&nightly).await.unwrap();

        let signal = handler.wait_while_running(&environment).await;
        run.close().await.unwrap();
        collie.close().await.unwrap();
        signal
    });
    handled_by_host(&environment, signal);
}

/// The host program, attached: it connects to the Attached engine its
/// environment names, starts the Run `nightly` and waits for its own
/// handler's signal, then closes the Run and the Connection.
#[test]
#[ignore = "run as a child by sdk::interrupt::ctrl_c_reaches_the_host_alone_whose_handler_closes_the_run_attached"]
fn interrupted_host_attached() {
    let nightly = RunId::parse(NIGHTLY).unwrap();
    let environment = host_environment();
    let engine = Endpoint::Attached(AttachedEngine {
        executable: PathBuf::from(&environment[&OsString::from(ENGINE_FILE)]),
        environment: environment.clone(),
    });
    let signal = real_time_runtime().block_on(async {
        let handler = Handler::install();
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &engine,
            attached_hello(),
        )
        .await
        .unwrap();
        let run = connection
            .start()
            .run_id(&nightly)
            .tags(Tags::parse(["shop"]).unwrap())
            .await
            .unwrap();

        let signal = handler.wait_while_running(&environment).await;
        run.close().await.unwrap();
        connection.close().await.unwrap();
        signal
    });
    handled_by_host(&environment, signal);
}

/// The host's own handlers of `SIGINT` and `SIGTERM`, as the SDK's docs
/// advise a program to install.
struct Handler {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

impl Handler {
    /// Installs both, before the host connects, so neither signal can end
    /// it unhandled.
    fn install() -> Self {
        Self {
            interrupt: signal(SignalKind::interrupt()).unwrap(),
            terminate: signal(SignalKind::terminate()).unwrap(),
        }
    }

    /// Says the Run runs, through the file its parent waits for, then
    /// waits for either signal and returns its name.
    #[expect(
        clippy::disallowed_methods,
        reason = "the host creates an empty file on its own current-thread runtime, which nothing else waits on"
    )]
    async fn wait_while_running(
        mut self,
        environment: &BTreeMap<OsString, OsString>,
    ) -> &'static str {
        std::fs::write(&environment[&OsString::from(RUNNING_FILE)], "").unwrap();
        tokio::select! {
            _ = self.interrupt.recv() => "SIGINT",
            _ = self.terminate.recv() => "SIGTERM",
        }
    }
}

/// Writes the signal the host's handler got, once it closed everything.
#[expect(
    clippy::disallowed_methods,
    reason = "the host writes its result after its runtime ended"
)]
fn handled_by_host(environment: &BTreeMap<OsString, OsString>, signal: &str) {
    std::fs::write(&environment[&OsString::from(SIGNAL_FILE)], signal).unwrap();
}

/// The host's environment, which its parent set: the test's own, which
/// names its Goodfetch home, and the files and engine the host uses.
#[expect(
    clippy::disallowed_methods,
    reason = "the host is a program of its own, whose parent passes it the test's environment"
)]
fn host_environment() -> BTreeMap<OsString, OsString> {
    std::env::vars_os().collect()
}

/// What a host whose handler got `SIGINT` leaves behind.
fn handled() -> Interrupted {
    Interrupted {
        succeeded: true,
        signal: "SIGINT".to_owned(),
        stderr: String::new(),
    }
}

/// Starts the host test `host` in a process group of its own, with
/// `variables` added to `env`'s environment, waits for its Run to run,
/// sends `SIGINT` to its whole group, as Ctrl+C at a terminal does, and
/// waits for it to exit.
#[expect(
    clippy::disallowed_methods,
    reason = "the test polls its host's files and signals its group on its own thread, outside any async worker"
)]
fn interrupt(env: &TestEnv, host: &str, variables: &[(&str, OsString)]) -> Interrupted {
    use std::os::unix::process::CommandExt as _;

    let signal_file = env.folder().join("signal");
    let running_file = env.folder().join("running");
    let mut command = env.process_command(std::env::current_exe().unwrap());
    command
        .args([&format!("sdk::interrupt::{host}"), "--exact", "--ignored"])
        .arg("--nocapture")
        .env(SIGNAL_FILE, &signal_file)
        .env(RUNNING_FILE, &running_file)
        .envs(variables.iter().map(|(name, value)| (name, value)))
        .stderr(Stdio::piped())
        .process_group(0);
    let process = env.spawn(&mut command).unwrap();
    let started = Instant::now();
    while !running_file.exists() {
        let ended = process.child().try_wait().unwrap();
        assert!(
            ended.is_none(),
            "the host ended before its Run ran: {ended:?}"
        );
        assert!(started.elapsed() < PATIENCE, "the host's Run never ran");
        std::thread::sleep(Duration::from_millis(10));
    }

    let group = format!("-{}", process.child().id());
    let sent = std::process::Command::new("kill")
        .args(["-INT", "--", &group])
        .status()
        .unwrap();
    assert!(sent.success(), "`kill -INT -- {group}` failed: {sent}");
    let status: Option<ExitStatus> = process.wait_within(PATIENCE).unwrap();

    let mut stderr = String::new();
    if let Some(mut pipe) = process.child().stderr.take() {
        pipe.read_to_string(&mut stderr).unwrap();
    }
    Interrupted {
        succeeded: status.is_some_and(|status| status.success()),
        signal: std::fs::read_to_string(&signal_file).unwrap_or_default(),
        stderr,
    }
}
