//! `init` through Collie's executable (story:11-275, story:11-278): the
//! `browser` starter it embeds, written in each language into a folder
//! whose name holds a space and a non-ASCII letter, its manifest naming the
//! folder and the executable's version; a path already there is left as it
//! is and listed in `skipped`; a starter or language left out, with no
//! person to ask, and Foxhound's `crawl` are usage errors, exit `2`, that
//! write nothing. Asking on a terminal is tested in process, in the `init`
//! module's own tests; building and running each written project, in
//! [`projects`].
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and write the project folders themselves"
)]

mod projects;

use std::collections::BTreeMap;
use std::path::Path;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    Canaries, SecretKind, SnapshotValues, TestEnv, cli, snapshot_settings,
};
use serde_json::json;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The starters Collie's executable embeds.
const STARTERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/starters");

/// The project folder each test writes into, under the test's folder.
const PROJECT: &str = "my starter é";

/// Every file under `folder`, by its `/`-separated path from it, with its
/// text.
fn tree(folder: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(next) = folders.pop() {
        for item in std::fs::read_dir(&next).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let relative = path.strip_prefix(folder).unwrap();
            let relative = relative.to_string_lossy().replace('\\', "/");
            files.insert(relative, std::fs::read_to_string(&path).unwrap());
        }
    }
    files
}

/// The starter's files as `init` writes them into [`PROJECT`]: a file
/// stored as `<path>.template` at `<path>`, with the project's name and the
/// executable's version in place of their placeholders.
fn filled(starter: BTreeMap<String, String>) -> BTreeMap<String, String> {
    starter
        .into_iter()
        .map(|(path, text)| {
            let path = path
                .strip_suffix(".template")
                .map(str::to_owned)
                .unwrap_or(path);
            let text = text
                .replace("{{name}}", "my-starter")
                .replace("{{version}}", env!("CARGO_PKG_VERSION"));
            (path, text)
        })
        .collect()
}

/// `init --starter browser` writes each language's starter, every file of
/// it listed in `written`, into a new folder named with a space and a
/// non-ASCII letter.
///
/// Covers: story:11-275, command:init
#[rstest::rstest]
#[case::typescript("typescript")]
#[case::python("python")]
#[case::rust("rust")]
fn init_writes_the_browser_starter(#[case] language: &str) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args([
            "init",
            PROJECT,
            "--starter",
            "browser",
            "--lang",
            language,
            "--json",
        ])
        .json("InitResult")
        .unwrap();

    let starter = filled(tree(&Path::new(STARTERS).join("browser").join(language)));
    let printed = &checked.documents[0];
    assert_eq!(
        (printed, tree(&env.folder().join(PROJECT))),
        (
            &json!({
                "starter": "browser",
                "language": language,
                "written": starter.keys().collect::<Vec<_>>(),
                "skipped": [],
            }),
            starter,
        ),
        "{}",
        checked.output
    );
}

/// A file already at one of the starter's paths is left as it is and
/// listed in `skipped`; the others are written.
///
/// Covers: story:11-275, command:init
#[test]
fn init_never_overwrites_a_path() {
    let env = TestEnv::new();
    let project = env.folder().join(PROJECT);
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("package.json"), "{}\n").unwrap();

    let checked = cli(&env, COLLIE)
        .args([
            "init",
            PROJECT,
            "--starter",
            "browser",
            "--lang",
            "typescript",
            "--json",
        ])
        .json("InitResult")
        .unwrap();

    let printed = &checked.documents[0];
    let written = tree(&project);
    assert_eq!(
        (
            &printed["skipped"],
            printed["written"]
                .as_array()
                .map(|written| written.iter().any(|path| path == "package.json")),
            written.get("package.json").map(String::as_str),
            written.len(),
        ),
        (
            &json!(["package.json"]),
            Some(false),
            Some("{}\n"),
            tree(&Path::new(STARTERS).join("browser/typescript")).len(),
        ),
        "{}",
        checked.output
    );
}

