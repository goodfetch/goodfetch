//! A person's settings writes through the engine's public interface over
//! the in-memory network (spec #592's Seam 1, the Kennel as the Client;
//! ADR 0031): `config.policy_write` with the digest `config.policy_read`
//! gave writes the Operator policy atomically, every other byte kept, and
//! records the values before and after in the Audit log; with a digest the
//! file no longer has, as after a hand edit, it answers `policy_conflict`
//! and writes nothing. `config.user_write` writes a user-level value that
//! `config.explain` then shows, and is refused where an Operator policy
//! locks the key. `config.explain` marks the keys user configuration may
//! hold and those fixed for a Run's life. Every such write, the "Needs you"
//! answers' included, holds the lock beside the Operator policy from its
//! read to its rename, so a write in flight elsewhere keeps its change.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write and read the Goodfetch home's settings files themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::{OPERATOR_POLICY_FILE, USER_CONFIGURATION_FILE};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::consent_walls::{audit_entries, unrecordable};
use super::run::call;
use super::{Client, attached_on, engine_with, per_major};
use crate::settings_editor::LOCK_FILE;

const POLICY: &str =
    "# Kept by the platform team.\nversion = \"0.1\"\n\n[cap.timeout]\nrun = 900000\n";

/// The per-major folder's `file`.
fn file(env: &TestEnv, file: &str) -> std::path::PathBuf {
    per_major(env).join(file)
}

/// Writes `text` as `name` of the test home's per-major folder.
fn write(env: &TestEnv, name: &str, text: &str) {
    std::fs::create_dir_all(per_major(env)).unwrap();
    std::fs::write(file(env, name), text).unwrap();
}

/// A flag `config.explain` leaves out when it is off.
fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
}

/// Says hello as `surface` on an engine of `product`.
async fn opened(client: &mut Client, surface: &str, product: &str) {
    client
        .send(&[json!({
            "jsonrpc": "2.0", "id": 0, "method": "hello",
            "params": { "protocol_version": "0.1", "surface": surface, "needs": product, "product_version": "1.4.0" },
        })])
        .await;
    client.next().await.unwrap();
}

/// The Audit log's entries of `kind`, each its details.
fn recorded(env: &TestEnv, kind: &str) -> Vec<Value> {
    audit_entries(env)
        .into_iter()
        .filter(|entry| entry["kind"] == kind)
        .map(|entry| entry["details"].clone())
        .collect()
}

/// Reads the policy, writes a lock and a cap against the digest read, and
/// reads it again; answers the three replies.
async fn read_write_read(mut client: Client, edit: impl FnOnce()) -> [Value; 3] {
    opened(&mut client, "kennel", "collie").await;
    let read = call(&mut client, 1, "config.policy_read", json!({})).await;
    edit();
    let written = call(
        &mut client,
        2,
        "config.policy_write",
        json!({
            "digest": read["result"]["digest"],
            "changes": [
                { "table": "lock", "key": "timeout.navigation", "value": 10_000 },
                { "table": "cap", "key": "timeout.run", "value": 600_000 },
            ],
        }),
    )
    .await;
    let again = call(&mut client, 3, "config.policy_read", json!({})).await;
    [read, written, again]
}

/// A write against the digest read is made in one write, every other byte
/// of the file kept, and the Audit log receives each key's values before
/// and after; the policy read again has the written digest and entries.
///
/// Covers: story:12-96, operation:config.policy_read, operation:config.policy_write
#[test]
fn a_policy_write_with_the_digest_read_writes_atomically_with_a_diff_in_the_audit_log() {
    let env = TestEnv::new();
    write(&env, OPERATOR_POLICY_FILE, POLICY);

    let [read, written, again] = attached_on(&engine_with(Product::Collie, None), &env, |client| {
        read_write_read(client, || {})
    });

    let digest = written["result"]["digest"].clone();
    let diff = json!([
        { "table": "lock", "key": "timeout.navigation", "before": null, "after": 10_000 },
        { "table": "cap", "key": "timeout.run", "before": 900_000, "after": 600_000 },
    ]);
    assert_eq!(
        (
            &read["result"]["entries"],
            &written["result"]["diff"],
            std::fs::read_to_string(file(&env, OPERATOR_POLICY_FILE)).unwrap(),
            recorded(&env, "operator_policy_written"),
            &again["result"]["digest"],
            &again["result"]["entries"],
        ),
        (
            &json!([{ "table": "cap", "key": "timeout.run", "value": 900_000 }]),
            &diff,
            "# Kept by the platform team.\nversion = \"0.1\"\n\n[cap.timeout]\nrun = 600000\n\n[lock.timeout]\nnavigation = 10000\n".to_owned(),
            vec![json!({
                "digest_before": read["result"]["digest"],
                "digest_after": digest,
                "diff": diff,
            })],
            &digest,
            &json!([
                { "table": "lock", "key": "timeout.navigation", "value": 10_000 },
                { "table": "cap", "key": "timeout.run", "value": 600_000 },
            ]),
        )
    );
}

