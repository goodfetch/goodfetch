//! Settings through Collie's executable (spec #213's Seam 1): `config
//! explain` from the test's Goodfetch home, the project configuration of
//! its folder and the CLI's flags, in human and `--json` output; the
//! strict environment map; machine settings that cannot be read, which
//! fail every command that needs them; and every key only Foxhound honours
//! refused with `not_in_product` when given explicitly, table-driven from
//! the generated table of Foxhound's names (ADRs 0028, 0083 and 0087).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's and the project's settings files themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fs;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound_names;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{SnapshotValues, TestEnv, cli, command, snapshot_settings};
use rstest::rstest;
use serde_json::{Value, json};

use crate::daemon::StopsTheDaemon;
use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// On machine settings it cannot read, `config explain` fails before any
/// engine starts: it prints its failure as a settings file's refusal prints,
/// naming no operation, and nothing an engine's start logs or writes reaches
/// the person looking for that fault.
///
/// Covers: story:11-217, command:config.explain, error:setting_unknown
#[test]
fn config_explain_on_unreadable_settings_starts_no_engine() {
    let env = TestEnv::new();
    env.user_configuration("crawl.max_deepth = 3\n");

    let output = cli(&env, COLLIE)
        .args(["config", "explain", "--json"])
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (
            output.exit,
            printed["error"]["code"].clone(),
            printed["error"].get("operation"),
            output.stderr.as_str(),
        ),
        (Some(3), json!("setting_unknown"), None, ""),
        "{output}"
    );
}

/// The committed JSON Schema bundle, whose `config.explain` result is
/// `config explain --json`'s output.
const BUNDLE: &str = include_str!("../../../goodfetch-protocol/generated/bundle.json");

/// How a test reaches the engine: the executable itself, as an Attached
/// engine does, or the major's Daemon, which `--daemon` starts.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Attached,
    Daemon,
}

/// Lays out settings at three layers: an Operator policy that locks one
/// key, user configuration that sets one key and one only Foxhound honours,
/// and a project configuration that sets one key again and a second.
fn lay_out(env: &TestEnv) {
    let layout = HomeLayout::of(env.home());
    fs::create_dir_all(layout.per_major()).unwrap();
    fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.network.resolve]\n\"staging.example\" = [\"127.0.0.1\"]\n",
    )
    .unwrap();
    env.user_configuration("timeout.navigation = 45000\ncrawl.retry.max = 5\n");
    fs::write(
        env.folder().join("goodfetch.toml"),
        "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 60000\nfirst_byte = 10000\n",
    )
    .unwrap();
}

fn explain(env: &TestEnv, mode: Mode, arguments: &[&str]) -> (String, String, bool) {
    let command = match mode {
        Mode::Attached => cli(env, COLLIE).args(["config", "explain"]).args(arguments),
        Mode::Daemon => cli(env, COLLIE)
            .args(["config", "explain"])
            .args(arguments)
            .arg("--daemon"),
    };
    let output = command.run().unwrap();
    (output.stdout, output.stderr, output.exit == Some(0))
}

/// The shared snapshot filters, and one for the working folder as the OS
/// resolves it: on macOS the temporary folder sits under `/private`, which a
/// symbolic link hides from the test's own path.
fn settings(env: &TestEnv) -> insta::Settings {
    let mut settings = snapshot_settings(&SnapshotValues::of(env));
    settings.add_filter(r"/private\[FOLDER\]", "[FOLDER]");
    settings
}

fn validate(output: &Value) {
    let bundle: Value = serde_json::from_str(BUNDLE).unwrap();
    let schema = json!({
        "$ref": "#/$defs/SettingsExplanation",
        "$defs": bundle["$defs"],
    });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(output)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(errors, Vec::<String>::new());
}

/// Covers: story:3-89, story:3-142, command:config.explain
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn config_explain_json_shows_each_value_with_the_layer_that_set_and_locked_it(#[case] mode: Mode) {
    let env = TestEnv::new();
    lay_out(&env);
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let (stdout, _, succeeded) = explain(
        &env,
        mode,
        &[
            "timeout",
            "network.resolve",
            "crawl",
            "--set",
            "timeout.run=3600000",
            "--json",
        ],
    );

    assert!(succeeded);
    assert_eq!(stdout.lines().count(), 1, "stdout holds one JSON line only");
    let output: Value = serde_json::from_str(&stdout).unwrap();
    validate(&output);
    settings(&env).bind(|| {
        insta::assert_snapshot!(
            "config_explain_json",
            serde_json::to_string_pretty(&output).unwrap()
        );
    });
}

