//! `needs-you` through Collie's executable (specs #592 and #1937; ADR
//! 0069): `needs-you list` prints what waits for a person, nothing on a
//! fresh home with the storage levels off, and `needs-you dismiss` of an
//! item the list does not hold dismisses nothing.
//!
//! `needs-you answer` takes a person at a terminal through the list: in a
//! pseudo-terminal they choose an item, answer its kind's prompt and see it
//! leave, as an item another person resolves does; a kind with no prompt is
//! listed with what can be done about it. Under `--json`, from a pipe and
//! from a script it is refused with `person_required`.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{PtyRun, TestEnv, cli, pty};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::certs::{CHANGED, HOST, PINNED, changed_store, pin_of_host};
use crate::daemon::StopsTheDaemon;
use crate::engine_choice::{On, daemon_dispatched};
use crate::usage::{install_limit, install_requests, refusal};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Both storage levels off, so no filesystem of the machine is listed,
/// however full, and a sample each 100 ms, which every source reads its
/// items again at.
const QUIET: &str =
    "storage.low_free = 0\nstorage.critical_free = 0\ngovernor.sample_interval = 100\n";

/// What asks the person for an item.
const WHICH_ITEM: &str = "Which item? Its number, or Enter to end:";

/// What asks the person how to answer an item.
const WHICH_ANSWER: &str = "Which answer? Its number, or Enter for the list:";

/// What ends a session whose list holds nothing any more.
const NOTHING: &str = "Nothing waits for a person.";

/// Covers: command:needs-you.list, command:needs-you.dismiss
#[test]
fn needs_you_lists_nothing_on_a_fresh_home_and_dismisses_no_item_it_does_not_hold() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);

    let listed = cli(&env, COLLIE)
        .args(["needs-you", "list", "--json"])
        .run()
        .unwrap();
    let dismissed = cli(&env, COLLIE)
        .args(["needs-you", "dismiss", "--item", "gone", "--json"])
        .run()
        .unwrap();

    let parsed = |stdout: &str| serde_json::from_str::<Value>(stdout).unwrap_or(Value::Null);
    assert_eq!(
        (
            listed.exit,
            parsed(&listed.stdout),
            dismissed.exit,
            parsed(&dismissed.stdout)
        ),
        (
            Some(0),
            json!({ "items": [] }),
            Some(0),
            json!({ "dismissed": false })
        ),
        "{listed}\n{dismissed}"
    );
}

/// The kinds `needs-you list --json` lists.
fn kinds_listed(env: &TestEnv) -> Vec<String> {
    let listed = cli(env, COLLIE)
        .args(["needs-you", "list", "--json"])
        .run()
        .unwrap();
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap_or_else(|_| panic!("{listed}"));
    listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// `needs-you answer` in a pseudo-terminal, with `arguments`.
fn answering<'env>(env: &'env TestEnv, arguments: &[&str]) -> PtyRun<'env> {
    pty(env, COLLIE)
        .args(["needs-you", "answer"])
        .args(arguments)
        .spawn()
        .unwrap()
}

/// Waits for each text and types the line after it.
fn talk(session: &mut PtyRun<'_>, exchanges: &[(&str, &str)]) {
    for (shown, typed) in exchanges {
        session.expect(shown).unwrap();
        session.send_line(typed).unwrap();
    }
}

/// The question `certs accept` asks, too.
fn accept_question() -> String {
    format!(
        "The key of {HOST} changed from {PINNED} to {CHANGED}. Pin {CHANGED} in its place? [y/N]"
    )
}

/// A person is taken through the list: a changed certificate is listed with
/// its answers, kept by a no, accepted by a yes, and leaves the list, on the
/// engine the flags choose.
///
/// Covers: story:12-139, story:12-23, story:12-32, command:needs-you.answer
#[rstest::rstest]
#[case::attached(On::Attached)]
#[case::daemon(On::Daemon)]
fn a_person_answers_a_changed_certificate_from_the_list(#[case] on: On) {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    changed_store(&env);
    let question = accept_question();
    let mut session = answering(&env, &[on.flag()]);

    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "1"),
            (&question, "n"),
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "1"),
            (&question, "y"),
        ],
    );
    session.expect("Left the list: 1.").unwrap();
    session.expect(NOTHING).unwrap();
    let answered = session.finish().unwrap();

    assert_eq!(
        (
            answered.exit,
            pin_of_host(&env),
            kinds_listed(&env),
            daemon_dispatched(&env, "certs.accept"),
        ),
        (
            Some(0),
            Some((json!(CHANGED), Value::Null)),
            vec![],
            on == On::Daemon
        ),
        "{}",
        answered.stripped()
    );
}

