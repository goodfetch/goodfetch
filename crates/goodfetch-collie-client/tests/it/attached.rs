//! The Attached transport's child process (spec #606): a Connection that
//! goes, dropped while it opens here, kills the Attached engine it started
//! (`kill_on_drop`), so no engine outlives its program's Connection; and an
//! engine that ends before its Connection is closed fails the call waiting
//! with `engine_exited`, carrying how it ended and the tail of its stderr,
//! each line of which reached the program's subscriber at warning (ADR
//! 0078).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_collie_client::logging::CLIENT;
use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::EngineExitedDetails;
use goodfetch_test_support::{CapturedLog, TestEnv, capturing};
use tracing::Level;

use crate::connection::hello;
use crate::session_events::scripted_engine;

/// How long the test waits for the engine to start, then to go.
const PATIENCE: Duration = Duration::from_secs(30);

/// An Attached engine that writes its process ID to `PIDFILE`, then
/// answers nothing until it is killed.
const SILENT_ENGINE: &str = "#!/bin/sh\necho $$ > \"$PIDFILE\"\nexec sleep 600\n";

/// Whether the process `pid` is gone, or a zombie its parent has not
/// reaped yet: either way it no longer runs.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the process table on its own thread, never an async worker"
)]
fn gone(pid: &str) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    state.trim().is_empty() || state.trim_start().starts_with('Z')
}

/// The process ID `path` holds once the engine wrote it whole.
#[expect(
    clippy::disallowed_methods,
    reason = "a few bytes read on the test's own current-thread runtime, which nothing else waits on"
)]
fn written(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.ends_with('\n').then(|| text.trim().to_owned())
}

/// Dropping a Connection, here while it waits for the engine's `hello`,
/// kills the Attached engine it started.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the engine's script and reads its process ID on its own thread"
)]
fn a_dropped_connection_kills_the_attached_engine_it_started() {
    use std::os::unix::fs::PermissionsExt as _;

    let env = TestEnv::new();
    let executable = env.home().join("silent-engine.sh");
    std::fs::write(&executable, SILENT_ENGINE).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pidfile = env.home().join("engine.pid");
    let engine = Endpoint::Attached(AttachedEngine {
        executable,
        environment: [
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("PIDFILE".into(), pidfile.clone().into_os_string()),
        ]
        .into(),
    });
    let one = std::num::NonZeroUsize::MIN;
    let owned = goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();

    let pid = runtime.block_on(async {
        let opening =
            ClientConnection::open(ConnectionRuntime::owned(owned), &Network::Os, &engine, hello());
        let started = async {
            loop {
                if let Some(pid) = written(&pidfile) {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            opened = opening => panic!("the silent engine answered hello: {:?}", opened.map(|_| ())),
            pid = tokio::time::timeout(PATIENCE, started) => pid.unwrap(),
        }
    });
    let mut waited = Duration::ZERO;
    while !gone(&pid) && waited < PATIENCE {
        std::thread::sleep(Duration::from_millis(10));
        waited += Duration::from_millis(10);
    }

    let killed = gone(&pid);
    if !killed {
        // Leaves no engine behind the failure.
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid])
            .status();
    }
    assert!(killed, "the Attached engine {pid} still runs");
}

/// The line the crashing engine's script writes 40 times to its stderr,
/// 81,000 bytes in all with their newlines.
fn crash_line() -> String {
    format!("thread 'main' panicked: {}", "é".repeat(1000))
}

/// The lines `log` holds that the crate warned of under
/// `goodfetch::client` holding `written`, a line the Attached engine wrote
/// to its stderr.
fn stderr_warnings(log: &CapturedLog, written: &str) -> Vec<String> {
    String::from_utf8(log.written())
        .unwrap()
        .lines()
        .filter(|line| line.contains(&format!("WARN {CLIENT}:")) && line.contains(written))
        .map(str::to_owned)
        .collect()
}

