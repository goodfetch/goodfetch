//! `browsers install` and `browsers clean` through Collie's executable, on
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
use std::path::{Path, PathBuf};

use goodfetch_protocol::browser_cache::{BrowsersCleanOutput, BrowsersInstallOutput};
use goodfetch_test_support::{TestEnv, command};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Lays out a browser build's entry in the cache at `root`, last used at
/// `used_at`.
fn lay_out(root: &Path, build: &str, used_at: &str) -> PathBuf {
    let folder = root.join("browser").join("chrome-for-testing").join(build);
    fs::create_dir_all(folder.join("files")).unwrap();
    fs::write(folder.join("files").join("chrome"), "#!chrome\n").unwrap();
    fs::write(folder.join("lock"), "").unwrap();
    let entry = json!({
        "format_version": "0.1",
        "artifact": { "kind": "browser", "name": "chrome-for-testing", "build": build },
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "installed_at": used_at,
        "used_at": used_at,
    });
    fs::write(folder.join("entry.json"), entry.to_string()).unwrap();
    folder
}

fn run(env: &TestEnv, arguments: &[&str], variables: &[(&str, &Path)]) -> Value {
    let mut command = command(env, COLLIE);
    command.args(arguments);
    for (name, value) in variables {
        command.env(name, value);
    }
    let output = command.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&output).unwrap()
}

/// Covers: row:SDK-08, command:browsers.install
#[test]
fn browsers_install_fetches_what_the_version_pins_into_goodfetch_home() {
    let env = TestEnv::new();

    let output = run(&env, &["browsers", "install", "--json"], &[]);

    let output: BrowsersInstallOutput = serde_json::from_value(output).unwrap();
    assert_eq!(
        output,
        BrowsersInstallOutput {
            cache: env.home().join("browsers").display().to_string(),
            artifacts: Vec::new(),
        }
    );
}

/// Covers: story:3-140
#[test]
fn browsers_clean_dry_run_lists_what_it_would_delete_and_deletes_nothing() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");

    let output = run(&env, &["browsers", "clean", "--dry-run", "--json"], &[]);

    let output: BrowsersCleanOutput = serde_json::from_value(output).unwrap();
    assert!(output.dry_run);
    assert_eq!(
        output
            .deleted
            .iter()
            .map(|id| id.build.as_str())
            .collect::<Vec<_>>(),
        ["unused-since-2020"]
    );
    assert!(unused.join("entry.json").exists());
}

/// Covers: story:3-140, command:browsers.clean
#[test]
fn browsers_clean_deletes_what_nothing_pins_or_used_within_the_age() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");
    let recent = lay_out(&cache, "used-in-2100", "2100-01-01T00:00:00Z");

    let output = run(&env, &["browsers", "clean", "--json"], &[]);

    let output: BrowsersCleanOutput = serde_json::from_value(output).unwrap();
    assert_eq!(
        output
            .deleted
            .iter()
            .map(|id| id.build.as_str())
            .collect::<Vec<_>>(),
        ["unused-since-2020"]
    );
    assert!(!unused.exists());
    assert!(recent.join("entry.json").exists());
}

/// `browser_cache.max_unused_age` set in user configuration reaches the
/// clean-up: a build unused since 2020 is kept under an age of about 300
/// years (ADR 0098).
///
/// Covers: story:3-136, setting:browser_cache.max_unused_age
#[test]
fn browsers_clean_keeps_what_was_used_within_the_age_user_configuration_sets() {
    let env = TestEnv::new();
    let cache = env.home().join("browsers");
    let unused = lay_out(&cache, "unused-since-2020", "2020-01-01T00:00:00Z");
    fs::create_dir_all(env.home().join("0.0")).unwrap();
    fs::write(
        env.home().join("0.0").join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.browser_cache]\nmax_unused_age = 9999999999999\n",
    )
    .unwrap();

    let output = run(&env, &["browsers", "clean", "--json"], &[]);

    let output: BrowsersCleanOutput = serde_json::from_value(output).unwrap();
    assert_eq!(output.deleted, Vec::new());
    assert!(unused.join("entry.json").exists());
}

/// Covers: row:SDK-08
#[test]
fn goodfetch_browsers_path_wins_over_goodfetch_home() {
    let env = TestEnv::new();
    let copy = env.folder().join("prepared cache");
    let unused = lay_out(&copy, "unused-since-2020", "2020-01-01T00:00:00Z");

    let output = run(
        &env,
        &["browsers", "clean", "--json"],
        &[("GOODFETCH_BROWSERS_PATH", &copy)],
    );

    let output: BrowsersCleanOutput = serde_json::from_value(output).unwrap();
    assert_eq!(output.cache, copy.display().to_string());
    assert!(!unused.exists());
}

/// Covers: story:3-140
#[cfg(unix)]
#[test]
fn browsers_clean_never_cleans_a_read_only_goodfetch_browsers_path() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let copy = env.folder().join("read-only cache");
    let unused = lay_out(&copy, "unused-since-2020", "2020-01-01T00:00:00Z");
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o555)).unwrap();

    let output = run(
        &env,
        &["browsers", "clean", "--json"],
        &[("GOODFETCH_BROWSERS_PATH", &copy)],
    );
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(output["not_cleaned"], "cache_not_writable");
    assert_eq!(output["deleted"], json!([]));
    assert!(unused.join("entry.json").exists());
}
