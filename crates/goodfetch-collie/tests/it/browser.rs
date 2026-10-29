//! `browser install` and `browser clean` through Collie's executable, on
//! the Browser cache of the test's Goodfetch home or of
//! `GOODFETCH_BROWSERS_PATH` (ADRs 0024, 0059 and 0098). Entries are laid
//! out as a prepared copy holds them, used long before or long after the
//! command's own wall time, so no test reads a clock.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out cache entries by hand, as a prepared GOODFETCH_BROWSERS_PATH copy holds them"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fs;
use std::path::Path;

use goodfetch_engine::browser_cache::Platform;
use goodfetch_protocol::browser_cache::{
    BrowserCleaned, BrowserInstalled, InstallOutcome, InstalledBuild,
};
use goodfetch_test_support::{
    TestEnv, assert_cli_snapshot, browser_cache, cli, lay_out_browser_entry,
};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

fn run(env: &TestEnv, arguments: &[&str], variables: &[(&str, &Path)]) -> Value {
    let command = variables.iter().fold(
        cli(env, COLLIE).args(arguments),
        |command, (name, value)| command.env(name, value),
    );
    let output = command.run().unwrap();
    assert_eq!(output.exit, Some(0), "{output}");
    serde_json::from_str(&output.stdout).unwrap()
}

/// The names of the builds `output` removed.
fn removed(output: &BrowserCleaned) -> Vec<&str> {
    output
        .removed
        .iter()
        .map(|build| build.version.as_str())
        .collect()
}

/// `browser install` makes sure of each build the version pins for this
/// platform, in the cache `GOODFETCH_BROWSERS_PATH` names, which already
/// holds them here: no test downloads a browser.
///
/// Covers: row:SDK-08, story:11-214, command:browser.install
#[test]
fn browser_install_makes_sure_of_each_build_the_version_pins() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();

    let checked = cli(&env, COLLIE)
        .args(["browser", "install", "--json"])
        .env("GOODFETCH_BROWSERS_PATH", &cache)
        .json("BrowserInstalled")
        .unwrap();

    let output: BrowserInstalled = serde_json::from_value(checked.documents[0].clone()).unwrap();
    let platform = Platform::current().build_platform();
    assert_eq!(
        output,
        BrowserInstalled {
            builds: goodfetch_collie::pinned_artifacts()
                .into_iter()
                .filter(|pin| pin.platform.is_none() || pin.platform == platform)
                .map(|pin| InstalledBuild {
                    name: pin.id.name,
                    version: pin.id.build,
                    outcome: InstallOutcome::AlreadyInstalled,
                })
                .collect(),
        }
    );
    let mut settings = checked.output.settings();
    settings.add_filter(r#""version": "[^"]+""#, r#""version": "[build]""#);
    settings.bind(|| assert_cli_snapshot!(checked.output));
}

/// `browser clean --dry-run` lists, with their bytes, the builds `browser
/// clean` then removes, and deletes nothing; a build an installed
/// executable of the other Product pins survives both, and so does one
/// used within `browser_cache.max_unused_age` (ADR 0098).
///
/// Covers: story:11-214, story:11-296, story:3-140, command:browser.clean
#[test]
fn browser_clean_dry_run_lists_what_browser_clean_then_removes() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out_browser_entry(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");
    let recent = lay_out_browser_entry(&cache, "used-in-2100", "2100-01-01T00:00:00Z");
    let pinned = lay_out_browser_entry(&cache, "pinned-by-foxhound", "2020-01-01T00:00:00Z");
    install_a_foxhound_executable(&env, "pinned-by-foxhound");

    let dry_run = cli(&env, COLLIE)
        .args(["browser", "clean", "--dry-run", "--json"])
        .json("BrowserCleaned")
        .unwrap();
    let kept_by_dry_run = unused.exists();
    let cleaned = cli(&env, COLLIE)
        .args(["browser", "clean", "--json"])
        .json("BrowserCleaned")
        .unwrap();

    let listed: BrowserCleaned = serde_json::from_value(dry_run.documents[0].clone()).unwrap();
    let done: BrowserCleaned = serde_json::from_value(cleaned.documents[0].clone()).unwrap();
    assert_eq!(
        (
            kept_by_dry_run,
            removed(&listed) == removed(&done),
            listed.bytes_freed == done.bytes_freed,
            unused.exists(),
            recent.join("entry.json").exists(),
            pinned.join("entry.json").exists(),
        ),
        (true, true, true, false, true, true)
    );
    assert_cli_snapshot!("browser_clean_dry_run", dry_run.output);
    assert_cli_snapshot!("browser_clean", cleaned.output);
}

/// Writes the list of installed executables of the test's home: a Foxhound
/// executable of this version seen in 2100, which still counts, pinning
/// `build`.
fn install_a_foxhound_executable(env: &TestEnv, build: &str) {
    let path = env.folder().join("goodfetch-foxhound");
    fs::write(&path, "#!foxhound\n").unwrap();
    let list = json!({
        "format_version": "0.1",
        "executables": [{
            "kind": "product_executable", "product": "foxhound",
            "product_version": env!("CARGO_PKG_VERSION"),
            "path": path.display().to_string(),
            "size": 11, "modified_at": "2020-01-01T00:00:00Z",
            "pins": [{ "kind": "browser", "name": "chrome-for-testing", "build": build }],
            "last_seen": "2100-01-01T00:00:00Z",
        }],
    });
    fs::create_dir_all(env.home()).unwrap();
    fs::write(
        env.home().join("installed-executables.json"),
        list.to_string(),
    )
    .unwrap();
}