/// What the call `call` on a Connection to `engine` fails with, opened
/// under a subscriber recording the crate's own target at warning, with
/// what that subscriber wrote. It records none of the engine's, so a start
/// tells the engine no new log level first, which the scripts do not
/// expect.
fn failed_under_a_subscriber<F>(
    engine: &Endpoint,
    call: impl FnOnce(ClientConnection) -> F,
) -> (Error, CapturedLog)
where
    F: Future<Output = Result<(), Error>>,
{
    let (subscriber, log) = capturing(&[(CLIENT, Level::WARN)]);
    let failed = tracing::subscriber::with_default(subscriber, || {
        goodfetch_test_support::real_time_runtime().block_on(async {
            let opened = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &Network::Os,
                engine,
                hello(),
            )
            .await;
            match opened {
                Ok((connection, _said)) => call(connection).await,
                Err(error) => Err(error),
            }
        })
    });
    (failed.unwrap_err(), log)
}

/// The `engine-exited-carries-its-evidence` client conformance case, played
/// by the Scripted engine as the Attached engine: it dies while a Run
/// starts, exit code 101, after writing 81,000 bytes of crash output to
/// its stderr. The start fails with `engine_exited`, not retryable, its
/// details the exit code, no signal, and the last 64 KiB of the stderr,
/// cut where a character starts, its message headed by the call and
/// holding none of the stderr; a start made after fails alike. Each of the
/// 40 lines reached the program's subscriber at warning. The crate writes to no stdout or stderr of its
/// own, which its lints forbid (`print_stdout`, `print_stderr`).
/// Covers: story:11-24, story:11-27, error:engine_exited
#[test]
fn an_attached_engine_that_dies_during_a_call_fails_it_with_engine_exited_and_its_evidence() {
    let env = TestEnv::new();
    let engine = scripted_engine(&env, "connect/exit-during-a-call.json");

    let (error, log) = failed_under_a_subscriber(&engine, |connection| async move {
        let started = connection.start().await.map(|_run| ());
        let later = connection.start().await.map(|_run| ());
        let evidence = |failed: &Result<(), Error>| {
            failed.as_ref().err().map(|error| {
                let details = error.details::<EngineExitedDetails>();
                (error.code(), details, error.message().to_owned())
            })
        };
        assert_eq!(evidence(&later), evidence(&started));
        started
    });

    let line = crash_line();
    let tail = format!("{}\n{}", "é".repeat(367), format!("{line}\n").repeat(32));
    assert_eq!(
        (error.code(), error.group(), error.retryable()),
        (ErrorCode::ENGINE_EXITED, ErrorGroup::Unavailable, false),
        "{error:?}"
    );
    assert_eq!(
        error.details::<EngineExitedDetails>(),
        Some(EngineExitedDetails {
            exit_code: Some(101),
            signal: None,
            stderr_tail: tail,
        })
    );
    assert_eq!(
        (error.operation(), error.message()),
        (
            Some("run.start"),
            "run.start: the engine ended before its Connection was closed, with exit code 101"
        )
    );
    assert_eq!(stderr_warnings(&log, &line).len(), 40);
}

/// An Attached engine a signal ends, here `SIGKILL` before it answers
/// `hello`, fails the connect with `engine_exited` naming the signal and
/// no exit code.
/// Covers: error:engine_exited
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the engine's script on its own thread"
)]
fn an_attached_engine_a_signal_ends_fails_the_call_with_engine_exited_naming_the_signal() {
    use std::os::unix::fs::PermissionsExt as _;

    let env = TestEnv::new();
    let executable: PathBuf = env.home().join("killed-engine.sh");
    std::fs::write(&executable, "#!/bin/sh\necho 'going' >&2\nkill -KILL $$\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let engine = Endpoint::Attached(AttachedEngine {
        executable,
        environment: [("PATH".into(), "/usr/bin:/bin".into())].into(),
    });

    let (error, _log) = failed_under_a_subscriber(&engine, |_connection| async { Ok(()) });

    assert_eq!(error.code(), ErrorCode::ENGINE_EXITED, "{error:?}");
    assert_eq!(
        error.details::<EngineExitedDetails>(),
        Some(EngineExitedDetails {
            exit_code: None,
            signal: Some("SIGKILL".to_owned()),
            stderr_tail: "going\n".to_owned(),
        })
    );
    assert_eq!(
        (error.operation(), error.message()),
        (
            Some("hello"),
            "hello: the engine ended before its Connection was closed, with SIGKILL"
        )
    );
}
