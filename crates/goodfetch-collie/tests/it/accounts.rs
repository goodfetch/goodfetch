//! The Accounts store a Daemon opens as it starts (ADR 0149), through
//! Collie's executable: `daemon run`, the Exposed Daemon, has accounts
//! unless `accounts.enabled` is off, and a machine's Daemon only when it is
//! on; a Daemon with accounts makes its store at its start, says so in
//! `daemon.status` and finds what the store held after a restart, and one
//! without makes no store.

use std::path::PathBuf;

use goodfetch_engine::accounts::AccountsStore;
use goodfetch_protocol::home::ACCOUNTS_STORE;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{RuntimeFile, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of, runtime_file};
use crate::daemon_client;
use crate::daemon_run::{ANY_PORT, Foreground, opened};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

fn store_file(env: &TestEnv) -> PathBuf {
    HomeLayout::of(env.home()).per_major().join(ACCOUNTS_STORE)
}

/// `daemon.status`'s answer from the foreground Daemon of `env`, started
/// and stopped around the call.
fn foreground_status(env: &TestEnv) -> Value {
    let mut daemon = Foreground::start(env, COLLIE, &[]);
    let endpoint = daemon.serving();
    let status = opened(env, &endpoint).call(1, "daemon.status", &json!({}));
    assert!(daemon.terminate());
    status["result"].clone()
}

/// Covers: setting:accounts.enabled, format:accounts-store
#[rstest]
#[case::by_default("", true)]
#[case::on("accounts.enabled = true\n", true)]
#[case::off("accounts.enabled = false\n", false)]
fn daemon_run_opens_its_accounts_store_unless_accounts_are_off(
    #[case] setting: &str,
    #[case] accounts: bool,
) {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n{setting}"));

    let status = foreground_status(&env);

    assert_eq!(
        (status["accounts"].clone(), store_file(&env).is_file()),
        (json!(accounts), accounts)
    );
}

/// What the Accounts store held when `daemon run` stopped is there when it
/// starts again.
/// Covers: setting:accounts.enabled, format:accounts-store
#[test]
fn daemon_runs_accounts_store_survives_a_restart() {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));
    foreground_status(&env);
    AccountsStore::open(&store_file(&env), None)
        .unwrap()
        .make_account("Grace", "2026-01-05T09:00:00Z".parse().unwrap())
        .unwrap();

    let status = foreground_status(&env);

    assert_eq!(status["accounts"], json!(true));
    assert_eq!(
        AccountsStore::open(&store_file(&env), None)
            .unwrap()
            .account()
            .unwrap()
            .map(|account| account.name),
        Some("Grace".to_owned())
    );
}

/// Covers: setting:accounts.enabled
#[rstest]
#[case::by_default("", false)]
#[case::on("accounts.enabled = true\n", true)]
fn a_machines_daemon_opens_its_accounts_store_only_when_accounts_are_on(
    #[case] setting: &str,
    #[case] accounts: bool,
) {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.idle_timeout = 60000\n{setting}"));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let found = RuntimeFile::read(&runtime_file(&env)).unwrap();

    let status = daemon_client(&found).call(1, "daemon.status", &json!({}));

    assert_eq!(
        (
            status["result"]["accounts"].clone(),
            store_file(&env).is_file()
        ),
        (json!(accounts), accounts)
    );
}
