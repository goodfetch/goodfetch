//! The Daemon's token never reaches the Kennel's logs: every event the
//! Client logs at every level while it reads the runtime file, says
//! `hello` and loses the Daemon is captured, and none holds the token. A
//! binary of its own, since it installs the process's log subscriber.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "a test helper builds the runtime, reads the test tool manifest's variable and starts the Scripted engine (ADRs 0060 and 0069)"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_kennel::client::{self, ConnectionChange};
use goodfetch_kennel::home::Home;
use goodfetch_kennel::tasks::Tasks;
use tracing_subscriber::fmt::MakeWriter;

const TOKEN: &str = "kennel-test-token";

/// Each write of the log subscriber, sent to the test.
#[derive(Clone)]
struct Captured(mpsc::Sender<Vec<u8>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.send(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn scripted_engine() -> Option<PathBuf> {
    let Some(manifest) = std::env::var_os("GOODFETCH_TEST_TOOLS") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "GOODFETCH_TEST_TOOLS is unset, and CI runs every test: run the Kennel's tests with `just kennel-check`"
        );
        #[expect(
            clippy::print_stderr,
            reason = "a test skipped on this host says why, as the suites' skips do"
        )]
        {
            eprintln!(
                "skipped: the_daemons_token_never_reaches_the_logs, GOODFETCH_TEST_TOOLS is unset, so no goodfetch-scripted-engine was built (`just kennel-check` builds it)"
            );
        }
        return None;
    };
    let tools: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
    Some(PathBuf::from(tools["scripted_engine"].as_str().unwrap()))
}

#[test]
fn the_daemons_token_never_reaches_the_logs() {
    let Some(engine) = scripted_engine() else {
        return;
    };
    let (sender, written) = mpsc::channel();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(Captured(sender))
        .init();
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/hello-accepted-then-closed.json");
    let mut daemon = Command::new(engine)
        .arg("--script")
        .arg(script)
        .args(["--mode", "socket", "--token", TOKEN])
        .env("GOODFETCH_HOME", home.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    BufReader::new(daemon.stdout.take().unwrap())
        .read_line(&mut String::new())
        .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let tasks = Tasks::new(runtime.handle().clone());
    let (sender_of_changes, mut changes) = tokio::sync::mpsc::channel(4);
    let (_look_again, look_again) = tokio::sync::mpsc::channel(1);
    tasks.spawn(client::follow(
        tasks.clone(),
        home,
        look_again,
        move |change| {
            let _ = sender_of_changes.try_send(change);
        },
    ));

    let seen: Vec<ConnectionChange> = runtime.block_on(async {
        let mut seen = Vec::new();
        for _ in 0..2 {
            let change = tokio::time::timeout(Duration::from_secs(20), changes.recv());
            seen.push(change.await.unwrap().unwrap());
        }
        seen
    });
    assert!(daemon.wait().unwrap().success());
    let logs: Vec<u8> = written.try_iter().flatten().collect();
    let logs = String::from_utf8(logs).unwrap();

    assert!(
        matches!(seen[0], ConnectionChange::Connected(_)),
        "{seen:?}"
    );
    assert!(
        logs.contains("reached the Daemon"),
        "the Client logged: {logs}"
    );
    assert!(!logs.contains(TOKEN), "the token is in the logs: {logs}");
}