/// A write against a digest the file no longer has, after a hand edit since
/// the read, answers `policy_conflict`, whose `fix` says to read the policy
/// again, and writes nothing: the hand edit stays, and the Audit log
/// records no write.
///
/// Covers: story:12-96, operation:config.policy_write, error:policy_conflict
#[test]
fn a_policy_write_with_a_stale_digest_is_a_policy_conflict_and_changes_nothing() {
    let env = TestEnv::new();
    write(&env, OPERATOR_POLICY_FILE, POLICY);
    let hand_edit = "version = \"0.1\"\n\n[cap.timeout]\nrun = 300000\n";

    let [read, written, _] = attached_on(&engine_with(Product::Collie, None), &env, |client| {
        read_write_read(client, || write(&env, OPERATOR_POLICY_FILE, hand_edit))
    });

    assert_eq!(
        (
            &written["error"]["data"]["code"],
            &written["error"]["data"]["digest"],
            written["error"]["data"]["fix"]
                .as_str()
                .unwrap()
                .contains("config.policy_read"),
            std::fs::read_to_string(file(&env, OPERATOR_POLICY_FILE)).unwrap(),
            recorded(&env, "operator_policy_written"),
        ),
        (
            &json!("policy_conflict"),
            &read["result"]["digest"],
            true,
            hand_edit.to_owned(),
            Vec::<Value>::new(),
        )
    );
}

/// The same write sent again against the digest first read, as a Client
/// resends an idempotent write after its Connection dropped before the
/// reply came, answers as written: its changes hold in the file, which it
/// leaves as it is, so the person is not told nothing was written, and the
/// Audit log records the one write that was made.
///
/// Covers: story:12-96, operation:config.policy_write
#[test]
fn a_policy_write_sent_again_after_it_was_made_answers_as_written() {
    let env = TestEnv::new();
    write(&env, OPERATOR_POLICY_FILE, POLICY);

    let [first, again] = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            let read = call(&mut client, 1, "config.policy_read", json!({})).await;
            let params = json!({
                "digest": read["result"]["digest"],
                "changes": [{ "table": "cap", "key": "timeout.run", "value": 600_000 }],
            });
            let first = call(&mut client, 2, "config.policy_write", params.clone()).await;
            let again = call(&mut client, 3, "config.policy_write", params).await;
            [first, again]
        },
    );

    let written = std::fs::read_to_string(file(&env, OPERATOR_POLICY_FILE)).unwrap();
    assert_eq!(
        (
            &again["result"]["digest"],
            recorded(&env, "operator_policy_written").len(),
            written.contains("run = 600000"),
        ),
        (&first["result"]["digest"], 1, true),
        "{again}"
    );
}

