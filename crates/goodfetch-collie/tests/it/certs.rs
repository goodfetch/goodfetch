//! `certs list` through Collie's executable (spec #564's seam; ADRs 0042
//! and 0066): the known-certificates store of the Goodfetch home
//! `GOODFETCH_HOME` names, written through the engine's own store for two
//! people of one shared store, is read by the CLI command and by
//! `certs.list` over the protocol, which answer the same pins: the
//! caller's, and nothing of the other person's. A store of a newer major is
//! refused with the registry's `format_version_unsupported`.
//!
//! `certs accept` and `certs forget` are a person's answers (ADRs 0011 and
//! 0042): asked in a pseudo-terminal they replace or remove the pin, the
//! accepted change recorded in the Audit log; under `--json`, with stdin no
//! terminal, and over the protocol from any surface but the CLI's, as an
//! agent's or a program's, they are refused with `person_required` and the
//! pin stays.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::Arc;

use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::{KeyHash, KnownCertificates};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{TestEnv, cli, pty};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::engine_choice::{On, daemon_dispatched};
use goodfetch_test_client::{Attached, Hello, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The person an engine without accounts acts for: the OS user.
const CALLER: &str = "os_user";

/// A store holding a pin of the OS user's, made by a Run, and two of
/// another person's on the same shared store, one for the same host.
fn shared_store(env: &TestEnv) -> KnownCertificates {
    let store = KnownCertificates::of(
        &Home::new(env.home().to_path_buf()),
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    );
    let at: jiff::Timestamp = "2026-01-05T09:00:00Z".parse().unwrap();
    goodfetch_test_support::runtime().block_on(async {
        store
            .pin(
                CALLER,
                "expired.certificates.test",
                &KeyHash::of_public_key(b"the caller's key"),
                Some(&RunId::parse("pinned").unwrap()),
                at,
            )
            .await
            .unwrap();
        store
            .pin(
                "grace",
                "expired.certificates.test",
                &KeyHash::of_public_key(b"grace's key"),
                None,
                at,
            )
            .await
            .unwrap();
        store
            .pin(
                "grace",
                "intranet.example",
                &KeyHash::of_public_key(b"grace's intranet key"),
                None,
                at,
            )
            .await
            .unwrap();
    });
    store
}

/// What `certs list --json` printed, and what `certs.list` answered an
/// Attached engine's Client.
fn listed_both_ways(env: &TestEnv) -> (Value, Value) {
    let printed = cli(env, COLLIE)
        .args(["certs", "list", "--json"])
        .run()
        .unwrap();
    let mut engine = Attached::start(env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let answered = engine.ask(1, "certs.list", &json!({}));
    let _ = engine.finish();
    (serde_json::from_str(&printed.stdout).unwrap(), answered)
}

/// Stories 4-35 and 4-43: the caller's pins, with the SHA-256 of the key
/// each one holds, and none of the other person's on the same store.
/// Covers: story:4-35, story:4-43, command:certs.list, operation:certs.list
#[test]
fn certs_list_shows_the_callers_pins_and_nothing_of_another_persons() {
    let env = TestEnv::new();
    shared_store(&env);

    let (printed, answered) = listed_both_ways(&env);

    // SHA-256 of "the caller's key", in base64, computed apart.
    let expected = json!({
        "pins": [{
            "host": "expired.certificates.test",
            "spki_sha256": "sha256/ACwLaIK/Nlbggk81BpIt0Z2eZrh9gHln/WS7pnwGge0=",
            "pinned_at": "2026-01-05T09:00:00Z",
            "run_id": "pinned",
        }],
    });
    assert_eq!(
        (printed, answered["result"].clone()),
        (expected.clone(), expected)
    );
}

/// Covers: format:known-certificates, error:format_version_unsupported
#[test]
fn certs_list_refuses_a_store_of_a_newer_major() {
    let env = TestEnv::new();
    let store = shared_store(&env);
    Connection::open(store.path())
        .unwrap()
        .execute("UPDATE format SET version = '1.0'", [])
        .unwrap();

    let refused = cli(&env, COLLIE)
        .args(["certs", "list", "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap();
    assert_eq!(
        (
            &printed["error"]["code"],
            &printed["error"]["format"],
            &printed["error"]["found"],
        ),
        (
            &json!("format_version_unsupported"),
            &json!("known-certificates"),
            &json!("1.0")
        ),
        "{printed}"
    );
}

/// The expired host, pinned by the OS user.
const HOST: &str = "expired.certificates.test";

/// The SHA-256 of "the caller's key", in base64, computed apart.
const PINNED: &str = "sha256/ACwLaIK/Nlbggk81BpIt0Z2eZrh9gHln/WS7pnwGge0=";

/// The SHA-256 of "the changed key", in base64, computed apart.
const CHANGED: &str = "sha256/1jlINei1T2GB2iKNenHQJy1POCDvVjQqkhrCm156RBk=";

/// The shared store, with the key that stopped the caller's pinned host
/// waiting to be accepted.
fn changed_store(env: &TestEnv) -> KnownCertificates {
    let store = shared_store(env);
    goodfetch_test_support::runtime().block_on(async {
        store
            .record_change(
                CALLER,
                HOST,
                &KeyHash::of_public_key(b"the changed key"),
                Some(&RunId::parse("stopped").unwrap()),
                "2026-01-05T10:00:00Z".parse().unwrap(),
            )
            .await
            .unwrap();
    });
    store
}

/// The caller's pin of the expired host, and the change waiting for it, as
/// `certs list --json` shows them; none when it has no pin.
fn pin_of_host(env: &TestEnv) -> Option<(Value, Value)> {
    let listed = cli(env, COLLIE)
        .args(["certs", "list", "--json"])
        .run()
        .unwrap();
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap_or_else(|_| panic!("{listed}"));
    listed["pins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pin| pin["host"] == HOST)
        .map(|pin| {
            (
                pin["spki_sha256"].clone(),
                pin["changed_spki_sha256"].clone(),
            )
        })
}

/// The Audit log's entries, each its kind, actor and details.
fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let path = Home::new(env.home().to_path_buf()).audit_log();
    if !path.exists() {
        return Vec::new();
    }
    Connection::open(path)
        .unwrap()
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| {
            let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
            json!([entry["kind"], entry["actor"], entry["details"]])
        })
        .collect()
}

/// An Attached engine's answers to `calls`, made after a `hello` from
/// `surface`.
fn answers_from(env: &TestEnv, surface: Surface, calls: &[(&str, Value)]) -> Vec<Value> {
    let mut engine = Attached::start(env, COLLIE);
    engine.send(&Hello::needing(Product::Collie).surface(surface).message());
    let _ = engine.next();
    let answers = (1..)
        .zip(calls)
        .map(|(id, (method, params))| engine.ask(id, method, params))
        .collect();
    let _ = engine.finish();
    answers
}

/// Story 4-36: `certs accept` asks the person at the terminal, showing
/// both keys; a yes pins the changed key in place of the old one, records
/// the accepted change in the Audit log, which verifies, and no change
/// waits any more; a no changes nothing, calling nothing.
/// Covers: story:4-36, command:certs.accept
#[rstest::rstest]
/// Covers: operation:certs.accept
#[case::answered_yes("y", Some((json!(CHANGED), Value::Null)), 1)]
#[case::answered_no("n", Some((json!(PINNED), json!(CHANGED))), 0)]
fn certs_accept_asks_the_person_at_the_terminal(
    #[case] answer: &str,
    #[case] after: Option<(Value, Value)>,
    #[case] recorded: usize,
) {
    let env = TestEnv::new();
    changed_store(&env);
    let mut accept = pty(&env, COLLIE)
        .args(["certs", "accept", HOST])
        .spawn()
        .unwrap();

    accept
        .expect(&format!(
            "The key of {HOST} changed from {PINNED} to {CHANGED}. Pin {CHANGED} in its place? [y/N]"
        ))
        .unwrap();
    accept.send_line(answer).unwrap();
    let accepted = accept.finish().unwrap();

    assert_eq!(
        (accepted.exit, pin_of_host(&env)),
        (Some(0), after),
        "{}",
        accepted.stripped()
    );
    let accepted_change = json!([
        "certificate_change_accepted",
        { "kind": "person", "name": CALLER },
        { "host": HOST, "pinned_spki_sha256": PINNED, "accepted_spki_sha256": CHANGED },
    ]);
    assert_eq!(audit_entries(&env), [accepted_change][..recorded]);
    let verified = cli(&env, COLLIE)
        .args(["audit", "verify", "--json"])
        .run()
        .unwrap();
    let verified: Value = serde_json::from_str(&verified.stdout).unwrap();
    assert_eq!(
        (&verified["intact"], &verified["entries"]),
        (&json!(true), &json!(recorded))
    );
}

/// `certs accept` and `certs forget` make their calls on the engine their
/// flags choose: an Attached engine in their process under `--attached`,
/// the Daemon of the major, which they start, under `--daemon`; either
/// answers the person's yes the same (ticket #1183).
///
/// Covers: story:4-36, story:4-37, story:11-199
#[rstest::rstest]
/// Covers: command:certs.accept
#[case::accept_attached("accept", On::Attached, Some((json!(CHANGED), Value::Null)))]
/// Covers: command:certs.accept
#[case::accept_daemon("accept", On::Daemon, Some((json!(CHANGED), Value::Null)))]
/// Covers: command:certs.forget
#[case::forget_attached("forget", On::Attached, None)]
/// Covers: command:certs.forget
#[case::forget_daemon("forget", On::Daemon, None)]
fn certs_accept_and_forget_run_on_the_engine_their_flags_choose(
    #[case] command: &str,
    #[case] on: On,
    #[case] after: Option<(Value, Value)>,
) {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    changed_store(&env);
    let mut asked = pty(&env, COLLIE)
        .args(["certs", command, HOST, on.flag()])
        .spawn()
        .unwrap();

    asked.expect("[y/N]").unwrap();
    asked.send_line("y").unwrap();
    let answered = asked.finish().unwrap();

    assert_eq!(
        (
            answered.exit,
            pin_of_host(&env),
            daemon_dispatched(&env, &format!("certs.{command}")),
        ),
        (Some(0), after, on == On::Daemon),
        "{}",
        answered.stripped()
    );
}

/// Story 4-36: under `--json`, and with stdin no terminal as under a
/// script, an agent or CI, `certs accept` is refused with
/// `person_required` and exit 4 before any engine call; the pin and its
/// waiting change stay. The CLI refuses it itself, so no reply carries the
/// code: `a_person_only_certs_call_is_refused_to_every_surface_but_a_persons`
/// reaches `error:person_required` over the protocol.
/// Covers: story:4-36, command:certs.accept, exit:refused
#[rstest::rstest]
#[case::under_json(&["certs", "accept", HOST, "--json"], "person_required")]
#[case::from_a_pipe(&["certs", "accept", HOST], "")]
fn certs_accept_is_refused_under_json_and_without_a_terminal(
    #[case] args: &[&str],
    #[case] printed_code: &str,
) {
    let env = TestEnv::new();
    changed_store(&env);

    let refused = cli(&env, COLLIE).args(args).stdin("y\n").run().unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap_or(Value::Null);
    assert_eq!(
        (
            refused.exit,
            printed["error"]["code"].as_str().unwrap_or_default(),
            refused.stderr.contains("code: person_required") || !printed_code.is_empty(),
            pin_of_host(&env),
            audit_entries(&env).len(),
        ),
        (
            Some(4),
            printed_code,
            true,
            Some((json!(PINNED), json!(CHANGED))),
            0
        ),
        "{refused}"
    );
}

/// Stories 4-36 and 4-37: over the protocol, `certs.accept` and
/// `certs.forget` are refused with `person_required` to a Client acting
/// for a model (MCP) and to a program's SDK, through which a Recipe's code
/// reaches the engine too; the CLI's own Connection is answered (positive
/// control), and the refused calls changed nothing.
/// Covers: story:4-36, story:4-37, operation:certs.accept, operation:certs.forget
#[rstest::rstest]
/// Covers: error:person_required
#[case::from_an_agent(Surface::Mcp, "person_required", Some((json!(PINNED), json!(CHANGED))))]
/// Covers: error:person_required
#[case::from_the_typescript_sdk(
    Surface::SdkTypescript,
    "person_required",
    Some((json!(PINNED), json!(CHANGED)))
)]
/// Covers: error:person_required
#[case::from_the_python_sdk(Surface::SdkPython, "person_required", Some((json!(PINNED), json!(CHANGED))))]
/// Covers: error:person_required
#[case::from_a_rust_program(Surface::Rust, "person_required", Some((json!(PINNED), json!(CHANGED))))]
#[case::from_the_cli(Surface::Cli, "answered", None)]
fn a_person_only_certs_call_is_refused_to_every_surface_but_a_persons(
    #[case] surface: Surface,
    #[case] expected: &str,
    #[case] after: Option<(Value, Value)>,
) {
    let env = TestEnv::new();
    changed_store(&env);

    let answers = answers_from(
        &env,
        surface,
        &[
            (
                "certs.accept",
                json!({ "host": HOST, "spki_sha256": CHANGED }),
            ),
            ("certs.forget", json!({ "host": HOST })),
        ],
    );

    let outcome = |reply: &Value| match &reply["error"]["data"]["code"] {
        Value::String(code) => code.clone(),
        _ if reply["result"].is_object() => "answered".to_owned(),
        _ => panic!("{reply}"),
    };
    assert_eq!(
        answers.iter().map(outcome).collect::<Vec<_>>(),
        [expected, expected]
    );
    assert_eq!(pin_of_host(&env), after);
}

/// Story 4-36: a key other than the one that stopped the host is not
/// accepted, even by a person; the pin stays.
/// Covers: story:4-36, operation:certs.accept
#[test]
fn certs_accept_refuses_a_key_that_did_not_stop_the_host() {
    let env = TestEnv::new();
    changed_store(&env);

    let answers = answers_from(
        &env,
        Surface::Cli,
        &[(
            "certs.accept",
            json!({ "host": HOST, "spki_sha256": PINNED }),
        )],
    );

    assert_eq!(
        (&answers[0]["error"]["code"], pin_of_host(&env)),
        (&json!(-32602), Some((json!(PINNED), json!(CHANGED)))),
        "{}",
        answers[0]
    );
}

/// Story 4-37: `certs forget` asks the person at the terminal; a yes
/// removes the caller's pin and its waiting change, and nothing of another
/// person's; a no keeps it, calling nothing.
/// Covers: story:4-37, command:certs.forget
#[rstest::rstest]
/// Covers: operation:certs.forget
#[case::answered_yes("y", None)]
#[case::answered_no("n", Some((json!(PINNED), json!(CHANGED))))]
fn certs_forget_asks_the_person_at_the_terminal(
    #[case] answer: &str,
    #[case] after: Option<(Value, Value)>,
) {
    let env = TestEnv::new();
    let store = changed_store(&env);
    let mut forget = pty(&env, COLLIE)
        .args(["certs", "forget", HOST])
        .spawn()
        .unwrap();

    forget
        .expect(&format!(
            "Remove the pin {PINNED} of {HOST}? Its next connection under trust on first use pins again. [y/N]"
        ))
        .unwrap();
    forget.send_line(answer).unwrap();
    let forgotten = forget.finish().unwrap();

    assert_eq!(
        (forgotten.exit, pin_of_host(&env)),
        (Some(0), after),
        "{}",
        forgotten.stripped()
    );
    assert_eq!(
        goodfetch_test_support::runtime()
            .block_on(store.list("grace"))
            .unwrap()
            .len(),
        2
    );
}

/// Story 4-37: under `--json`, and with stdin no terminal, `certs forget`
/// is refused with `person_required` and exit 4; the pin stays. As for
/// `certs accept`, the CLI refuses it itself.
/// Covers: story:4-37, command:certs.forget, exit:refused
#[rstest::rstest]
#[case::under_json(&["certs", "forget", HOST, "--json"], "person_required")]
#[case::from_a_pipe(&["certs", "forget", HOST], "")]
fn certs_forget_is_refused_under_json_and_without_a_terminal(
    #[case] args: &[&str],
    #[case] printed_code: &str,
) {
    let env = TestEnv::new();
    changed_store(&env);

    let refused = cli(&env, COLLIE).args(args).stdin("y\n").run().unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap_or(Value::Null);
    assert_eq!(
        (
            refused.exit,
            printed["error"]["code"].as_str().unwrap_or_default(),
            refused.stderr.contains("code: person_required") || !printed_code.is_empty(),
            pin_of_host(&env),
        ),
        (
            Some(4),
            printed_code,
            true,
            Some((json!(PINNED), json!(CHANGED)))
        ),
        "{refused}"
    );
}

/// Stories 4-36 and 4-37: with stdin no terminal, `certs accept` and
/// `certs forget` are refused before any engine call, so the refusal never
/// depends on the store: of a host without a pin, too, it is
/// `person_required` and exit 4, never the engine's answer about the pin.
/// Covers: story:4-36, story:4-37, exit:refused
#[rstest::rstest]
/// Covers: command:certs.accept
#[case::accept("accept")]
/// Covers: command:certs.forget
#[case::forget("forget")]
fn a_person_only_certs_command_from_a_pipe_is_refused_before_reading_the_store(
    #[case] command: &str,
) {
    let env = TestEnv::new();

    let refused = cli(&env, COLLIE)
        .args(["certs", command, HOST])
        .stdin("y\n")
        .run()
        .unwrap();

    assert_eq!(
        (
            refused.exit,
            refused.stderr.contains("code: person_required")
        ),
        (Some(4), true),
        "{refused}"
    );
}