/// A `--project` that names no file sets nothing: the answer lists it as
/// not found.
/// Covers: story:3-89, command:config.explain
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn config_explain_lists_a_named_project_file_that_is_missing_as_not_found(#[case] mode: Mode) {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let missing = env.folder().join("no-such-project.toml");
    let missing = missing.to_str().unwrap();

    let (stdout, _, succeeded) = explain(
        &env,
        mode,
        &["timeout.navigation", "--project", missing, "--json"],
    );

    assert!(succeeded);
    let output: Value = serde_json::from_str(&stdout).unwrap();
    let project: Vec<&Value> = output["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|file| file["layer"] == "project_configuration")
        .collect();
    assert_eq!(
        project,
        [&json!({
            "layer": "project_configuration",
            "path": missing,
            "found": false,
        })]
    );
}

/// `governor.huge_pages`, read once as the engine starts, shows off by
/// default (issue 2199).
/// Covers: command:config.explain
#[test]
fn config_explain_shows_huge_pages_off_by_default() {
    let env = TestEnv::new();

    let (stdout, _, succeeded) = explain(&env, Mode::Attached, &["governor.huge_pages", "--json"]);

    assert!(succeeded);
    let output: Value = serde_json::from_str(&stdout).unwrap();
    let shown: Vec<(&Value, &Value, &Value)> = output["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| (&setting["key"], &setting["layer"], &setting["value"]))
        .collect();
    assert_eq!(
        shown,
        [(
            &json!("governor.huge_pages"),
            &json!("default"),
            &json!(false)
        )]
    );
}

/// Covers: story:3-89, command:config.explain
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn config_explain_shows_one_line_per_key_with_its_origin(#[case] mode: Mode) {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    lay_out(&env);

    let (stdout, _, succeeded) = explain(&env, mode, &["timeout", "network.resolve", "crawl"]);

    assert!(succeeded);
    settings(&env).bind(|| {
        insta::assert_snapshot!("config_explain_human", stdout);
    });
}

/// ADR 0118's two Happy Eyeballs delays are settings of both Products,
/// RFC 8305's values built in, which a layer may change.
///
/// Covers: story:3-148, command:config.explain, setting:resolver.resolution_delay, setting:resolver.attempt_delay
#[test]
fn config_explain_shows_the_happy_eyeballs_delays() {
    let env = TestEnv::new();
    let layout = HomeLayout::of(env.home());
    fs::create_dir_all(layout.per_major()).unwrap();
    fs::write(
        layout.user_configuration(),
        "version = \"0.1\"\n\n[settings.resolver]\nattempt_delay = 100\n",
    )
    .unwrap();

    let (stdout, _, succeeded) = explain(&env, Mode::Attached, &["resolver", "--json"]);

    assert!(succeeded);
    let output: Value = serde_json::from_str(&stdout).unwrap();
    let delays: Vec<Value> = output["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            json!([
                setting["key"],
                setting["value"],
                setting["layer"],
                setting["products"]
            ])
        })
        .collect();
    assert_eq!(
        delays,
        [
            json!([
                "resolver.attempt_delay",
                100,
                "user_configuration",
                ["collie", "foxhound"]
            ]),
            json!([
                "resolver.resolution_delay",
                50,
                "default",
                ["collie", "foxhound"]
            ]),
        ]
    );
}