/// A user-level value written from the Kennel takes effect: `config.explain`
/// shows it set by user configuration, and the Audit log records it; a
/// value an Operator policy locks is refused with `setting_locked` and
/// writes nothing.
///
/// Covers: story:12-95, operation:config.user_write, error:setting_locked
#[test]
fn a_user_level_value_written_takes_effect_and_a_locked_one_is_refused() {
    let env = TestEnv::new();
    write(
        &env,
        OPERATOR_POLICY_FILE,
        "version = \"0.1\"\n\n[lock.timeout]\nrun = 600000\n",
    );

    let (written, locked, explained) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            let user_write =
                |key: &str, value: Value| json!({ "changes": [{ "key": key, "value": value }] });
            let written = call(
                &mut client,
                1,
                "config.user_write",
                user_write("timeout.navigation", json!(45_000)),
            )
            .await;
            let locked = call(
                &mut client,
                2,
                "config.user_write",
                user_write("timeout.run", json!(60_000)),
            )
            .await;
            let explained = call(
                &mut client,
                3,
                "config.explain",
                json!({ "keys": ["timeout.navigation", "timeout.run"] }),
            )
            .await;
            (written, locked, explained)
        },
    );

    let shown: Vec<Value> = explained["result"]["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            json!([
                setting["key"],
                setting["value"],
                setting["layer"],
                flag(&setting["user_settable"])
            ])
        })
        .collect();
    assert_eq!(
        (
            &written["result"]["diff"],
            &locked["error"]["data"]["code"],
            shown,
            std::fs::read_to_string(file(&env, USER_CONFIGURATION_FILE)).unwrap(),
            recorded(&env, "user_configuration_written").len(),
        ),
        (
            &json!([{ "key": "timeout.navigation", "before": null, "after": 45_000 }]),
            &json!("setting_locked"),
            vec![
                json!(["timeout.navigation", 45_000, "user_configuration", true]),
                json!(["timeout.run", 600_000, "operator_policy", false]),
            ],
            "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 45000\n".to_owned(),
            1,
        )
    );
}

/// `config.explain` marks a key only an Operator policy may set as no
/// user-level value, and a crawl's scope as fixed for a Run's life.
///
/// Covers: story:12-95, operation:config.explain
#[test]
fn config_explain_marks_the_keys_user_configuration_may_hold_and_those_fixed_for_a_run() {
    let env = TestEnv::new();

    let explained = attached_on(
        &engine_with(Product::Foxhound, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "foxhound").await;
            call(
            &mut client,
            1,
            "config.explain",
            json!({ "keys": ["crawl.scope.strategy", "egress.no_private_endpoints", "timeout.run"] }),
        )
        .await
        },
    );

    let marks: Vec<Value> = explained["result"]["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            json!([
                setting["key"],
                flag(&setting["user_settable"]),
                flag(&setting["fixed_for_run"])
            ])
        })
        .collect();
    assert_eq!(
        marks,
        vec![
            json!(["crawl.scope.strategy", true, true]),
            json!(["egress.no_private_endpoints", false, false]),
            json!(["timeout.run", true, false]),
        ]
    );
}

/// The person's entries are refused to an SDK, which acts for a program.
///
/// Covers: operation:config.policy_write
#[test]
fn a_policy_write_from_an_sdk_is_refused() {
    let env = TestEnv::new();

    let written = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "sdk_typescript", "collie").await;
            call(
                &mut client,
                1,
                "config.policy_write",
                json!({ "digest": "sha256:0000000000000000", "changes": [] }),
            )
            .await
        },
    );

    assert_eq!(
        (
            written["result"].is_null(),
            file(&env, OPERATOR_POLICY_FILE).exists()
        ),
        (true, false)
    );
}

/// A write with no changes against a policy that is not there still writes
/// the file its answer's digest names, so the next write against that digest
/// is not stale.
///
/// Covers: operation:config.policy_write
#[test]
fn a_policy_write_with_no_changes_writes_the_file_its_digest_names() {
    let env = TestEnv::new();

    let [emptied, next] = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            let read = call(&mut client, 1, "config.policy_read", json!({})).await;
            let emptied = call(
                &mut client,
                2,
                "config.policy_write",
                json!({ "digest": read["result"]["digest"], "changes": [] }),
            )
            .await;
            let next = call(
                &mut client,
                3,
                "config.policy_write",
                json!({
                    "digest": emptied["result"]["digest"],
                    "changes": [{ "table": "cap", "key": "timeout.run", "value": 600_000 }],
                }),
            )
            .await;
            [emptied, next]
        },
    );

    assert_eq!(
        (
            emptied["error"].clone(),
            next["error"].clone(),
            file(&env, OPERATOR_POLICY_FILE).exists()
        ),
        (Value::Null, Value::Null, true),
        "{emptied} {next}"
    );
}

