//! An abort forwarded to the engine that owns its Run (ADR 0031), at
//! `ToOwner`'s interface, on the paused `Clock`. The owner is a stand-in on
//! a Unix socket in the test's folder, which answers `hello` and never the
//! call. While the stand-in reads, the paused clock is held still
//! (TESTING.md, Time): no timer fires during the real socket's waits, and
//! once it has the call, the forwarder's own deadline is the only timer
//! left before the test's.
#![expect(
    clippy::disallowed_methods,
    reason = "the test runs on the test Clock (ADR 0059), holds the paused clock with a blocking task (TESTING.md, Time) and writes the owner's token file"
)]

use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::connection::{PrivateEndpoint, ProductVersion};
use goodfetch_protocol::run::{RunId, RunOwner};
use goodfetch_test_support::TestEnv;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::ToOwner;
use crate::clock::Clock;
use crate::home::Home;
use crate::runs::Forwarder as _;
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::Network;

/// Longer than any deadline the forwarder keeps.
const GIVEN_UP: Duration = Duration::from_hours(1);

/// An owner that accepts `hello` and never answers the forwarded abort
/// leaves the abort to the writer lock: the forwarder gives up on it.
#[test]
fn a_forwarded_abort_the_owner_never_answers_ends_without_its_report() {
    let env = TestEnv::new();

    let (forwarded, call) = goodfetch_test_support::runtime().block_on(async {
        let (mut listener, address) = Network::Os
            .listen(&env.folder().join("owner.sock"))
            .await
            .unwrap();
        let token_file = env.folder().join("owner.token");
        std::fs::write(&token_file, "the-owners-token").unwrap();
        let owner = RunOwner::Attached(PrivateEndpoint {
            address,
            token_file,
        });
        let forwarder = ToOwner {
            home: Home::new(env.home().to_owned()),
            version: ProductVersion::parse("0.0.0").unwrap(),
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
        };
        let tasks = TaskOwner::root(BlockingPolicy::Inline);
        let run_id = RunId::parse("elsewhere").unwrap();
        let (release, released) = mpsc::channel::<()>();
        // Holds the paused clock still until the stand-in has the call.
        tokio::task::spawn_blocking(move || released.recv().is_err());
        let owning = async {
            let (reader, mut writer) = listener.accept().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            lines.next_line().await.unwrap().unwrap();
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}\n")
                .await
                .unwrap();
            let call = lines.next_line().await.unwrap().unwrap();
            drop(release);
            // The Connection stays open, unanswered, until the test ends.
            (serde_json::from_str::<Value>(&call).unwrap(), lines, writer)
        };
        let aborting = tokio::time::timeout(GIVEN_UP, forwarder.abort(&owner, &run_id, &tasks));
        let ((call, _lines, _writer), forwarded) = tokio::join!(owning, aborting);
        (forwarded, call["method"].clone())
    });

    assert_eq!(
        (forwarded.ok(), call),
        (Some(None), Value::from("run.abort"))
    );
}