/// `content.extractor` is `dom_smoothie` by default, and `config explain`
/// names the layer that chose another: the project configuration's, then
/// a flag's over it (#788).
///
/// Covers: story:6-11, command:config.explain, setting:content.extractor
#[test]
fn config_explain_names_the_layer_that_chose_the_extractor() {
    let extractor = |env: &TestEnv, arguments: &[&str]| {
        let (stdout, stderr, succeeded) = explain(
            env,
            Mode::Attached,
            &[&["content.extractor", "--json"], arguments].concat(),
        );
        assert!(succeeded, "{stderr}");
        let output: Value = serde_json::from_str(&stdout).unwrap();
        output["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|setting| json!([setting["key"], setting["value"], setting["layer"]]))
            .collect::<Vec<_>>()
    };
    let by_default = TestEnv::new();
    let projected = TestEnv::new();
    fs::write(
        projected.folder().join("goodfetch.toml"),
        "version = \"0.1\"\n\n[settings.content]\nextractor = \"trafilatura\"\n",
    )
    .unwrap();

    assert_eq!(
        (
            extractor(&by_default, &[]),
            extractor(&projected, &[]),
            extractor(&projected, &["--set", "content.extractor=dom_smoothie"]),
        ),
        (
            vec![json!(["content.extractor", "dom_smoothie", "default"])],
            vec![json!([
                "content.extractor",
                "trafilatura",
                "project_configuration"
            ])],
            vec![json!(["content.extractor", "dom_smoothie", "run"])],
        )
    );
}

/// An extractor no built-in has is refused as any invalid setting value
/// is, naming the setting (#788).
///
/// Covers: story:6-11, setting:content.extractor, error:setting_invalid
#[test]
fn an_unknown_extractor_is_an_invalid_setting_value() {
    let env = TestEnv::new();

    let (stdout, stderr, succeeded) = explain(
        &env,
        Mode::Attached,
        &["--set", "content.extractor=readability"],
    );

    assert_eq!(
        (
            stdout.as_str(),
            stderr.contains("setting_invalid") && stderr.contains("content.extractor"),
            succeeded
        ),
        ("", true, false),
        "{stderr}"
    );
}

/// `extraction.readability` is `retry` by default, and `config explain`
/// names the layer that set it, the Operator policy's lock included; a
/// value it does not take is refused, naming it (spec #2164).
///
/// Covers: command:config.explain, setting:extraction.readability, error:setting_invalid
#[test]
fn config_explain_shows_the_readability_attempts_and_the_layer_that_set_or_locked_them() {
    let readability = |env: &TestEnv, arguments: &[&str]| {
        let (stdout, stderr, succeeded) = explain(
            env,
            Mode::Attached,
            &[&["extraction.readability", "--json"], arguments].concat(),
        );
        assert!(succeeded, "{stderr}");
        let output: Value = serde_json::from_str(&stdout).unwrap();
        output["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|setting| {
                json!([
                    setting["key"],
                    setting["value"],
                    setting["layer"],
                    setting["locked_by"]
                ])
            })
            .collect::<Vec<_>>()
    };
    let by_default = TestEnv::new();
    let projected = TestEnv::new();
    fs::write(
        projected.folder().join("goodfetch.toml"),
        "version = \"0.1\"\n\n[settings.extraction]\nreadability = \"single_attempt\"\n",
    )
    .unwrap();
    let locked = TestEnv::new();
    let layout = HomeLayout::of(locked.home());
    fs::create_dir_all(layout.per_major()).unwrap();
    fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.extraction]\nreadability = \"single_attempt\"\n",
    )
    .unwrap();

    let (refused, stderr, succeeded) = explain(
        &by_default,
        Mode::Attached,
        &["--set", "extraction.readability=once"],
    );

    assert_eq!(
        (
            readability(&by_default, &[]),
            readability(&projected, &[]),
            readability(&projected, &["--set", "extraction.readability=retry"]),
            readability(&locked, &[]),
        ),
        (
            vec![json!(["extraction.readability", "retry", "default", null])],
            vec![json!([
                "extraction.readability",
                "single_attempt",
                "project_configuration",
                null
            ])],
            vec![json!(["extraction.readability", "retry", "run", null])],
            vec![json!([
                "extraction.readability",
                "single_attempt",
                "operator_policy",
                "operator_policy"
            ])],
        )
    );
    assert_eq!(
        (
            refused.as_str(),
            stderr.contains("setting_invalid") && stderr.contains("extraction.readability"),
            succeeded
        ),
        ("", true, false),
        "{stderr}"
    );
}

/// `browser.headless` is Collie's, `true` by default, and `config explain`
/// names the layer that set it: the default, then user configuration's
/// `false` (spec #577).
///
/// Covers: story:5-56, command:config.explain, setting:browser.headless
#[test]
fn config_explain_shows_browser_headless_true_by_default_and_the_layer_that_set_it() {
    let headless = |env: &TestEnv| {
        let (stdout, _, succeeded) = explain(env, Mode::Attached, &["browser.headless", "--json"]);
        assert!(succeeded, "{stdout}");
        let output: Value = serde_json::from_str(&stdout).unwrap();
        output["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|setting| {
                json!([
                    setting["key"],
                    setting["value"],
                    setting["layer"],
                    setting["products"]
                ])
            })
            .collect::<Vec<_>>()
    };
    let by_default = TestEnv::new();
    let configured = TestEnv::new();
    let layout = HomeLayout::of(configured.home());
    fs::create_dir_all(layout.per_major()).unwrap();
    fs::write(
        layout.user_configuration(),
        "version = \"0.1\"\n\n[settings.browser]\nheadless = false\n",
    )
    .unwrap();

    assert_eq!(
        (headless(&by_default), headless(&configured)),
        (
            vec![json!([
                "browser.headless",
                true,
                "default",
                ["collie", "foxhound"]
            ])],
            vec![json!([
                "browser.headless",
                false,
                "user_configuration",
                ["collie", "foxhound"]
            ])],
        )
    );
}

