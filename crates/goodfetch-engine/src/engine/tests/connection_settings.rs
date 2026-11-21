//! A Connection's own settings through the engine's public interface over
//! the in-memory network (ADR 0083): `hello`'s `settings` checked at the
//! connection scope, each Run it starts taking them beneath its own, and
//! `config.explain` showing what the Connection or a live Run has.

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::consent_walls::write;
use super::run::call;
use super::{attached_on, engine_with, hello};

/// A Collie `hello` whose Connection sets `settings`.
fn hello_setting(settings: Value) -> Value {
    let mut hello = hello("collie", "0.1");
    hello["params"]["settings"] = settings;
    hello
}

/// Each explained setting of `explained` as its key, value and layer.
fn keyed(explained: &Value) -> Vec<Value> {
    explained["result"]["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| json!([setting["key"], setting["value"], setting["layer"]]))
        .collect()
}

/// A setting a `hello` refuses reads in the form that `hello` declares,
/// and its details keep its key canonical (ADR 0099).
#[rstest]
#[case::typescript("typescript", "`timeout.firstByte` is not valid")]
#[case::python("python", "`timeout.first_byte` is not valid")]
#[case::rust("rust", "`timeout::FIRST_BYTE` is not valid")]
fn a_setting_a_hello_refuses_reads_in_its_declared_form(#[case] form: &str, #[case] spelled: &str) {
    let env = TestEnv::new();
    let mut said = hello_setting(json!({ "timeout": { "first_byte": "soon" } }));
    said["params"]["form"] = json!(form);

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[said]).await;
            client.next().await.unwrap()
        },
    );

    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains(spelled), "{refused}");
    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"]
        ),
        (&json!("setting_invalid"), &json!("timeout.first_byte")),
        "{refused}"
    );
}

/// A key no program's code sets refuses the `hello` that gives it, naming
/// where it is set, and the Connection opens no further: an Operator
/// policy's key is locked by its default, and the project file's own key
/// is not applicable.
///
/// Covers: story:11-72, error:setting_locked, error:setting_not_applicable
#[rstest]
#[case::operator_policy(
    json!({ "wall": { "denied_domains": ["a.test"] } }),
    "setting_locked",
    "wall.denied_domains",
    "Operator policy"
)]
#[case::project_file(
    json!({ "timeout": { "connect": 1_000 } }),
    "setting_not_applicable",
    "timeout.connect",
    "project file"
)]
fn a_hello_setting_a_key_no_programs_code_sets_is_refused(
    #[case] settings: Value,
    #[case] code: &str,
    #[case] key: &str,
    #[case] set_in: &str,
) {
    let env = TestEnv::new();

    let (refused, after) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello_setting(settings)]).await;
            let refused = client.next().await.unwrap();
            let after = client.next().await;
            (refused, after)
        },
    );

    let details = &refused["error"]["data"];
    assert_eq!(
        (
            &details["code"],
            &details["key"],
            details["fix"].as_str().unwrap().contains(set_in),
            after,
        ),
        (&json!(code), &json!(key), true, None),
        "{refused}"
    );
}

/// `timeout.run` is the Connection's alone: a Run's own is refused with
/// `setting_not_applicable` pointing at `connect()`, and no Run begins.
///
/// Covers: story:11-72, setting:timeout.run, error:setting_not_applicable
#[test]
fn a_runs_own_run_timeout_is_refused_before_it_begins() {
    let env = TestEnv::new();

    let started = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(
                &mut client,
                1,
                "run.start",
                json!({ "run_id": "timed", "settings": { "timeout": { "run": 60_000 } } }),
            )
            .await
        },
    );

    let details = &started["error"]["data"];
    assert_eq!(
        (
            &details["code"],
            &details["key"],
            details["fix"].as_str().unwrap().contains("`connect()`"),
            env.home().join("1").join("runs").join("timed").exists(),
        ),
        (
            &json!("setting_not_applicable"),
            &json!("timeout.run"),
            true,
            false
        ),
        "{started}"
    );
}