/// Covers: story:3-140, command:browser.clean
#[test]
fn browser_clean_deletes_what_nothing_pins_or_used_within_the_age() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out_browser_entry(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");
    let recent = lay_out_browser_entry(&cache, "used-in-2100", "2100-01-01T00:00:00Z");

    let output = run(&env, &["browser", "clean", "--json"], &[]);

    let output: BrowserCleaned = serde_json::from_value(output).unwrap();
    assert_eq!(removed(&output), ["unused-since-2020"]);
    assert!(!unused.exists());
    assert!(recent.join("entry.json").exists());
}

/// Writes the list of installed executables of the test's home: a linked
/// program seen in 2100, gone from its path, and a Foxhound executable
/// last seen in 2020, gone too, each pinning one build.
fn install_two_executables(env: &TestEnv) {
    let entry = |kind: &str, build: &str, last_seen: &str| {
        json!({
            "kind": kind, "product": "foxhound", "product_version": "9.9.9",
            "path": env.folder().join(build).display().to_string(),
            "size": 1, "modified_at": "2020-01-01T00:00:00Z",
            "pins": [{ "kind": "browser", "name": "chrome-for-testing", "build": build }],
            "last_seen": last_seen,
        })
    };
    let list = json!({
        "format_version": "0.1",
        "executables": [
            entry("linked_program", "pinned-by-a-program", "2100-01-01T00:00:00Z"),
            entry("product_executable", "pinned-by-a-removed-foxhound", "2020-01-01T00:00:00Z"),
        ],
    });
    fs::create_dir_all(env.home()).unwrap();
    fs::write(
        env.home().join("installed-executables.json"),
        list.to_string(),
    )
    .unwrap();
}

/// A build is kept while an executable that counts as installed pins it,
/// even one only linking the engine, and deleted once that executable's
/// entry stops counting (ADR 0098).
///
/// Covers: story:3-139, story:3-140
#[test]
fn browser_clean_keeps_what_an_installed_executable_pins() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let kept = lay_out_browser_entry(&cache, "pinned-by-a-program", "2020-01-01T00:00:00Z");
    lay_out_browser_entry(
        &cache,
        "pinned-by-a-removed-foxhound",
        "2020-01-01T00:00:00Z",
    );
    install_two_executables(&env);

    let output = run(&env, &["browser", "clean", "--json"], &[]);

    let output: BrowserCleaned = serde_json::from_value(output).unwrap();
    assert_eq!(
        (removed(&output), kept.join("entry.json").exists()),
        (vec!["pinned-by-a-removed-foxhound"], true)
    );
}

/// `browser_cache.max_unused_age` set in user configuration reaches the
/// clean-up: a build unused since 2020 is kept under an age of about 300
/// years (ADR 0098).
///
/// Covers: story:3-136, setting:browser_cache.max_unused_age
#[test]
fn browser_clean_keeps_what_was_used_within_the_age_user_configuration_sets() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out_browser_entry(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");
    fs::create_dir_all(env.home().join("0.0")).unwrap();
    fs::write(
        env.home().join("0.0").join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.browser_cache]\nmax_unused_age = 9999999999999\n",
    )
    .unwrap();

    let output = run(&env, &["browser", "clean", "--json"], &[]);

    let output: BrowserCleaned = serde_json::from_value(output).unwrap();
    assert_eq!(output.removed, Vec::new());
    assert!(unused.join("entry.json").exists());
}

/// Covers: row:SDK-08
#[test]
fn goodfetch_browsers_path_wins_over_goodfetch_home() {
    let env = TestEnv::new();
    let copy = env.folder().join("prepared cache");
    let unused = lay_out_browser_entry(&copy, "unused-since-2020", "2020-01-01T00:00:00Z");

    let output = run(
        &env,
        &["browser", "clean", "--json"],
        &[("GOODFETCH_BROWSERS_PATH", &copy)],
    );

    let output: BrowserCleaned = serde_json::from_value(output).unwrap();
    assert_eq!(removed(&output), ["unused-since-2020"]);
    assert!(!unused.exists());
}

/// Covers: story:3-140
#[cfg(unix)]
#[test]
fn browser_clean_never_cleans_a_read_only_goodfetch_browsers_path() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let copy = env.folder().join("read-only cache");
    let unused = lay_out_browser_entry(&copy, "unused-since-2020", "2020-01-01T00:00:00Z");
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o555)).unwrap();

    let output = run(
        &env,
        &["browser", "clean", "--json"],
        &[("GOODFETCH_BROWSERS_PATH", &copy)],
    );
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(output["not_cleaned"], "cache_not_writable");
    assert_eq!(output["removed"], json!([]));
    assert!(unused.join("entry.json").exists());
}

/// A setting `browser install` and `browser clean` cannot take fails them
/// as every command's failure prints, `{ "error" }` under `--json`, and
/// exits by its code's row, `3` for `setting_invalid`, as `config explain`
/// does for the same setting (ADR 0069).
///
/// Covers: story:11-214, setting:browser_cache.max_unused_age, error:setting_invalid, exit:program
#[rstest::rstest]
/// Covers: command:browser.install
#[case::install("install")]
/// Covers: command:browser.clean
#[case::clean("clean")]
fn a_browser_command_fails_a_bad_setting_by_its_row(#[case] command: &str) {
    let env = TestEnv::new();
    fs::create_dir_all(env.home().join("0.0")).unwrap();
    fs::write(
        env.home().join("0.0").join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.browser_cache]\nmax_unused_age = \"soon\"\n",
    )
    .unwrap();

    let output = cli(&env, COLLIE)
        .args(["browser", command, "--json"])
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (output.exit, printed["error"]["code"].clone()),
        (Some(3), json!("setting_invalid")),
        "{output}"
    );
}