/// Machine settings that cannot be read fail every command that needs
/// them with their Error code, printed as every command's failure is and
/// exiting its row, before it does anything: no Run index is rebuilt and
/// no Daemon starts on them, as `browser install` and `browser clean` fail
/// (`browser.rs`).
///
/// Covers: error:setting_invalid, exit:program
#[rstest]
/// Covers: command:runs.reindex
#[case::runs_reindex(&["runs", "reindex", "--json"])]
/// Covers: command:daemon.start
#[case::daemon_start(&["daemon", "start", "--json"])]
fn a_command_that_needs_the_machines_settings_fails_with_their_error_code_when_they_cannot_be_read(
    #[case] arguments: &[&str],
) {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    env.user_configuration("timeout.navigation = \"soon\"\n");

    let output = cli(&env, COLLIE).args(arguments).run().unwrap();

    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or_default();
    assert_eq!(
        (output.exit, printed["error"]["code"].clone()),
        (Some(3), json!("setting_invalid")),
        "{}",
        output.stderr
    );
}

/// Covers: story:3-142, error:not_in_product
#[test]
fn a_foxhound_key_given_as_a_flag_is_not_in_product() {
    let env = TestEnv::new();

    let (stdout, stderr, succeeded) =
        explain(&env, Mode::Attached, &["--set", "crawl.retry.max=1"]);

    assert_eq!(
        (
            stdout.as_str(),
            stderr.contains("not_in_product"),
            succeeded
        ),
        ("", true, false)
    );
}

/// Covers: story:3-133, error:setting_unknown
#[test]
fn an_unknown_goodfetch_variable_stops_the_executable_before_its_command() {
    let env = TestEnv::new();
    let mut command = command(&env, COLLIE);
    command
        .args(["config", "explain"])
        .env("GOODFETCH_HOMES", env.home());

    let output = command.output().unwrap();

    assert_eq!(
        (
            output.stdout.is_empty(),
            String::from_utf8_lossy(&output.stderr).contains("setting_unknown"),
            output.status.success()
        ),
        (true, true, false)
    );
}

/// A Run's `settings` holding each key only Foxhound honours, through an
/// Attached engine of Collie's executable: the product check comes before
/// the scope check, so each answers `not_in_product` (ADR 0087). The table
/// is the generated one of Foxhound's names; the expected answer is one
/// Error code for every row.
///
/// Covers: story:3-142, error:not_in_product
#[test]
fn every_key_only_foxhound_honours_is_not_in_product_on_collies_engine() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let mut answers = Vec::new();
    for (id, key) in (1_i64..).zip(foxhound_names::SETTINGS) {
        engine.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "run.start",
            "params": { "run_id": "refused", "settings": { *key: 1 } },
        }));
        let reply = engine.next().unwrap();
        answers.push((
            (*key).to_owned(),
            reply["error"]["data"]["code"].clone(),
            reply["error"]["data"]["requested"].clone(),
        ));
    }
    let _ = engine.finish();

    let expected: Vec<(String, Value, Value)> = foxhound_names::SETTINGS
        .iter()
        .map(|key| ((*key).to_owned(), json!("not_in_product"), json!(key)))
        .collect();
    assert_eq!(answers, expected);
}

/// The message of a Foxhound key's refusal, once (TESTING.md, CLI).
///
/// Covers: error:not_in_product
#[test]
fn a_foxhound_keys_refusal_names_foxhound_and_its_install() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    engine.send(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "run.start",
        "params": { "run_id": "refused", "settings": { "crawl": { "retry": { "max": 1 } } } },
    }));
    let reply = engine.next().unwrap();
    let _ = engine.finish();

    insta::assert_snapshot!(
        "foxhound_key_not_in_product",
        serde_json::to_string_pretty(&reply["error"]).unwrap()
    );
}