/// The three states of a Run's own settings over its Connection's: what
/// it leaves out it inherits, its `null` clears, its value sets; and a
/// live Run explains the settings it opened with.
///
/// Covers: story:11-70, story:11-72, operation:config.explain
#[test]
fn config_explain_shows_the_connections_settings_beneath_the_runs() {
    let env = TestEnv::new();
    let keys = json!(["checkpoint.interval", "timeout.navigation", "timeout.wait"]);

    let (connection, given, live) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client
                .send(&[hello_setting(json!({
                    "timeout": { "navigation": 1_000, "wait": 3_000 },
                    "checkpoint": { "interval": 60_000 },
                }))])
                .await;
            client.next().await;
            let connection = call(&mut client, 1, "config.explain", json!({ "keys": keys })).await;
            let given = call(
                &mut client,
                2,
                "config.explain",
                json!({
                    "keys": keys,
                    "settings": { "timeout": { "wait": 4_000 }, "checkpoint": { "interval": null } },
                }),
            )
            .await;
            call(
                &mut client,
                3,
                "run.start",
                json!({ "run_id": "set", "settings": { "timeout": { "wait": 5_000 } } }),
            )
            .await;
            let live = call(
                &mut client,
                4,
                "config.explain",
                json!({ "keys": keys, "run_id": "set" }),
            )
            .await;
            call(&mut client, 5, "run.finish", json!({ "run_id": "set" })).await;
            (connection, given, live)
        },
    );

    assert_eq!(
        (keyed(&connection), keyed(&given), keyed(&live)),
        (
            vec![
                json!(["checkpoint.interval", 60_000, "run"]),
                json!(["timeout.navigation", 1_000, "run"]),
                json!(["timeout.wait", 3_000, "run"]),
            ],
            vec![
                json!(["checkpoint.interval", null, "run"]),
                json!(["timeout.navigation", 1_000, "run"]),
                json!(["timeout.wait", 4_000, "run"]),
            ],
            vec![
                json!(["checkpoint.interval", 60_000, "run"]),
                json!(["timeout.navigation", 1_000, "run"]),
                json!(["timeout.wait", 5_000, "run"]),
            ],
        )
    );
}

/// `config.explain` naming a Run explains only a live one, with the
/// settings it opened with: a Run ID no live Run has fails with
/// `run_not_found`, one that is no Run ID with `run_id_invalid`, and one
/// given with values to resolve, which the Run would not read, with
/// `invalid_params`.
///
/// Covers: operation:config.explain, error:run_not_found, error:run_id_invalid
#[rstest]
#[case::not_live(json!({ "run_id": "ended" }), "run_not_found")]
#[case::not_a_run_id(json!({ "run_id": "not a Run ID" }), "run_id_invalid")]
#[case::with_run_values(
    json!({ "run_id": "ended", "settings": { "timeout": { "wait": 1000 } } }),
    "invalid_params"
)]
#[case::with_a_project(
    json!({ "run_id": "ended", "missing_project": "goodfetch.toml" }),
    "invalid_params"
)]
fn config_explain_naming_a_run_fails_on_anything_but_a_live_run_alone(
    #[case] params: Value,
    #[case] code: &str,
) {
    let env = TestEnv::new();

    let explained = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "config.explain", params).await
        },
    );

    assert_eq!(
        explained["error"]["data"]["code"],
        json!(code),
        "{explained}"
    );
}