/// A user-level value the Audit log cannot record is written all the same,
/// and the write answers `internal_error` saying the entry is missing
/// rather than succeeding unrecorded.
///
/// Covers: operation:config.user_write
#[test]
fn a_write_the_audit_log_cannot_record_is_made_and_says_so() {
    let env = TestEnv::new();
    let home = env.home().to_path_buf();

    let written = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            unrecordable(&home);
            call(
                &mut client,
                1,
                "config.user_write",
                json!({ "changes": [{ "key": "timeout.navigation", "value": 45_000 }] }),
            )
            .await
        },
    );

    assert_eq!(
        (
            &written["error"]["code"],
            std::fs::read_to_string(file(&env, USER_CONFIGURATION_FILE))
                .unwrap()
                .contains("navigation = 45000"),
        ),
        (&json!(-32603), true),
        "{written}"
    );
}

/// Two user-level writes sent together, so the engine works them at once,
/// both land and are both recorded: the first does not hold the lock beside
/// the Operator policy while its entry waits for the Audit log's writer, so
/// the second, under the Simulation's inline blocking policy on its one
/// thread, does not wait on that lock for ever.
///
/// Covers: operation:config.user_write
#[test]
fn two_settings_writes_at_once_both_land_and_are_both_recorded() {
    let env = TestEnv::new();

    let replies = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            let user_write = |id: i64, key: &str, value: Value| {
                json!({
                    "jsonrpc": "2.0", "id": id, "method": "config.user_write",
                    "params": { "changes": [{ "key": key, "value": value }] },
                })
            };
            client
                .send(&[
                    user_write(1, "timeout.navigation", json!(45_000)),
                    user_write(2, "timeout.run", json!(60_000)),
                ])
                .await;
            let mut replies = Vec::new();
            while replies.len() < 2 {
                let message = client.next().await.unwrap();
                if message.get("id").is_some() {
                    replies.push(message);
                }
            }
            replies
        },
    );

    let written = std::fs::read_to_string(file(&env, USER_CONFIGURATION_FILE)).unwrap();
    assert_eq!(
        (
            replies
                .iter()
                .map(|reply| reply["error"].clone())
                .collect::<Vec<_>>(),
            values_at(
                &written,
                &["settings.timeout.navigation", "settings.timeout.run"]
            ),
            recorded(&env, "user_configuration_written").len(),
        ),
        (
            vec![Value::Null, Value::Null],
            vec![json!(45_000), json!(60_000)],
            2,
        ),
        "{replies:?}"
    );
}

/// `config.user_write` of `audit.retention` is seen by the next Audit log
/// entry, its own: the engine's write refreshes its snapshot of the
/// machine's settings before it records the write, so that entry prunes
/// the one recorded two seconds before it (ADR 0140).
///
/// Covers: operation:config.user_write, setting:audit.retention
#[test]
fn a_retention_written_by_the_engine_is_seen_by_the_next_audit_entry() {
    let env = TestEnv::new();

    attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            let user_write = async |client: &mut Client, id: i64, key: &str, value: Value| {
                let written = call(
                    client,
                    id,
                    "config.user_write",
                    json!({ "changes": [{ "key": key, "value": value }] }),
                )
                .await;
                assert!(written.get("result").is_some(), "{written}");
            };
            user_write(&mut client, 1, "timeout.navigation", json!(45_000)).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            user_write(&mut client, 2, "audit.retention", json!(1_000)).await;
        },
    );

    assert_eq!(
        audit_entries(&env)
            .iter()
            .map(|entry| entry["kind"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        [
            "audit_log_anchor",
            "audit_log_pruned",
            "user_configuration_written"
        ]
    );
}

/// How long the other writer holds the lock waiting for the answer under
/// test, at most.
const HELD: Duration = Duration::from_millis(300);

/// User configuration with one value of its own.
const CONFIGURATION: &str = "version = \"0.1\"\n\n[settings]\ntimeout.navigation = 20000\n";

/// Another writer of the machine's settings files, as another engine of the
/// major writes them: it holds the lock beside the Operator policy from its
/// read of a file to its write of it.
struct OtherWriter {
    answered: mpsc::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl OtherWriter {
    /// Takes the lock, reads the file at `path`, and answers once it holds
    /// both; it then waits for [`Self::answered`] or [`HELD`], writes what it
    /// read with `from` replaced by `to`, and lets the lock go.
    fn holding(path: PathBuf, from: &'static str, to: &'static str) -> Self {
        let (held, holds) = mpsc::channel();
        let (answered, waits) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let lock = std::fs::File::create(path.with_file_name(LOCK_FILE)).unwrap();
            lock.lock().unwrap();
            let read = std::fs::read_to_string(&path).unwrap();
            held.send(()).unwrap();
            let _ = waits.recv_timeout(HELD);
            std::fs::write(&path, read.replace(from, to)).unwrap();
            drop(lock);
        });
        holds.recv().unwrap();
        Self { answered, thread }
    }

    /// Tells it the answer under test replied, then waits for its write.
    fn answered(self) {
        let _ = self.answered.send(());
        self.thread.join().unwrap();
    }
}

