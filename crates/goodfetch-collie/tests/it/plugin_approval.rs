//! `plugin add`'s approval through Collie's executable (ADR 0033, issue
//! 900): once the Plugin is installed and pinned, the person sees each
//! permission its manifest declares with its limits, printed once, and a
//! yes approves the Plugin and those permissions in the Operator policy;
//! a no leaves the policy as it was. Under `--json` the command is refused
//! with `person_required`, exit 4, before any engine call. The Plugin is in
//! the store and pinned already, so no registry is reached.
#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{TestEnv, cli, pty};
use rstest::rstest;
use serde_json::{Value, json};

use crate::COLLIE;

/// A visibly fake image index digest.
const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// The Plugin's OCI reference, pinned by [`DIGEST`].
fn reference() -> String {
    format!("registry.example.com/goodfetch-plugin-kettle@{DIGEST}")
}

/// Places the Plugin `kettle` in the store of `env` with `manifest`, added
/// at [`DIGEST`], and pins it in the lockfile beside the project
/// configuration of `env`'s folder.
fn place(env: &TestEnv, manifest: &Value) {
    let plugin = env.home().join("plugins").join("kettle");
    let bundle = plugin.join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(plugin.join("1.0.0.digest"), DIGEST).unwrap();
    std::fs::write(env.folder().join("goodfetch.toml"), "version = \"0.1\"\n").unwrap();
    std::fs::write(
        env.folder().join("goodfetch-plugins.lock"),
        format!(
            "version = \"0.1\"\n\n[[plugin]]\nname = \"kettle\"\nversion = \"1.0.0\"\ndigest = \"{DIGEST}\"\n"
        ),
    )
    .unwrap();
}

/// Places the Plugin `kettle` in the store of `env`, added at [`DIGEST`],
/// declaring Effects on `shop.test` and the value of `env:TOKEN`, and pins
/// it in the lockfile beside the project configuration of `env`'s folder.
fn placed(env: &TestEnv) {
    place(
        env,
        &json!({
            "manifest_version": "0.1",
            "name": "kettle",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "kettle",
            "network_level": "none",
            "permissions": {
                "domains": ["shop.test"],
                "secrets": ["env:TOKEN"],
                "grants": ["effects", "secret_value"],
            },
        }),
    );
}

/// The person installs the Plugin, sees its three permissions once, each
/// with what it lets the Plugin do and what the engine cannot check, and
/// answers the approval: a yes writes it in the Operator policy, a no
/// writes nothing.
///
/// Covers: story:7-18, story:7-20, command:plugin.add
#[rstest]
#[case::approved("y", true)]
#[case::declined("n", false)]
fn plugin_add_states_the_permissions_once_and_asks_for_the_approval(
    #[case] answer: &str,
    #[case] approved: bool,
) {
    let env = TestEnv::new();
    placed(&env);
    let policy = HomeLayout::of(env.home()).operator_policy();

    let mut added = pty(&env, COLLIE)
        .args(["plugin", "add", &reference()])
        .spawn()
        .unwrap();
    added.expect("[y/N]").unwrap();
    added.send_line("y").unwrap();
    added.expect("in the Operator policy? [y/N]").unwrap();
    added.send_line(answer).unwrap();
    let output = added.finish().unwrap();

    let shown = output.stripped();
    assert_eq!(output.exit, Some(0), "{shown}");
    assert_eq!(
        (
            shown.matches("The engine cannot check").count(),
            shown.contains("receive the value of env:TOKEN itself"),
        ),
        (3, true),
        "{shown}"
    );
    let written = std::fs::read_to_string(&policy).unwrap_or_default();
    assert_eq!(
        (
            written.contains("allow = true"),
            written.contains(DIGEST),
            shown.contains("Approved the Plugin kettle"),
        ),
        (approved, approved, approved),
        "{written}\n{shown}"
    );
}

/// Under `--json` no person answers: the command is refused with
/// `person_required`, exit 4, its fix naming the interactive command, and
/// nothing is installed or approved.
///
/// Covers: story:7-20, command:plugin.add
#[test]
fn plugin_add_under_json_is_refused_for_want_of_a_person() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["plugin", "add", &reference(), "--json"])
        .run()
        .unwrap();

    let failed: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        (
            output.exit,
            &failed["error"]["code"],
            failed["error"]["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("plugin add")),
        ),
        (Some(4), &json!("person_required"), true),
        "{output:?}"
    );
    assert!(!HomeLayout::of(env.home()).operator_policy().exists());
}