/// A configuration-only key a program's code passes to `connect()` is
/// refused with `setting_not_applicable`, its fix naming the user
/// configuration and Operator policy files, while `config.explain` still
/// reports it with the value and layer of the file that set it.
///
/// Covers: story:11-71, error:setting_not_applicable, operation:config.explain
#[rstest]
/// Covers: setting:storage.low_free
#[case::storage(json!({ "storage": { "low_free": 1_000 } }), "storage.low_free", json!(1_000))]
/// Covers: setting:log.level
#[case::log(json!({ "log": { "level": "warn" } }), "log.level", json!("warn"))]
/// Covers: setting:daemon.idle_timeout
#[case::daemon(json!({ "daemon": { "idle_timeout": 1_000 } }), "daemon.idle_timeout", json!(1_000))]
/// Covers: setting:browser_cache.max_unused_age
#[case::browser_cache(
    json!({ "browser_cache": { "max_unused_age": 1_000 } }),
    "browser_cache.max_unused_age",
    json!(1_000)
)]
/// Covers: setting:governor.huge_pages
#[case::huge_pages(json!({ "governor": { "huge_pages": true } }), "governor.huge_pages", json!(true))]
/// Covers: setting:governor.worker_threads
#[case::worker_threads(
    json!({ "governor": { "worker_threads": 4 } }),
    "governor.worker_threads",
    json!(4)
)]
/// Covers: setting:governor.blocking_threads
#[case::blocking_threads(
    json!({ "governor": { "blocking_threads": "per_core" } }),
    "governor.blocking_threads",
    json!("per_core")
)]
/// Covers: setting:usage.ledger_read_interval
#[case::usage(
    json!({ "usage": { "ledger_read_interval": 1_000 } }),
    "usage.ledger_read_interval",
    json!(1_000)
)]
fn a_configuration_only_key_in_connect_is_refused_and_still_explained(
    #[case] settings: Value,
    #[case] key: &'static str,
    #[case] value: Value,
) {
    let env = TestEnv::new();
    write(
        &env,
        "configuration.toml",
        &format!("version = \"0.1\"\n\n[settings]\n{key} = {value}\n"),
    );

    let engine = engine_with(Product::Collie, None);
    let refused = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello_setting(settings)]).await;
        client.next().await.unwrap()
    });
    let explained = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(&mut client, 1, "config.explain", json!({ "keys": [key] })).await
    });

    let details = &refused["error"]["data"];
    let fix = details["fix"].as_str().unwrap_or_default();
    let folder = env.home().join("1");
    assert_eq!(
        (
            &details["code"],
            &details["key"],
            ["configuration.toml", "operator-policy.toml"]
                .map(|file| fix.contains(&folder.join(file).display().to_string())),
            keyed(&explained),
        ),
        (
            &json!("setting_not_applicable"),
            &json!(key),
            [true, true],
            vec![json!([key, value, "user_configuration"])],
        ),
        "{refused}"
    );
}

/// Each setting `connect()` refuses says how to fix it: a misspelled key
/// names its closest neighbour, a value of the wrong type the type the
/// setting takes, a key the Operator policy locked the layer that locked
/// it, and a Foxhound key on a Collie engine the Product it needs.
///
/// Covers: story:11-76
#[rstest]
/// Covers: error:setting_unknown
#[case::misspelled(
    json!({ "timeout": { "navigations": 1_000 } }),
    "setting_unknown",
    "/closest",
    json!("timeout.navigation")
)]
/// Covers: error:setting_invalid
#[case::wrong_type(
    json!({ "timeout": { "wait": "soon" } }),
    "setting_invalid",
    "/reason",
    json!("expected a whole number of milliseconds, found `\"soon\"`")
)]
/// Covers: error:setting_locked
#[case::locked(
    json!({ "timeout": { "navigation": 1_000 } }),
    "setting_locked",
    "/limit/layer",
    json!("operator_policy")
)]
/// Covers: error:not_in_product
#[case::foxhound_key(
    json!({ "crawl": { "retry": { "max": 5 } } }),
    "not_in_product",
    "/needs",
    json!("foxhound")
)]
fn a_setting_connect_refuses_says_how_to_fix_it(
    #[case] settings: Value,
    #[case] code: &str,
    #[case] detail: &str,
    #[case] says: Value,
) {
    let env = TestEnv::new();
    // The lock the `locked` case meets; the others set no key it holds.
    write(
        &env,
        "operator-policy.toml",
        "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
    );

    let refused = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello_setting(settings)]).await;
            client.next().await.unwrap()
        },
    );

    let details = &refused["error"]["data"];
    assert_eq!(
        (&details["code"], details.pointer(detail)),
        (&json!(code), Some(&says)),
        "{refused}"
    );
}