/// `needs-you answer <item>` goes straight to one item, and ends once it is
/// answered.
///
/// Covers: story:12-139, command:needs-you.answer
#[test]
fn a_person_goes_straight_to_one_item() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    changed_store(&env);
    let mut session = answering(
        &env,
        &[&format!("certificate_changed:{HOST}"), "--attached"],
    );

    talk(
        &mut session,
        &[
            ("Which answer? Its number, or Enter to end:", "2"),
            ("pins again. [y/N]", "y"),
        ],
    );
    let answered = session.finish().unwrap();

    assert_eq!(
        (
            answered.exit,
            answered.stripped().contains(WHICH_ITEM),
            pin_of_host(&env)
        ),
        (Some(0), false, None),
        "{}",
        answered.stripped()
    );
}

/// An item the list does not hold is refused, asking nothing.
///
/// Covers: story:12-139, command:needs-you.answer
#[test]
fn an_item_that_is_not_listed_is_refused() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);

    let refused = answering(&env, &["gone", "--attached"]).finish().unwrap();

    let printed = refused.stripped();
    assert_eq!(
        (
            refused.exit,
            printed.contains("code: invalid_params"),
            printed.contains("`needs-you list`")
        ),
        (Some(3), true, true),
        "{printed}"
    );
}

/// What another person resolves leaves the list while it is open, and an
/// item chosen meanwhile is not answered.
///
/// Covers: story:12-32, story:12-139
#[test]
fn an_item_another_person_resolves_leaves_the_open_list() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    changed_store(&env);
    let mut session = answering(&env, &["--daemon"]);
    talk(&mut session, &[(WHICH_ITEM, "1")]);
    session.expect(WHICH_ANSWER).unwrap();

    let mut other = pty(&env, COLLIE)
        .args(["certs", "forget", HOST, "--daemon"])
        .spawn()
        .unwrap();
    talk(&mut other, &[("[y/N]", "y")]);
    let other = other.finish().unwrap();
    session.expect("Left the list: 1.").unwrap();
    session.expect(NOTHING).unwrap();
    let ended = session.finish().unwrap();

    assert_eq!(
        (other.exit, ended.exit),
        (Some(0), Some(0)),
        "{}\n{}",
        other.stripped(),
        ended.stripped()
    );
}

/// A Run of the index `run_id`, which an Attached engine started and left,
/// stopped for `stop_reason`.
fn stopped_run(env: &TestEnv, run_id: &str, stop_reason: &Value) {
    let started = cli(env, COLLIE)
        .args(["run", "start", "--run-id", run_id, "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{started}");
    let changed = Connection::open(HomeLayout::of(env.home()).run_index())
        .unwrap()
        .execute(
            "UPDATE runs SET lifecycle = 'stopped', stop_reason = ?2, ended_at = coalesce(ended_at, started_at) WHERE run_id = ?1",
            (run_id, stop_reason.to_string()),
        )
        .unwrap();
    assert_eq!(changed, 1);
}

/// A Run a Limit stopped is answered by raising the Limit: what is no
/// amount is refused, and the Run leaves the list once the Limit has
/// something left.
///
/// Covers: story:12-139, story:12-32
#[test]
fn a_person_raises_the_limit_that_stopped_a_run_from_the_list() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    let set = cli(&env, COLLIE)
        .args(install_requests("set", 2))
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");
    stopped_run(
        &env,
        "held",
        &json!({ "kind": "limit", "scope": { "kind": "install", "id": "" }, "usage_kind": "requests" }),
    );
    let mut session = answering(&env, &["--attached"]);

    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "1"),
            (
                "The install Limit on requests allows 2. Its new amount:",
                "more",
            ),
        ],
    );
    session.expect("`more` is no amount").unwrap();
    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "1"),
            ("Its new amount:", "1000"),
            ("Raise the install Limit on requests to 1000? [y/N]", "y"),
        ],
    );
    session.expect("Left the list: 1.").unwrap();
    session.expect(NOTHING).unwrap();
    let raised = session.finish().unwrap();

    assert_eq!(
        (raised.exit, install_limit(&env), kinds_listed(&env)),
        (Some(0), (json!(1_000), json!("person")), vec![]),
        "{}",
        raised.stripped()
    );
}

