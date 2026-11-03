//! What the Kennel's tests share: a runtime and its tracker, a channel that
//! collects reports, and the Scripted engine as a Daemon of a test's home.
#![expect(
    clippy::disallowed_methods,
    reason = "test helpers build the runtime, read the test tool manifest's variable and start the Scripted engine (ADRs 0060 and 0069)"
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use goodfetch_kennel::home::Home;
use goodfetch_kennel::tasks::Tasks;
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// How long a test waits for a report before it fails.
pub(crate) const PATIENCE: Duration = Duration::from_secs(20);

/// The variable naming the test tool manifest (`just test`, ADR 0062).
const TOOLS_VARIABLE: &str = "GOODFETCH_TEST_TOOLS";

/// A multi-thread runtime with one worker, as the Kennel's `main` builds.
pub(crate) fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

/// A tracker on `runtime`, and a channel collecting what a module
/// reports.
pub(crate) fn reports<T: Send + 'static>(
    runtime: &Runtime,
) -> (Tasks, impl Fn(T) + Send + Sync + 'static, mpsc::Receiver<T>) {
    let tasks = Tasks::new(runtime.handle().clone());
    let (sender, receiver) = mpsc::channel(16);
    let report = move |value| {
        let _ = sender.try_send(value);
    };
    (tasks, report, receiver)
}

/// The next report, or a failure after [`PATIENCE`].
pub(crate) async fn next<T>(receiver: &mut mpsc::Receiver<T>) -> T {
    tokio::time::timeout(PATIENCE, receiver.recv())
        .await
        .expect("a report within the test's patience")
        .expect("the reporting task is still running")
}

/// A script of this crate's `tests/scripts/`.
pub(crate) fn script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(name)
}

#[derive(Deserialize)]
struct Tools {
    scripted_engine: Option<PathBuf>,
    collie: Option<PathBuf>,
}

/// The Scripted engine the root workspace built, from the test tool
/// manifest. Without a manifest, as when the Kennel's tests run on a Mac
/// that has not built the root workspace, the caller skips with the reason
/// printed; in CI, and in `just kennel-check`, which writes the manifest,
/// its absence fails.
pub(crate) fn scripted_engine(name: &str) -> Option<PathBuf> {
    Some(
        tools(name)?
            .scripted_engine
            .expect("the test tool manifest names goodfetch-scripted-engine"),
    )
}

/// Collie's executable the root workspace built, from the test tool
/// manifest, for the test against a real Daemon; skipped, or failed, as
/// [`scripted_engine`] says.
pub(crate) fn collie(name: &str) -> Option<PathBuf> {
    Some(
        tools(name)?
            .collie
            .expect("the test tool manifest names goodfetch-collie"),
    )
}

/// The test tool manifest, or `None` after saying the test `name` is
/// skipped.
fn tools(name: &str) -> Option<Tools> {
    let Some(manifest) = std::env::var_os(TOOLS_VARIABLE) else {
        assert!(
            std::env::var_os("CI").is_none(),
            "{TOOLS_VARIABLE} is unset, and CI runs every test: run the Kennel's tests with `just kennel-check`"
        );
        #[expect(
            clippy::print_stderr,
            reason = "a test skipped on this host says why, as the suites' skips do"
        )]
        {
            eprintln!(
                "skipped: {name}, {TOOLS_VARIABLE} is unset, so the root workspace built no test tool (`just kennel-check` builds them)"
            );
        }
        return None;
    };
    let text = std::fs::read_to_string(&manifest).unwrap();
    Some(serde_json::from_str(&text).unwrap())
}

/// The Scripted engine playing a script as the Daemon of a home.
pub(crate) struct ScriptedDaemon {
    child: Child,
}

impl ScriptedDaemon {
    /// Starts `engine` on `script` as the Daemon of `home`, with `token`,
    /// and returns once a Client can find it.
    pub(crate) fn start(engine: &Path, home: &Home, script: &Path, token: &str) -> Self {
        Self::start_as(engine, home, script, token, "0.0.0")
    }

    /// Starts it as [`ScriptedDaemon::start`] does, as a Daemon of
    /// `product_version`, which names its per-major folder.
    pub(crate) fn start_as(
        engine: &Path,
        home: &Home,
        script: &Path,
        token: &str,
        product_version: &str,
    ) -> Self {
        let mut child = Command::new(engine)
            .arg("--script")
            .arg(script)
            .args(["--mode", "socket", "--token", token])
            .args(["--product-version", product_version])
            .env("GOODFETCH_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut ready = String::new();
        BufReader::new(stdout).read_line(&mut ready).unwrap();
        assert!(
            ready.contains("address"),
            "the Scripted engine announced itself: {ready:?}"
        );
        Self { child }
    }

    /// Waits for the play to end: success when every step was played. A
    /// play still going after [`PATIENCE`] fails the test, which would
    /// otherwise wait for a Client that never comes.
    pub(crate) fn ended(mut self) -> ExitStatus {
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "the Scripted engine's play ended within the test's patience"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ScriptedDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