/// Story 7-3: before any approval is written, `plugin add` prints every
/// field the person grants: kinds, network level, permissions, environment
/// variables, files, processes and browsers, both for a toy Plugin with
/// none of the optional ones and for a browser Plugin that declares each.
/// The Operator policy stays absent until the person confirms.
///
/// Covers: story:7-3, command:plugin.add
#[rstest]
#[case::toy(
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "none",
    }),
    &[
        "Kinds: extract",
        "Network level: none",
        "Environment variables: none",
        "Files: none",
        "Processes: none",
        "Browsers: none",
        "Permissions: none",
    ],
)]
#[case::with_browsers(
    json!({
        "manifest_version": "0.1",
        "name": "kettle",
        "version": "1.0.0",
        "kinds": ["browser"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "kettle",
        "network_level": "proxy_tcp",
        "env": { "KETTLE_ENDPOINT": "endpoint" },
        "files": ["/etc/kettle/ca.pem"],
        "processes": ["/usr/bin/kettle-helper"],
        "browsers": [{
            "name": "kettle-browser",
            "platforms": {
                "linux-x64": {
                    "url": "https://downloads.example.com/kettle-browser-linux-x64.tar.gz",
                    "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                },
            },
        }],
        "permissions": {
            "domains": ["api.example.com"],
            "grants": ["effects"],
        },
    }),
    &[
        "Kinds: browser",
        "Network level: proxy_tcp",
        "Environment variables: KETTLE_ENDPOINT <- endpoint",
        "Files: /etc/kettle/ca.pem",
        "Processes: /usr/bin/kettle-helper",
        "kettle-browser",
        "linux-x64",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "cause Effects on api.example.com",
    ],
)]
fn plugin_add_shows_everything_granted_before_writing_the_approval(
    #[case] manifest: Value,
    #[case] expected: &[&str],
) {
    let env = TestEnv::new();
    place(&env, &manifest);
    let policy = HomeLayout::of(env.home()).operator_policy();

    let mut added = pty(&env, COLLIE)
        .args(["plugin", "add", &reference()])
        .spawn()
        .unwrap();
    added.expect("[y/N]").unwrap();
    added.send_line("y").unwrap();
    added.expect("in the Operator policy? [y/N]").unwrap();
    let written_before = policy.exists();
    added.send_line("y").unwrap();
    let output = added.finish().unwrap();

    let shown = output.stripped();
    let missing: Vec<&&str> = expected
        .iter()
        .filter(|needle| !shown.contains(**needle))
        .collect();
    assert_eq!(
        (output.exit, written_before, policy.exists(), missing),
        (Some(0), false, true, Vec::<&&str>::new()),
        "{shown}"
    );
}

/// `plugin search --json` prints kinds, signer identity and conformance
/// results from the cached Plugin index, and `plugin index import --json` is
/// refused with `person_required` (exit 4) naming `plugin index import`.
///
/// Covers: story:7-1, story:7-9, command:plugin.search, command:plugin.index_import
#[test]
fn plugin_search_lists_index_and_index_import_requires_a_person() {
    let env = TestEnv::new();
    let index_dir = env.home().join("plugin-index");
    std::fs::create_dir_all(&index_dir).unwrap();
    let fixture = json!({
        "payload": {
            "version": 3,
            "signed_at": "2026-11-27T00:00:00Z",
            "expires_at": "2099-12-31T00:00:00Z",
            "plugins": [{
                "name": "kettle",
                "reference": "ghcr.io/vendor/goodfetch-plugin-kettle",
                "digest": DIGEST,
                "kinds": ["extract"],
                "signer": {
                    "issuer": "https://token.actions.githubusercontent.com",
                    "workflow": "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0",
                    "repository": "github.com/vendor/kettle",
                },
                "origin": "third_party",
                "conformance": [{
                    "suite": "extract",
                    "passed": true,
                    "engine_version": "0.1.0",
                }],
            }],
            "revocations": [],
        },
        "bundle": {
            "key_id": "test-key",
            "issuer": "https://token.actions.githubusercontent.com",
            "workflow": "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main",
            "signature": "AAAA",
        },
    });
    std::fs::write(index_dir.join("index.json"), fixture.to_string()).unwrap();

    // `plugin search --json` lists kinds, signer and conformance results.
    let search = cli(&env, COLLIE)
        .args(["plugin", "search", "--json"])
        .run()
        .unwrap();
    assert_eq!(search.exit, Some(0), "stderr: {}", search.stderr);
    let body: Value = serde_json::from_str(&search.stdout).unwrap();
    assert_eq!(body["index_version"], json!(3));
    assert_eq!(body["plugins"][0]["name"], json!("kettle"));
    assert_eq!(body["plugins"][0]["kinds"], json!(["extract"]));
    assert_eq!(
        body["plugins"][0]["signer"]["repository"],
        json!("github.com/vendor/kettle")
    );
    assert_eq!(body["plugins"][0]["conformance"][0]["passed"], json!(true));

    // `plugin index import <file> --json` refuses with `person_required`, exit 4.
    let index_file = env.folder().join("signed-index.json");
    std::fs::write(&index_file, fixture.to_string()).unwrap();
    let import = cli(&env, COLLIE)
        .args([
            "plugin",
            "index",
            "import",
            index_file.to_str().unwrap(),
            "--json",
        ])
        .run()
        .unwrap();
    assert_eq!(import.exit, Some(4));
    let failed: Value = serde_json::from_str(&import.stdout).unwrap();
    assert_eq!(failed["error"]["code"], json!("person_required"));
    let fix = failed["error"]["details"]["fix"]
        .as_str()
        .or_else(|| failed["error"]["fix"].as_str());
    assert!(
        fix.is_some_and(|text| text.contains("plugin index import")),
        "{failed}"
    );
}
