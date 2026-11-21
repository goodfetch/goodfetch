//! What only Foxhound carries, through Collie's executable (ADRs 0069, 0087,
//! 0090 and 0094): each operation's command, each command only Foxhound's
//! executable has, each setting key and each value of a shared type a
//! command takes is refused with `not_in_product` and exits `10`,
//! table-driven from the generated table of Foxhound's names; the `fix`
//! names the install command of the channel the executable came from; and a
//! configuration file holding Foxhound's keys loads, `config explain`
//! listing them as Foxhound's while a key neither Product knows stays
//! `setting_unknown`.
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the project configuration it loads itself"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::SharedType;
use goodfetch_protocol::entries::product_of_value;
use goodfetch_protocol::foxhound_names;
use goodfetch_test_support::{SnapshotValues, TestEnv, cli, command, snapshot_settings};
use rstest::rstest;
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// What a refused command answered: its Error code, what it names as
/// asked and its exit code.
type Refusal = (Value, Value, Option<i32>);

/// How a row's command line runs: through the CLI helper, which records the
/// command for the exercise audit (ADR 0072), or, for a command line that
/// names no command of Collie's executable, which the audit would find
/// unmatched, as a plain process.
#[derive(Clone, Copy)]
enum Through {
    Helper,
    Process,
}

/// Runs Collie's executable on `arguments` in `env` with the extra
/// variables `variables`, unrecorded: its stdout, stderr and exit code.
fn unrecorded(
    env: &TestEnv,
    arguments: &[&str],
    variables: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let mut process = command(env, COLLIE);
    process.args(arguments);
    for (name, value) in variables {
        process.env(name, value);
    }
    let output = process.output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status.code(),
    )
}

fn refusal(env: &TestEnv, arguments: &[String], through: Through) -> Refusal {
    let (stdout, exit) = match through {
        Through::Helper => {
            let output = cli(env, COLLIE)
                .args(arguments)
                .arg("--json")
                .run()
                .unwrap();
            (output.stdout, output.exit)
        }
        Through::Process => {
            let arguments: Vec<&str> = arguments
                .iter()
                .map(String::as_str)
                .chain(["--json"])
                .collect();
            let (stdout, _, exit) = unrecorded(env, &arguments, &[]);
            (stdout, exit)
        }
    };
    let printed: Value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (
        printed["error"]["code"].clone(),
        printed["error"]["requested"].clone(),
        exit,
    )
}

/// One row per name: the command line that names it and the refusal
/// expected, `not_in_product` naming it, exit `10`.
fn refused_rows(env: &TestEnv, rows: &[(String, Vec<String>)], through: Through) {
    let answered: Vec<(String, Refusal)> = rows
        .iter()
        .map(|(name, arguments)| (name.clone(), refusal(env, arguments, through)))
        .collect();
    let expected: Vec<(String, Refusal)> = rows
        .iter()
        .map(|(name, _)| {
            (
                name.clone(),
                (json!("not_in_product"), json!(name), Some(10)),
            )
        })
        .collect();
    assert_eq!(answered, expected);
}

/// The command line's spelling of a catalog word (ADR 0099).
fn spelled(word: &str) -> String {
    word.replace('_', "-")
}

/// The command of each operation only Foxhound carries, and each command
/// only Foxhound's executable has, through Collie's executable, which
/// refuses them before any engine starts.
///
/// Covers: story:11-216
#[test]
fn each_command_only_foxhound_carries_is_not_in_product_and_exits_10() {
    let env = TestEnv::new();
    let operations = foxhound_names::OPERATIONS.iter().map(|name| {
        let words = name.split('.').map(spelled).collect();
        ((*name).to_owned(), words)
    });
    // A command generated from an operation is that operation's row, and
    // the refusal names the operation.
    let generated = |path: &str| {
        foxhound_names::OPERATIONS
            .iter()
            .any(|name| name.split('.').map(spelled).collect::<Vec<_>>().join(".") == path)
    };
    let commands = foxhound_names::COMMANDS
        .iter()
        .filter(|path| !generated(path))
        .map(|path| {
            let words = path.split('.').map(str::to_owned).collect();
            ((*path).to_owned(), words)
        });

    refused_rows(
        &env,
        &operations.chain(commands).collect::<Vec<_>>(),
        Through::Process,
    );
}

