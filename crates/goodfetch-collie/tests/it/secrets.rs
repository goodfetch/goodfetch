//! Secret references through Collie's executable (spec #564's seam; ADRs
//! 0015, 0016, 0067 and 0092): `run_state.key_reference`, the first
//! credential setting, takes only a Secret reference and resolves it when a
//! Run starts, through the `env:` provider over the engine's environment;
//! the use is recorded in the Audit log, and a reference that gives no
//! value refuses the start with `secret_unavailable`. A Secret canary
//! planted as the value is searched in every reply, the logs and every file
//! the test's folder holds, raw and encoded.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The variable the tests' key-encryption key is read from.
const VARIABLE: &str = "GF_TEST_RUN_KEY";

/// The user configuration that names the key-encryption key by reference.
fn by_reference(env: &TestEnv) {
    env.user_configuration(&format!("run_state.key_reference = \"env:{VARIABLE}\"\n"));
}

/// A canary planted as the key-encryption key's value, with its ID.
fn planted(env: &TestEnv) -> (Canaries, String, String) {
    let mut canaries = Canaries::new(env.seed());
    let canary = canaries.plant(SecretKind::RunDataKey);
    let (id, value) = (canary.id().to_owned(), canary.value().to_owned());
    (canaries, id, value)
}

/// The replies of an Attached engine started with `variables`, to a `run.start`
/// of `run_id` and, when it started, its `run.finish`.
fn started(env: &TestEnv, variables: &[(&str, &str)], run_id: &str) -> Vec<Value> {
    let mut engine = Attached::start_with(env, |command| {
        for (name, value) in variables {
            command.env(name, value);
        }
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let mut replies = vec![call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": run_id }),
    )];
    if replies[0].get("result").is_some() {
        replies.push(call(
            &mut engine,
            2,
            "run.finish",
            &json!({ "run_id": run_id }),
        ));
    }
    let (rest, _) = engine.finish();
    replies.extend(rest);
    replies
}

/// The leaks of `canaries` in `replies` and in every file of `env`'s folder,
/// its logs and Goodfetch home included.
fn leaks(canaries: &Canaries, env: &TestEnv, replies: &[Value]) -> Vec<String> {
    let replies = serde_json::to_vec(replies).unwrap();
    canaries
        .leaks(&[
            Output::Bytes {
                name: "replies",
                bytes: &replies,
            },
            Output::Path(env.folder()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{} in {}", hit.canary, hit.output))
        .collect()
}

/// Covers: story:4-49, error:setting_invalid, setting:run_state.key_reference
#[test]
fn a_raw_value_as_the_key_reference_is_refused_with_setting_invalid_naming_no_value() {
    let env = TestEnv::new();
    let (mut canaries, id, value) = planted(&env);
    env.user_configuration(&format!("run_state.key_reference = \"{value}\"\n"));
    let configuration = env.home().join("0.0").join("configuration.toml");
    canaries.record_planted(&id, &configuration.display().to_string());

    let replies = started(&env, &[], "raw");

    assert_eq!(
        (
            &replies[0]["error"]["data"]["code"],
            &replies[0]["error"]["data"]["key"],
            leaks(&canaries, &env, &replies),
        ),
        (
            &json!("setting_invalid"),
            &json!("run_state.key_reference"),
            Vec::<String>::new()
        )
    );
}

/// Covers: story:4-49, setting:run_state.key_reference
#[test]
fn config_explain_shows_the_key_reference_never_its_value() {
    let env = TestEnv::new();
    let (canaries, _, value) = planted(&env);
    by_reference(&env);

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "run_state.key_reference", "--json"])
        .env(VARIABLE, &value)
        .run()
        .unwrap();

    let explained: Value = serde_json::from_str(&output.stdout).unwrap();
    let shown: Vec<&Value> = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|setting| setting["key"] == "run_state.key_reference")
        .map(|setting| &setting["value"])
        .collect();
    let printed = format!("{}{}", output.stdout, output.stderr);
    assert_eq!(
        (
            shown,
            canaries
                .leaks(&[Output::Bytes {
                    name: "config explain",
                    bytes: printed.as_bytes(),
                }])
                .unwrap()
        ),
        (vec![&json!(format!("env:{VARIABLE}"))], vec![])
    );
}

/// Covers: story:4-49, story:4-62, setting:run_state.key_reference
#[test]
fn a_run_resolves_its_key_reference_at_start_and_records_the_use_without_the_value() {
    let env = TestEnv::new();
    let (canaries, _, value) = planted(&env);
    by_reference(&env);

    let replies = started(&env, &[(VARIABLE, &value)], "keyed");

    let recorded: Vec<(String, Value)> = Connection::open(env.home().join("audit-log.sqlite"))
        .unwrap()
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| {
            let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
            (
                entry["kind"].as_str().unwrap().to_owned(),
                entry["details"].clone(),
            )
        })
        .collect();
    assert_eq!(
        (
            &replies[1]["result"]["lifecycle"],
            recorded,
            leaks(&canaries, &env, &replies)
        ),
        (
            &json!("finished"),
            vec![(
                "secret_reference_used".to_owned(),
                json!({
                    "placeholder": "run_state.key_reference",
                    "reference": format!("env:{VARIABLE}"),
                    "domain": null,
                })
            )],
            Vec::<String>::new()
        )
    );
}

/// Covers: story:4-60, error:secret_unavailable, setting:run_state.key_reference
#[test]
fn a_run_whose_key_variable_is_unset_is_refused_with_secret_unavailable_before_it_exists() {
    let env = TestEnv::new();
    by_reference(&env);

    let replies = started(&env, &[], "unkeyed");

    let mut data = replies[0]["error"]["data"].clone();
    let fix = data.as_object_mut().unwrap().remove("fix");
    assert_eq!(
        (
            data,
            fix.is_some_and(|fix| fix.is_string()),
            env.home().join("0.0").join("runs").join("unkeyed").exists()
        ),
        (
            json!({
                "code": "secret_unavailable",
                "placeholder": "run_state.key_reference",
                "reference": format!("env:{VARIABLE}"),
                "provider": "env",
                "failure": "missing",
                "retryable": false,
            }),
            true,
            false
        )
    );
}
