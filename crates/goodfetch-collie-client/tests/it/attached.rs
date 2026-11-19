//! The Attached transport's child process (spec #606): a Connection that
//! goes, dropped while it opens here, kills the Attached engine it started
//! (`kill_on_drop`), so no engine outlives its program's Connection.

use std::path::Path;
use std::time::Duration;

use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime};
use goodfetch_test_support::TestEnv;

use crate::connection::hello;

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
