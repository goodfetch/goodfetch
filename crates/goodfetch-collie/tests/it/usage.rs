//! `usage` through Collie's executable (spec #213's Seam 1; ADRs 0041,
//! 0059 and 0090): `usage set` adds a Limit to the user's Usage ledger, in
//! the Goodfetch home `GOODFETCH_HOME` names, outside every per-major
//! folder; `usage list` shows the Usage against each Limit by scope; and
//! `usage raise`, a person's answer, is asked in a pseudo-terminal and
//! refused with `person_required` under `--json` or when stdin is no
//! terminal (ADRs 0011, 0041 and 0069).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::{ErrorGroup, Product};
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, Hello};
use goodfetch_test_support::{CliOutput as Output, TestEnv, assert_cli_snapshot, cli, pty};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::engine_choice::{On, daemon_dispatched};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Under `--json`, `usage list` prints `UsageListResult`; for people, its
/// fields.
///
/// Covers: story:3-67, story:3-79, story:11-212, command:usage.set, command:usage.list
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
        .json("UsageListResult")
        .unwrap();
    let human = cli(&env, COLLIE)
        .args(["usage", "list"])
        .run()
        .unwrap()
        .without_dispatch_records();

    assert_cli_snapshot!("usage_list_human", human);
    assert_eq!(
        (
            listed.output.exit,
            human.exit,
            listed.documents[0].clone(),
            env.home().join("usage-ledger.sqlite").is_file()
        ),
        (
            Some(0),
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

/// Covers: story:3-75, story:11-210, command:usage.raise
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

/// The refusal a failed command printed: its code, whether it is
/// retryable and its `fix`, from the `CommandFailed` on stdout under
/// `--json`, else from its lines on stderr.
fn refusal(output: &Output, json: bool) -> (Value, Value, String) {
    if json {
        let printed: Value =
            serde_json::from_str(&output.stdout).unwrap_or_else(|_| panic!("{output}"));
        let error = &printed["error"];
        return (
            error["code"].clone(),
            error["retryable"].clone(),
            error["fix"].as_str().unwrap_or_default().to_owned(),
        );
    }
    let line = |label: &str| {
        output
            .stderr
            .lines()
            .find_map(|line| line.strip_prefix(label))
            .unwrap_or_default()
            .to_owned()
    };
    (
        json!(line("code: ")),
        json!(line("retryable: ") == "true"),
        line("fix: "),
    )
}

/// Where no person answers, `usage raise` is refused with
/// `person_required` before any engine call: under `--json` as one
/// `CommandFailed` on stdout, from a pipe or a script on stderr. Each exits
/// `4`, its `fix` names the interactive command and the Kennel, and the
/// Limit stays as it was.
///
/// Covers: story:3-75, story:11-210, command:usage.raise, error:person_required, exit:refused
#[rstest::rstest]
#[case::under_json(true, Some("y\n"))]
#[case::from_a_pipe(false, Some("y\n"))]
#[case::from_a_script(false, None)]
fn usage_raise_is_refused_where_no_person_answers(#[case] json: bool, #[case] stdin: Option<&str>) {
    let env = TestEnv::new();
    let set = cli(&env, COLLIE)
        .args(install_requests("set", 500))
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");

    let mut raise = cli(&env, COLLIE).args(install_requests("raise", 1_000));
    if json {
        raise = raise.arg("--json");
    }
    if let Some(stdin) = stdin {
        raise = raise.stdin(stdin);
    }
    let refused = raise.run().unwrap();

    let (code, retryable, fix) = refusal(&refused, json);
    assert_eq!(
        (
            refused.exit,
            code,
            retryable,
            fix.contains("`usage raise`") && fix.contains("Kennel"),
            install_limit(&env),
        ),
        (
            Some(4),
            json!("person_required"),
            json!(false),
            true,
            (json!(500), json!("client"))
        ),
        "{refused}"
    );
}

/// `usage raise --json` is refused at a terminal too: a person answers
/// the prompt, never a program reading its JSON. It asks nothing.
///
/// Covers: story:11-210, command:usage.raise, error:person_required
#[test]
fn usage_raise_under_json_is_refused_at_a_terminal() {
    let env = TestEnv::new();
    let set = cli(&env, COLLIE)
        .args(install_requests("set", 500))
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");

    let refused = pty(&env, COLLIE)
        .args(install_requests("raise", 1_000))
        .arg("--json")
        .spawn()
        .unwrap()
        .finish()
        .unwrap();

    let printed = refused.stripped();
    assert_eq!(
        (
            refused.exit,
            printed.contains(r#""code":"person_required""#),
            printed.contains("Raise the install Limit"),
            install_limit(&env),
        ),
        (Some(4), true, false, (json!(500), json!("client"))),
        "{printed}"
    );
}

/// `person_required` is one refusal for the CLI and an agent alike: the
/// failure `usage raise --json` prints and an Attached engine's reply to
/// `usage.raise` from an MCP server carry the same code, the `refused`
/// group, which the SDKs raise as their `RefusedError`, and the same
/// retryable.
///
/// Covers: story:11-210, operation:usage.raise, error:person_required
#[test]
fn person_required_is_one_refusal_for_the_cli_and_an_agent() {
    let env = TestEnv::new();

    let printed = cli(&env, COLLIE)
        .args(install_requests("raise", 1_000))
        .arg("--json")
        .run()
        .unwrap();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(
        &Hello::needing(Product::Collie)
            .surface(Surface::Mcp)
            .message(),
    );
    let _ = engine.next();
    let replied = engine.ask(
        1,
        "usage.raise",
        &json!({ "scope": "install", "usage_kind": "requests", "amount": 1_000 }),
    );
    let _ = engine.finish();

    let printed: Value =
        serde_json::from_str(&printed.stdout).unwrap_or_else(|_| panic!("{printed}"));
    let replied_group = ErrorGroup::ALL
        .into_iter()
        .find(|group| json!(group.json_rpc_class()) == replied["error"]["code"])
        .map(|group| json!(group.as_str()));
    assert_eq!(
        (
            printed["error"]["code"].clone(),
            Some(printed["error"]["group"].clone()),
            printed["error"]["retryable"].clone(),
        ),
        (
            replied["error"]["data"]["code"].clone(),
            replied_group,
            replied["error"]["data"]["retryable"].clone(),
        ),
        "{replied}"
    );
    assert_eq!(printed["error"]["group"], json!("refused"));
}