/// A Run stopped by storage is listed with the space to free, and leaves
/// the list when the person dismisses it.
///
/// Covers: story:12-139, story:12-32
#[test]
fn a_person_dismisses_a_run_stopped_by_storage_from_the_list() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    stopped_run(
        &env,
        "filled",
        &json!({ "kind": "resources", "resource": "storage", "folder": "/data" }),
    );
    let mut session = answering(&env, &["--attached"]);

    session
        .expect("To do: free space on /data, then start the Run again with its Run ID")
        .unwrap();
    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            ("Dismiss this item? It does not come back. [y/N]", "y"),
        ],
    );
    session.expect("Left the list: 1.").unwrap();
    session.expect(NOTHING).unwrap();
    let dismissed = session.finish().unwrap();

    assert_eq!(
        (dismissed.exit, kinds_listed(&env)),
        (Some(0), vec![]),
        "{}",
        dismissed.stripped()
    );
}

/// A kind with no prompt is listed with what can be done about it: a
/// filesystem at a storage level asks for space, which nothing in the
/// command frees. A line that names no item asks again, and Enter ends the
/// session. The engine lists the filesystem at its first sample, before the
/// session's list or after; the certificate, waiting since longer, keeps the
/// session open until then.
///
/// Covers: story:12-139, story:12-23
#[test]
fn a_kind_with_no_prompt_is_listed_with_what_can_be_done() {
    let env = TestEnv::new();
    env.user_configuration(
        "storage.low_free = 9007199254740991\nstorage.critical_free = 0\ngovernor.sample_interval = 100\n",
    );
    changed_store(&env);
    let mut session = answering(&env, &["--attached"]);

    session.expect("2. The filesystem holding").unwrap();
    session.expect("To do: free space on").unwrap();
    talk(&mut session, &[(WHICH_ITEM, "2")]);
    session.expect("No answer is given here.").unwrap();
    talk(&mut session, &[(WHICH_ITEM, "7")]);
    session.expect("No item 7 is listed.").unwrap();
    talk(&mut session, &[(WHICH_ITEM, "")]);
    let ended = session.finish().unwrap();

    assert_eq!(ended.exit, Some(0), "{}", ended.stripped());
}

/// Where no person answers, `needs-you answer` is refused with
/// `person_required` before any engine call, exit `4`, its `fix` naming the
/// exact command, while `needs-you list` reads normally; nothing changed.
///
/// Covers: story:12-140, command:needs-you.answer, exit:refused
#[rstest::rstest]
#[case::under_json(&[], true, Some("1\n"), "`goodfetch-collie needs-you answer`")]
#[case::from_a_pipe(&[], false, Some("1\n"), "`goodfetch-collie needs-you answer`")]
#[case::from_a_script(&["gone"], false, None, "`goodfetch-collie needs-you answer gone`")]
fn needs_you_answer_is_refused_where_no_person_answers(
    #[case] item: &[&str],
    #[case] json: bool,
    #[case] stdin: Option<&str>,
    #[case] command: &str,
) {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    changed_store(&env);

    let mut asked = cli(&env, COLLIE).args(["needs-you", "answer"]).args(item);
    if json {
        asked = asked.arg("--json");
    }
    if let Some(stdin) = stdin {
        asked = asked.stdin(stdin);
    }
    let refused = asked.run().unwrap();

    let (code, retryable, fix) = refusal(&refused, json);
    assert_eq!(
        (
            refused.exit,
            code,
            retryable,
            fix.contains(command),
            pin_of_host(&env),
            kinds_listed(&env),
        ),
        (
            Some(4),
            json!("person_required"),
            json!(false),
            true,
            Some((json!(PINNED), json!(CHANGED))),
            vec!["certificate_changed".to_owned()]
        ),
        "{refused}"
    );
}

/// `needs-you answer --json` is refused at a terminal too, asking nothing.
///
/// Covers: story:12-140, command:needs-you.answer
#[test]
fn needs_you_answer_under_json_is_refused_at_a_terminal() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    changed_store(&env);

    let refused = answering(&env, &["--json"]).finish().unwrap();

    let printed = refused.stripped();
    assert_eq!(
        (
            refused.exit,
            printed.contains(r#""code":"person_required""#),
            printed.contains(WHICH_ITEM),
        ),
        (Some(4), true, false),
        "{printed}"
    );
}
