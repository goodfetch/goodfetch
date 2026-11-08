//! A call forwarded to the engine that owns its Run (ADR 0031), on an
//! in-memory Connection and the paused `Clock`: the owner is a stand-in that
//! answers `hello`, then the call once its delay has passed, if it ever does.
#![expect(
    clippy::disallowed_methods,
    reason = "the test runs on the test Clock (ADR 0059) and writes the machine's settings files"
)]

use std::time::{Duration, SystemTime};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::RUN_ABORT;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::{NoAnswer, ToOwner};
use crate::clock::Clock;
use crate::home::Home;
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::{self, Network};

/// Longer than any deadline the forwarder keeps.
const NEVER: Duration = Duration::from_hours(1);

/// A minute: past `timeout.close`'s default of 30 s.
const A_MINUTE: Duration = Duration::from_mins(1);

/// What the forwarder made of a call the owner answers after `delay`, on a
/// machine whose user configuration, if any, is `configuration`.
fn forwarded(configuration: Option<&str>, delay: Duration) -> Result<Value, NoAnswer> {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_owned());
    let version = ProductVersion::parse("0.0.0").unwrap();
    let files = SettingsFiles::of(&home, &version);
    if let Some(configuration) = configuration {
        std::fs::create_dir_all(files.user_configuration().parent().unwrap()).unwrap();
        std::fs::write(files.user_configuration(), configuration).unwrap();
    }
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let forwarder = ToOwner {
            home,
            version,
            clock,
            settings: Settings::new(
                files,
                Product::Collie,
                TaskOwner::root(BlockingPolicy::Inline),
            ),
        };
        let (network, dialer) = transport::in_memory();
        let connection = dialer.connect().await.unwrap();
        let owner = Network::InMemory(network)
            .attached_connection()
            .await
            .unwrap();
        let owning = async {
            let (reader, mut writer) = owner.split();
            let mut lines = BufReader::new(reader).lines();
            lines.next_line().await.unwrap().unwrap();
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}\n")
                .await
                .unwrap();
            lines.next_line().await.unwrap().unwrap();
            clock.sleep(delay).await;
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"answered\":true}}\n")
                .await
                .unwrap();
            // The Connection stays open until the forwarder lets go of it.
            std::future::pending::<()>().await;
        };
        let hello = forwarder.hello("the-owners-token".to_owned(), Product::Collie);
        let params = json!({});
        let calling = forwarder.exchange(connection, &hello, RUN_ABORT.name, &params);
        tokio::select! {
            () = owning => panic!("the stand-in ended"),
            answer = calling => answer.map(|answer| answer.unwrap()),
        }
    })
}

/// An owner that accepts `hello` and never answers leaves the abort to the
/// writer lock: the forwarder gives up on it.
#[test]
fn a_forwarded_call_the_owner_never_answers_ends_at_its_deadline() {
    let answer = forwarded(None, NEVER);

    assert!(
        matches!(&answer, Err(NoAnswer::CallUnanswered(method)) if method == RUN_ABORT.name),
        "{answer:?}"
    );
}

/// The machine's `timeout.close` set above its default gives a slow owner
/// that long.
#[test]
fn a_forwarded_call_waits_as_long_as_the_machines_timeout_close_gives_it() {
    let answer = forwarded(
        Some("version = \"0.1\"\n\n[settings.timeout]\nclose = 120000\n"),
        A_MINUTE,
    );

    assert_eq!(answer.unwrap(), json!({ "answered": true }));
}

/// Settings that cannot be read leave a call its default, rather than no
/// deadline or no call.
#[rstest]
#[case::default_of_no_settings(None)]
#[case::default_of_settings_that_cannot_be_read(Some("version = \"0.1\"\n\n[settings\n"))]
fn a_forwarded_call_waits_for_timeout_closes_default_otherwise(
    #[case] configuration: Option<&str>,
) {
    let late = forwarded(configuration, A_MINUTE);
    let within = forwarded(configuration, Duration::from_secs(20));

    assert!(
        matches!(&late, Err(NoAnswer::CallUnanswered(_))),
        "{late:?}"
    );
    assert_eq!(within.unwrap(), json!({ "answered": true }));
}