/// With no person to ask, a starter or language left out is a usage error
/// naming its flag, and nothing is written.
///
/// Covers: story:11-275, command:init, exit:usage
#[rstest::rstest]
#[case::starter(&["--lang", "python"], "--starter")]
#[case::language(&["--starter", "browser"], "--lang")]
fn init_without_a_person_refuses_what_is_left_out(#[case] given: &[&str], #[case] flag: &str) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["init", PROJECT, "--json"])
        .args(given.iter().copied())
        .run()
        .unwrap();

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            output.stderr.contains(flag),
            env.folder().join(PROJECT).exists(),
        ),
        (Some(2), "", true, false),
        "{output}"
    );
}

/// Collie's executable embeds no Foxhound starter: `--starter crawl` is a
/// usage error naming Foxhound's executable, and nothing is written.
///
/// Covers: story:11-278, command:init, exit:usage
#[test]
fn init_refuses_foxhounds_starter() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args([
            "init",
            PROJECT,
            "--starter",
            "crawl",
            "--lang",
            "typescript",
        ])
        .run()
        .unwrap();

    assert_eq!(
        (
            output.exit,
            output
                .stderr
                .contains("goodfetch-foxhound init --starter crawl"),
            env.folder().join(PROJECT).exists(),
        ),
        (Some(2), true, false),
        "{output}"
    );
}

/// `init --deploy compose` writes `compose.yaml`, `operator-policy.toml` and
/// `seccomp-profile.json` into an empty directory without needing a project;
/// a second run writes nothing and lists all three as skipped; a pre-existing
/// path and a dangling symlink are left alone and never followed.
///
/// Covers: story:13-9, story:13-10, story:13-11, command:init
#[test]
fn init_deploy_compose_writes_three_files_and_never_overwrites_or_follows_dangling_links() {
    let env = TestEnv::new();

    // First run into an empty directory writes all three files.
    let first = cli(&env, COLLIE)
        .args(["init", "deploy-compose", "--deploy", "compose", "--json"])
        .json("InitDeployResult")
        .unwrap();
    assert_eq!(
        first.documents[0],
        json!({
            "target": "compose",
            "written": ["compose.yaml", "operator-policy.toml", "seccomp-profile.json"],
            "skipped": [],
        }),
        "{}",
        first.output
    );

    // Second run writes nothing and lists all three as skipped.
    let second = cli(&env, COLLIE)
        .args(["init", "deploy-compose", "--deploy", "compose", "--json"])
        .json("InitDeployResult")
        .unwrap();
    assert_eq!(
        second.documents[0],
        json!({
            "target": "compose",
            "written": [],
            "skipped": ["compose.yaml", "operator-policy.toml", "seccomp-profile.json"],
        }),
        "{}",
        second.output
    );

    // Pre-existing file + dangling symlink are left untouched and listed as skipped.
    let mixed = env.folder().join("deploy-mixed");
    std::fs::create_dir_all(&mixed).unwrap();
    std::fs::write(mixed.join("compose.yaml"), "# kept\n").unwrap();
    #[cfg(unix)]
    {
        let missing_target = mixed.join("does-not-exist-target");
        std::os::unix::fs::symlink(&missing_target, mixed.join("operator-policy.toml")).unwrap();
    }

    let third = cli(&env, COLLIE)
        .args(["init", "deploy-mixed", "--deploy", "compose", "--json"])
        .json("InitDeployResult")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(mixed.join("compose.yaml")).unwrap(),
        "# kept\n"
    );
    #[cfg(unix)]
    {
        assert!(
            !mixed.join("does-not-exist-target").exists(),
            "dangling symlink target must never be created"
        );
        assert_eq!(
            third.documents[0],
            json!({
                "target": "compose",
                "written": ["seccomp-profile.json"],
                "skipped": ["compose.yaml", "operator-policy.toml"],
            }),
            "{}",
            third.output
        );
    }
    #[cfg(not(unix))]
    {
        assert_eq!(
            third.documents[0],
            json!({
                "target": "compose",
                "written": ["operator-policy.toml", "seccomp-profile.json"],
                "skipped": ["compose.yaml"],
            }),
            "{}",
            third.output
        );
    }
}