/// The values at each of `paths`, dotted, of the settings file `text`, as
/// JSON.
fn values_at(text: &str, paths: &[&str]) -> Vec<Value> {
    fn json_of(value: &toml_edit::Value) -> Value {
        match value {
            toml_edit::Value::Integer(integer) => json!(integer.value()),
            toml_edit::Value::String(text) => json!(text.value()),
            toml_edit::Value::Array(array) => array.iter().map(json_of).collect(),
            other => panic!("no such value in these tests: {other}"),
        }
    }
    let document: toml_edit::DocumentMut = text.parse().unwrap();
    paths
        .iter()
        .map(|path| {
            path.split('.')
                .try_fold(document.as_item(), |item, key| item.get(key))
                .and_then(toml_edit::Item::as_value)
                .map_or(Value::Null, json_of)
        })
        .collect()
}

/// A person's settings write, made while another writer holds the lock
/// beside the Operator policy between its read and its write of the same
/// file, waits for that write and keeps it: both changes land.
#[rstest]
/// Covers: operation:needs_you.deny_wall_domain
#[case::a_wall_domain_denied(
    OPERATOR_POLICY_FILE,
    POLICY,
    ("run = 900000", "run = 600000"),
    "needs_you.deny_wall_domain",
    json!({ "domain": "walled.test", "reason": "The site's owner objected." }),
    ["cap.timeout.run", "settings.wall.denied_domains"],
    [json!(600_000), json!(["walled.test"])],
)]
/// Covers: operation:needs_you.allow_consent_domain
#[case::a_consent_domain_allowed(
    USER_CONFIGURATION_FILE,
    CONFIGURATION,
    ("= 20000", "= 30000"),
    "needs_you.allow_consent_domain",
    json!({ "domain": "example.com" }),
    ["settings.timeout.navigation", "settings.consent.domains"],
    [json!(30_000), json!(["example.com"])],
)]
/// Covers: operation:config.user_write
#[case::a_user_level_value_written(
    USER_CONFIGURATION_FILE,
    CONFIGURATION,
    ("= 20000", "= 30000"),
    "config.user_write",
    json!({ "changes": [{ "key": "timeout.run", "value": 60_000 }] }),
    ["settings.timeout.navigation", "settings.timeout.run"],
    [json!(30_000), json!(60_000)],
)]
fn a_settings_write_waits_for_a_write_in_flight_and_both_land(
    #[case] name: &str,
    #[case] text: &str,
    #[case] other_change: (&'static str, &'static str),
    #[case] method: &'static str,
    #[case] params: Value,
    #[case] paths: [&str; 2],
    #[case] expected: [Value; 2],
) {
    let env = TestEnv::new();
    write(&env, name, text);
    let path = file(&env, name);

    let answer = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "kennel", "collie").await;
            // Held from here, so a slow start cannot spend the hold before
            // the call: with the lock broken, the call's write then ends
            // first, and the other writer's overwrites it.
            let other = OtherWriter::holding(path, other_change.0, other_change.1);
            let answer = call(&mut client, 1, method, params).await;
            other.answered();
            answer
        },
    );

    let written = std::fs::read_to_string(file(&env, name)).unwrap();
    assert_eq!(
        (answer["error"].clone(), values_at(&written, &paths)),
        (Value::Null, expected.to_vec()),
        "{written}"
    );
}
