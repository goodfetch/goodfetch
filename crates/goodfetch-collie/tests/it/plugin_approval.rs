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

/// Places the Plugin `kettle` in the store of `env`, added at [`DIGEST`],
/// declaring Effects on `shop.test` and the value of `env:TOKEN`, and pins
/// it in the lockfile beside the project configuration of `env`'s folder.
fn placed(env: &TestEnv) {
    let plugin = env.home().join("plugins").join("kettle");
    let bundle = plugin.join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    let manifest = json!({
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
    });
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