/// An unknown `--deploy` target is a usage error (exit 2) naming the closest
/// known target, and writes nothing.
///
/// Covers: story:13-9, command:init, exit:usage
#[test]
fn init_deploy_unknown_target_is_usage_error_naming_closest() {
    let env = TestEnv::new();
    let output = cli(&env, COLLIE)
        .args(["init", "deploy-bad", "--deploy", "compose-v1", "--json"])
        .run()
        .unwrap();
    assert_eq!(output.exit, Some(2), "{output}");
    assert!(
        output.stderr.contains("compose"),
        "expected closest target `compose` in stderr: {output}"
    );
    assert!(!env.folder().join("deploy-bad").exists());
}

fn assert_compose_properties(compose_text: &str) {
    let compose: serde_json::Value = serde_saphyr::from_str(compose_text).unwrap();
    let daemon = &compose["services"]["daemon"];
    assert_eq!(daemon["command"], json!(["daemon", "run"]));
    assert_eq!(daemon["user"], json!("10001:10001"));
    assert_eq!(daemon["restart"], json!("unless-stopped"));
    assert_eq!(daemon["stop_grace_period"], json!("120s"));
    assert_eq!(daemon["shm_size"], json!("1g"));
    assert_eq!(daemon["logging"]["driver"], json!("local"));
    assert!(
        daemon.get("init").is_none(),
        "compose must not set init: true"
    );
    assert!(
        daemon.get("environment").is_none(),
        "compose must not pass settings via environment variables"
    );
    assert!(
        daemon["security_opt"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("seccomp=./seccomp-profile.json"))
    );
    assert!(
        daemon["ports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().is_some_and(|p| p.starts_with("127.0.0.1:")))
    );
    assert!(
        daemon["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("goodfetch-home:/home/goodfetch/.goodfetch"))
    );
    assert!(compose["volumes"].get("goodfetch-home").is_some());
    let hc = daemon["healthcheck"]["test"].as_array().unwrap();
    assert!(hc.iter().any(|v| v.as_str() == Some("daemon")));
    assert!(hc.iter().any(|v| v.as_str() == Some("health")));
}

/// With a Secret canary in `GOODFETCH_TOKEN`, no written file and no CLI
/// output contains the canary; rendered files name the installed executable,
/// version and pinned image digest, pass YAML Compose validation (and
/// `docker compose config` when available), and pass Operator policy parsing.
///
/// Covers: story:13-10, story:13-11, command:init
#[test]
fn init_deploy_compose_redacts_canary_and_renders_valid_compose_and_policy() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let token_value = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let dir = env.folder().join("deploy-verified");

    let checked = cli(&env, COLLIE)
        .env("GOODFETCH_TOKEN", &token_value)
        .args(["init", "deploy-verified", "--deploy", "compose", "--json"])
        .json("InitDeployResult")
        .unwrap();

    // Canary appears nowhere in stdout, stderr or any written file.
    assert!(!checked.output.stdout.contains(&token_value));
    assert!(!checked.output.stderr.contains(&token_value));
    for (path, text) in tree(&dir) {
        assert!(
            !text.contains(&token_value),
            "Secret canary leaked into {path}"
        );
    }

    let compose_text = std::fs::read_to_string(dir.join("compose.yaml")).unwrap();
    let policy_text = std::fs::read_to_string(dir.join("operator-policy.toml")).unwrap();
    let seccomp_text = std::fs::read_to_string(dir.join("seccomp-profile.json")).unwrap();

    // Installed executable version and pinned digest, never `:latest`.
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        compose_text.contains(&format!("ghcr.io/goodfetch/collie:{version}@sha256:")),
        "{compose_text}"
    );
    assert!(!compose_text.contains(":latest"), "{compose_text}");
    assert!(compose_text.contains("goodfetch-collie"), "{compose_text}");
    assert_compose_properties(&compose_text);

    // Operator policy parses through Goodfetch's real settings parser and
    // seccomp profile is valid JSON.
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(layout.operator_policy(), &policy_text).unwrap();
    let explained = cli(&env, COLLIE)
        .args([
            "config",
            "explain",
            "daemon.address",
            "daemon.tls.upstream",
            "ops.address",
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(explained.exit, Some(0), "{explained}");
    let explained_json: serde_json::Value = serde_json::from_str(&explained.stdout).unwrap();
    let settings_list = explained_json["settings"].as_array().unwrap();
    for key in ["daemon.address", "daemon.tls.upstream", "ops.address"] {
        let entry = settings_list
            .iter()
            .find(|item| item["key"] == key)
            .unwrap_or_else(|| panic!("missing `{key}` in config explain: {explained_json}"));
        assert_eq!(
            entry["layer"],
            json!("operator_policy"),
            "`{key}` should resolve from the rendered operator-policy.toml: {entry}"
        );
    }
    let seccomp_json: serde_json::Value = serde_json::from_str(&seccomp_text).unwrap();
    assert!(seccomp_json["syscalls"].is_array());

    // Redacted snapshot (ADR 0062): redacts version and image digest without
    // mangling `0.0.0.0:7600` when the workspace version is `0.0.0`.
    let mut settings = snapshot_settings(&SnapshotValues::of(&env));
    settings.add_filter(r"sha256:[0-9a-f]{64}", "sha256:[IMAGE_DIGEST]");
    settings.add_filter(
        &format!(
            r"(Goodfetch Collie |ghcr\.io/goodfetch/collie:){}",
            regex::escape(version)
        ),
        "${1}[PRODUCT_VERSION]",
    );
    settings.bind(|| {
        insta::assert_snapshot!(
            "init_deploy_compose_rendered",
            format!(
                "=== compose.yaml ===\n{compose_text}\n=== operator-policy.toml ===\n{policy_text}"
            )
        );
    });

    // When `docker compose` is available on the host, prove it accepts the file.
    if std::process::Command::new("docker")
        .args(["compose", "version"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        let config_check = std::process::Command::new("docker")
            .args(["compose", "-f"])
            .arg(dir.join("compose.yaml"))
            .args(["config", "--quiet"])
            .current_dir(&dir)
            .output()
            .expect("run docker compose config");
        assert!(
            config_check.status.success(),
            "docker compose config rejected rendered compose.yaml: {}",
            String::from_utf8_lossy(&config_check.stderr)
        );
    } else if std::env::var_os("CI").is_some() {
        panic!("docker compose is required under CI to validate rendered compose.yaml");
    }
}

/// Combining `--starter` with an unknown `--deploy` target fails as a usage
/// error (exit `2`) before writing any starter files; combining `--starter`
/// with a valid `--deploy compose` writes both sets of files.
///
/// Covers: story:13-9, command:init
#[test]
fn init_starter_with_unknown_deploy_writes_nothing_and_valid_deploy_writes_both() {
    let env = TestEnv::new();
    let bad_dir = env.folder().join("starter-bad-deploy");
    let bad = cli(&env, COLLIE)
        .args([
            "init",
            "starter-bad-deploy",
            "--starter",
            "browser",
            "--lang",
            "typescript",
            "--deploy",
            "nope",
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(bad.exit, Some(2), "{bad}");
    assert!(bad.stderr.contains("compose"), "{bad}");
    assert!(
        !bad_dir.exists(),
        "unknown --deploy target must fail before creating starter files"
    );

    let ok_dir = env.folder().join("starter-with-deploy");
    let ok = cli(&env, COLLIE)
        .args([
            "init",
            "starter-with-deploy",
            "--starter",
            "browser",
            "--lang",
            "typescript",
            "--deploy",
            "compose",
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(ok.exit, Some(0), "{ok}");
    let body: serde_json::Value = serde_json::from_str(&ok.stdout).unwrap();
    let written: Vec<&str> = body["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert!(written.contains(&"package.json"), "{written:?}");
    assert!(written.contains(&"compose.yaml"), "{written:?}");
    assert!(written.contains(&"operator-policy.toml"), "{written:?}");
    assert!(written.contains(&"seccomp-profile.json"), "{written:?}");
    assert!(ok_dir.join("package.json").is_file());
    assert!(ok_dir.join("compose.yaml").is_file());
}