/// A namespace only Foxhound carries, given alone, names Foxhound too:
/// `crawl` is not a usage error.
///
/// Covers: story:11-216
#[test]
fn a_namespace_only_foxhound_carries_is_not_in_product() {
    let env = TestEnv::new();

    refused_rows(
        &env,
        &[("crawl".to_owned(), vec!["crawl".to_owned()])],
        Through::Process,
    );
}

/// Each setting key only Foxhound honours, given to a command that takes
/// settings, on an Attached engine of Collie's executable.
///
/// Covers: story:11-216, command:run.start, error:not_in_product, exit:not_in_product
#[test]
fn each_setting_key_only_foxhound_honours_is_not_in_product_and_exits_10() {
    let env = TestEnv::new();
    let rows: Vec<(String, Vec<String>)> = foxhound_names::SETTINGS
        .iter()
        .map(|key| {
            let arguments = ["run", "start", "--attached", "--set", &format!("{key}=1")]
                .map(str::to_owned)
                .to_vec();
            ((*key).to_owned(), arguments)
        })
        .collect();

    refused_rows(&env, &rows, Through::Helper);
}

/// Each value of a shared type a command takes (a Usage kind, a Limit
/// scope) that only Foxhound carries, given to `usage set`.
///
/// Covers: story:11-216, command:usage.set, error:not_in_product, exit:not_in_product
#[test]
fn each_shared_type_value_only_foxhound_carries_is_not_in_product_and_exits_10() {
    let env = TestEnv::new();
    let taken = |shared_type: SharedType| {
        foxhound_names::SHARED_TYPE_VALUES
            .iter()
            .filter(move |name| product_of_value(shared_type, name).is_some())
    };
    let usage_set = |scope: &str, usage_kind: &str| {
        [
            "usage",
            "set",
            "--attached",
            "--scope",
            scope,
            "--usage-kind",
            usage_kind,
            "--amount",
            "1",
        ]
        .map(str::to_owned)
        .to_vec()
    };
    let rows: Vec<(String, Vec<String>)> = taken(SharedType::UsageKind)
        .map(|kind| ((*kind).to_owned(), usage_set("install", kind)))
        .chain(
            taken(SharedType::LimitScope)
                .map(|scope| ((*scope).to_owned(), usage_set(scope, "pages"))),
        )
        .collect();
    assert!(!rows.is_empty(), "the table names a value a command takes");

    refused_rows(&env, &rows, Through::Helper);
}

/// The refusal a person reads: its message, then `code`, `retryable` and
/// `fix` on lines of their own, on stderr only.
///
/// Covers: story:11-216
#[test]
fn a_foxhound_command_tells_a_person_to_install_foxhound() {
    let env = TestEnv::new();

    let (stdout, stderr, exit) = unrecorded(&env, &["crawl", "start"], &[]);

    assert_eq!((exit, stdout.as_str()), (Some(10), ""), "{stderr}");
    insta::assert_snapshot!("foxhound_command_human", stderr);
}

/// The refusal a person reads of a setting key and of a value of a shared
/// type only Foxhound carries, given to commands of Collie's executable on
/// an Attached engine: its message naming what was asked, then `code`,
/// `retryable` and `fix` on lines of their own, on stderr only.
///
/// Covers: story:11-216, error:not_in_product, exit:not_in_product
#[rstest]
/// Covers: command:run.start
#[case::setting_key(
    "foxhound_setting_key_human",
    &["run", "start", "--attached", "--set", "crawl.max_depth=1"],
)]
/// Covers: command:usage.set
#[case::shared_type_value(
    "foxhound_shared_type_value_human",
    &["usage", "set", "--attached", "--scope", "install", "--usage-kind", "paid_fetch", "--amount", "1"],
)]
fn a_foxhound_setting_key_or_shared_type_value_tells_a_person_to_install_foxhound(
    #[case] name: &str,
    #[case] arguments: &[&str],
) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE).args(arguments).run().unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(10), ""),
        "{}",
        output.stderr
    );
    // The Attached engine's dispatch records, which the test's log level
    // turns on for the exercise audit, are no part of the refusal.
    let refusal = output
        .stderr
        .lines()
        .filter(|line| !line.contains(" TRACE goodfetch::"))
        .collect::<Vec<_>>()
        .join("\n");
    snapshot_settings(
        &SnapshotValues::of(&env).with_executable(env!("CARGO_PKG_VERSION"), "no-commit"),
    )
    .bind(|| insta::assert_snapshot!(name, refusal));
}

