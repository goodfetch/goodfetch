//! `usage` through Collie's executable (spec #213's Seam 1; ADRs 0041,
//! 0059 and 0090): `usage set` adds a Limit to the user's Usage ledger, in
//! the Goodfetch home `GOODFETCH_HOME` names, outside every per-major
//! folder; `usage list` shows the Usage against each Limit by scope; a
//! flag naming a value only Foxhound carries is refused with
//! `not_in_product`; and `usage raise`, a person's answer, is asked in a
//! pseudo-terminal and refused with `person_required` when stdin is no
//! terminal (ADRs 0041 and 0069).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{TestEnv, cli, pty};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::engine_choice::{On, daemon_dispatched};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Covers: story:3-67, story:3-79, command:usage.set, command:usage.list
#[test]
fn usage_lists_a_limit_set_in_the_ledger_of_the_goodfetch_home() {
    let env = TestEnv::new();
    let set = cli(&env, COLLIE)
        .args([
            "usage",
            "set",
            "--scope",
            "install",
            "--usage-kind",
            "requests",
            "--amount",
            "500",
        ])
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");

    let listed = cli(&env, COLLIE)
        .args(["usage", "list", "--json"])
        .run()
        .unwrap();

    let limits: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(
        (
            listed.exit,
            limits,
            env.home().join("usage-ledger.sqlite").is_file()
        ),
        (
            Some(0),
            json!({ "limits": [{
                "limit": {
                    "scope": { "kind": "install", "id": "" },
                    "usage_kind": "requests",
                    "amount": 500,
                    "must_understand": true,
                    "set_by": "client",
                },
                "standing": "held",
                "charged": 0,
                "held": 0,
                "left": 500,
            }]}),
            true
        )
    );
}

/// Covers: story:3-67, error:not_in_product
#[test]
fn a_flag_naming_paid_fetch_is_refused_by_collie() {
    let env = TestEnv::new();

    let refused = cli(&env, COLLIE)
        .args([
            "usage",
            "set",
            "--scope",
            "install",
            "--usage-kind",
            "paid_fetch",
            "--currency",
            "USD",
            "--amount",
            "5000000",
            "--json",
        ])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap();
    assert_eq!(
        (&printed["error"]["code"], refused.exit),
        (&json!("not_in_product"), Some(10))
    );
}

/// A ledger on a temporary filesystem is lost on each restart, as in a
/// container without a persistent volume, so `usage` warns (ADR 0041).
/// `/dev/shm` is a tmpfs on every Linux.
///
/// Covers: story:3-78
#[cfg(target_os = "linux")]
#[test]
fn usage_warns_when_the_ledger_sits_on_a_temporary_filesystem() {
    let env = TestEnv::new();
    let home = tempfile::Builder::new()
        .prefix("goodfetch home é ")
        .tempdir_in("/dev/shm")
        .unwrap();

    let listed = cli(&env, COLLIE)
        .env("GOODFETCH_HOME", home.path())
        .args(["usage", "list", "--json"])
        .run()
        .unwrap();

    assert_eq!(
        (
            listed.exit,
            listed.stderr.contains(&format!(
                "warning: the Usage ledger `{}` is on a temporary filesystem (tmpfs)",
                home.path().join("usage-ledger.sqlite").display()
            )),
        ),
        (Some(0), true),
        "{listed}"
    );
}

/// The arguments of `usage raise` and `usage set` for an install Limit of
/// `amount` requests.
fn install_requests(command: &str, amount: u64) -> Vec<String> {
    [
        "usage",
        command,
        "--scope",
        "install",
        "--usage-kind",
        "requests",
        "--amount",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([amount.to_string()])
    .collect()
}

/// The install Limit's amount and who set it, as `usage list --json`
/// shows them.
fn install_limit(env: &TestEnv) -> (Value, Value) {
    let listed = cli(env, COLLIE)
        .args(["usage", "list", "--json"])
        .run()
        .unwrap();
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap_or_else(|_| panic!("{listed}"));
    let limit = &listed["limits"][0]["limit"];
    assert!(limit.is_object(), "{listed}");
    (limit["amount"].clone(), limit["set_by"].clone())
}

/// Covers: story:3-75, command:usage.raise
#[rstest::rstest]
#[case::answered_yes("y", (json!(1_000), json!("person")))]
#[case::answered_no("n", (json!(500), json!("client")))]
fn usage_raise_asks_the_person_at_the_terminal(
    #[case] answer: &str,
    #[case] after: (Value, Value),
) {
    let env = TestEnv::new();
    let set = cli(&env, COLLIE)
        .args(install_requests("set", 500))
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");
    let mut raise = pty(&env, COLLIE)
        .args(install_requests("raise", 1_000))
        .spawn()
        .unwrap();

    raise
        .expect("Raise the install Limit on requests to 1000?")
        .unwrap();
    raise.send_line(answer).unwrap();
    let raised = raise.finish().unwrap();

    assert_eq!(
        (raised.exit, install_limit(&env)),
        (Some(0), after),
        "{}",
        raised.stripped()
    );
}

/// `usage raise` runs on the engine its flags choose: an Attached engine
/// in its process under `--attached`, the Daemon of the major, which it
/// starts, under `--daemon`; either raises the same Limit (ticket #1183).
///
/// Covers: story:3-75, story:11-199, command:usage.raise
#[rstest::rstest]
#[case::attached(On::Attached)]
#[case::daemon(On::Daemon)]
fn usage_raise_runs_on_the_engine_its_flags_choose(#[case] on: On) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let set = cli(&env, COLLIE)
        .args(install_requests("set", 500))
        .arg("--attached")
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");
    let mut raise = pty(&env, COLLIE)
        .args(install_requests("raise", 1_000))
        .arg(on.flag())
        .spawn()
        .unwrap();

    raise
        .expect("Raise the install Limit on requests to 1000?")
        .unwrap();
    raise.send_line("y").unwrap();
    let raised = raise.finish().unwrap();

    assert_eq!(
        (
            raised.exit,
            install_limit(&env),
            daemon_dispatched(&env, "usage.raise"),
        ),
        (Some(0), (json!(1_000), json!("person")), on == On::Daemon),
        "{}",
        raised.stripped()
    );
}

/// Covers: story:3-75, command:usage.raise
#[test]
fn usage_raise_is_refused_when_stdin_is_no_terminal() {
    let env = TestEnv::new();
    let set = cli(&env, COLLIE)
        .args(install_requests("set", 500))
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");

    let refused = cli(&env, COLLIE)
        .args(install_requests("raise", 1_000))
        .stdin("y\n")
        .run()
        .unwrap();

    assert_eq!(
        (
            refused.exit,
            refused.stderr.contains("code: person_required"),
            install_limit(&env),
        ),
        (Some(4), true, (json!(500), json!("client"))),
        "{refused}"
    );
}
