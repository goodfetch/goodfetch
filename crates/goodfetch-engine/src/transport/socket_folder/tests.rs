//! The engine's own sockets bind under a temporary folder whose path is
//! too long for the OS's socket paths, as a deep `TMPDIR` makes it.
#![expect(
    clippy::disallowed_methods,
    reason = "the test makes its temporary folder itself"
)]

use std::path::PathBuf;

use goodfetch_test_support::TestEnv;

use super::socket_folder_in;
use crate::transport::{Network, listen_wakes};

/// A temporary folder whose path alone passes the OS's limit on a socket's
/// path once a folder and a socket are added below it.
fn long_temporary(env: &TestEnv) -> PathBuf {
    let temporary = env
        .folder()
        .join("a temporary folder as deep as a TMPDIR set by a build tool or a sandbox can be");
    std::fs::create_dir_all(&temporary).unwrap();
    temporary
}

/// The Daemon's socket, an Attached engine's private endpoint and the
/// Politeness file's wake endpoint each listen in a folder made under a
/// long temporary folder.
#[cfg(unix)]
#[test]
fn every_engine_socket_listens_in_a_folder_made_under_a_long_temporary_folder() {
    let env = TestEnv::new();
    let temporary = long_temporary(&env);

    let listened = goodfetch_test_support::runtime().block_on(async {
        let daemon = socket_folder_in(&temporary, "goodfetch-daemon-", "daemon.sock").unwrap();
        let engine = socket_folder_in(&temporary, "goodfetch-engine-", "engine.sock").unwrap();
        let wake = socket_folder_in(&temporary, "goodfetch-wake-", "wake.sock").unwrap();
        (
            Network::Os
                .listen(&daemon.path().join("daemon.sock"))
                .await
                .map(drop)
                .map_err(|error| error.to_string()),
            Network::Os
                .listen(&engine.path().join("engine.sock"))
                .await
                .map(drop)
                .map_err(|error| error.to_string()),
            listen_wakes(&wake.path().join("wake.sock"))
                .await
                .map(drop)
                .map_err(|error| error.to_string()),
        )
    });

    assert_eq!(listened, (Ok(()), Ok(()), Ok(())));
}