/// The `fix` names the install command of the channel the executable came
/// from, which its channel's launcher or image says in `GOODFETCH_CHANNEL`;
/// a build from source came from crates.io.
///
/// Covers: story:11-216
#[rstest]
#[case::npm(Some("npm"), "npm install @goodfetch/foxhound")]
#[case::pypi(Some("pypi"), "pip install goodfetch-foxhound")]
#[case::crates_io(
    Some("crates-io"),
    "cargo install goodfetch-foxhound --locked --features executable"
)]
#[case::from_source(
    None,
    "cargo install goodfetch-foxhound --locked --features executable"
)]
#[case::homebrew(Some("homebrew"), "brew install goodfetch/tap/goodfetch-foxhound")]
// The installer is a PowerShell script on Windows, a shell script elsewhere.
#[case::archive(
    Some("installer"),
    if cfg!(windows) {
        "irm https://goodfetch.dev/install/foxhound.ps1 | iex"
    } else {
        "curl -fsSL https://goodfetch.dev/install/foxhound.sh | sh"
    }
)]
#[case::image(Some("oci"), "docker pull ghcr.io/goodfetch/foxhound")]
fn the_fix_names_the_install_command_of_the_executables_channel(
    #[case] channel: Option<&str>,
    #[case] install: &str,
) {
    let env = TestEnv::new();
    let variables: Vec<(&str, &str)> = channel
        .map(|channel| ("GOODFETCH_CHANNEL", channel))
        .into_iter()
        .collect();

    let (stdout, _, _) = unrecorded(&env, &["crawl", "start", "--json"], &variables);

    let printed: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        printed["error"]["fix"],
        json!(format!(
            "Run it with Foxhound, which contains everything Collie does (`{install}`), or connect to a Foxhound engine."
        ))
    );
}

/// A channel the executable does not know stops it before its command, as
/// any other deployment variable's bad value does (ADR 0083).
#[test]
fn an_unknown_channel_stops_the_executable_before_its_command() {
    let env = TestEnv::new();

    let (stdout, stderr, exit) =
        unrecorded(&env, &["crawl", "start"], &[("GOODFETCH_CHANNEL", "snap")]);

    assert_eq!(
        (exit, stdout.as_str(), stderr.contains("setting_invalid")),
        (Some(3), "", true),
        "{stderr}"
    );
}

/// One configuration file serves both executables: Foxhound's keys in user
/// configuration and in a project configuration load in Collie's, which
/// lists each as Foxhound's and applies none.
///
/// Covers: story:11-217, command:config.explain, setting:crawl.max_depth, setting:politeness.domain.delay
#[test]
fn config_explain_lists_foxhounds_keys_of_a_shared_file_as_foxhounds() {
    let env = TestEnv::new();
    env.user_configuration("crawl.max_depth = 3\n");
    std::fs::write(
        env.folder().join("goodfetch.toml"),
        "version = \"0.1\"\n\n[settings.politeness.domain]\ndelay = 1000\n",
    )
    .unwrap();

    let output = cli(&env, COLLIE)
        .args([
            "config",
            "explain",
            "crawl.max_depth",
            "politeness.domain.delay",
            "--json",
        ])
        .run()
        .unwrap();

    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    let printed: Value = serde_json::from_str(&output.stdout).unwrap();
    let listed: Vec<(Value, Value, Value, Value, Value)> = printed["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            (
                setting["key"].clone(),
                setting["value"].clone(),
                setting["layer"].clone(),
                setting["products"].clone(),
                setting["applied"].clone(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            (
                json!("crawl.max_depth"),
                json!(3),
                json!("user_configuration"),
                json!(["foxhound"]),
                json!(false)
            ),
            (
                json!("politeness.domain.delay"),
                json!(1000),
                json!("project_configuration"),
                json!(["foxhound"]),
                json!(false)
            ),
        ]
    );
}

/// A key neither Product knows stays `setting_unknown`, naming the closest
/// key, and exits with its group's row.
///
/// Covers: story:11-217, command:config.explain, error:setting_unknown
#[test]
fn a_key_neither_product_knows_is_setting_unknown() {
    let env = TestEnv::new();
    env.user_configuration("crawl.max_deepth = 3\n");

    let output = cli(&env, COLLIE).args(["config", "explain"]).run().unwrap();

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            output.stderr.contains("setting_unknown"),
            output.stderr.contains("`crawl.max_depth`"),
        ),
        (Some(3), "", true, true),
        "{}",
        output.stderr
    );
}
