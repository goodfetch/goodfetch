//! A person's settings writes through the engine's public interface over
//! the in-memory network (spec #592's Seam 1, the Console as the Client;
//! ADR 0031): `config.policy_write` with the digest `config.policy_read`
//! gave writes the Operator policy atomically, every other byte kept, and
//! records the values before and after in the Audit log; with a digest the
//! file no longer has, as after a hand edit, it answers `policy_conflict`
//! and writes nothing. `config.user_write` writes a user-level value that
//! `config.explain` then shows, and is refused where an Operator policy
//! locks the key. `config.explain` marks the keys user configuration may
//! hold and those fixed for a Run's life.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write and read the Goodfetch home's settings files themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::consent_walls::{audit_entries, unrecordable};
use super::run::call;
use super::{Client, attached_on, engine_with};

const POLICY: &str =
    "# Kept by the platform team.\nversion = \"0.1\"\n\n[cap.timeout]\nrun = 900000\n";

/// The per-major folder's `file`.
fn file(env: &TestEnv, file: &str) -> std::path::PathBuf {
    env.home().join("1").join(file)
}

/// Writes `text` as `name` of the test home's per-major folder.
fn write(env: &TestEnv, name: &str, text: &str) {
    std::fs::create_dir_all(env.home().join("1")).unwrap();
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
    opened(&mut client, "console", "collie").await;
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
    write(&env, "operator-policy.toml", POLICY);

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
            std::fs::read_to_string(file(&env, "operator-policy.toml")).unwrap(),
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
    write(&env, "operator-policy.toml", POLICY);
    let hand_edit = "version = \"0.1\"\n\n[cap.timeout]\nrun = 300000\n";

    let [read, written, _] = attached_on(&engine_with(Product::Collie, None), &env, |client| {
        read_write_read(client, || write(&env, "operator-policy.toml", hand_edit))
    });

    assert_eq!(
        (
            &written["error"]["data"]["code"],
            &written["error"]["data"]["digest"],
            written["error"]["data"]["fix"]
                .as_str()
                .unwrap()
                .contains("config.policy_read"),
            std::fs::read_to_string(file(&env, "operator-policy.toml")).unwrap(),
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
    write(&env, "operator-policy.toml", POLICY);

    let [first, again] = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "console", "collie").await;
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

    let written = std::fs::read_to_string(file(&env, "operator-policy.toml")).unwrap();
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

/// A user-level value written from the Console takes effect: `config.explain`
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
        "operator-policy.toml",
        "version = \"0.1\"\n\n[lock.timeout]\nrun = 600000\n",
    );

    let (written, locked, explained) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            opened(&mut client, "console", "collie").await;
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
            std::fs::read_to_string(file(&env, "configuration.toml")).unwrap(),
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
            opened(&mut client, "console", "foxhound").await;
            call(
            &mut client,
            1,
            "config.explain",
            json!({ "keys": ["console.sign_in.idle_expiry", "crawl.scope.strategy", "timeout.run"] }),
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
            json!(["console.sign_in.idle_expiry", false, false]),
            json!(["crawl.scope.strategy", true, true]),
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
            file(&env, "operator-policy.toml").exists()
        ),
        (true, false)
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
            opened(&mut client, "console", "collie").await;
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
            std::fs::read_to_string(file(&env, "configuration.toml"))
                .unwrap()
                .contains("navigation = 45000"),
        ),
        (&json!(-32603), true),
        "{written}"
    );
}
