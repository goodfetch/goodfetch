//! A person's approval of a Plugin through `plugin.add` (ADR 0033, issue
//! 900): the answer states each permission the manifest declares; with
//! `approve`, the Operator policy's `plugins.<name>` entry allows the
//! Plugin, pins it to its digest and approves those permissions, each on
//! its own, recorded in the Audit log; a policy managed elsewhere is left
//! as it is and the entry to add there is answered. The Plugin is in the
//! store and pinned already, so no registry is reached.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write and read the Goodfetch home's files themselves"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::OPERATOR_POLICY_FILE;
use goodfetch_protocol::plugin::lockfile::LOCKFILE_NAME;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::consent_walls::audit_entries;
use super::run::call;
use super::{attached_on, engine_with, per_major};
use crate::settings::json_of_table;

/// A visibly fake image index digest.
const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// Places the Plugin `kettle` in the store of `env`, added at [`DIGEST`],
/// declaring Effects on `shop.test` and the value of `env:TOKEN`, and pins
/// it in a project's lockfile; answers the project file's path.
fn placed(env: &TestEnv) -> String {
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
    let project = env.home().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(LOCKFILE_NAME),
        format!(
            "version = \"0.1\"\n\n[[plugin]]\nname = \"kettle\"\nversion = \"1.0.0\"\ndigest = \"{DIGEST}\"\n"
        ),
    )
    .unwrap();
    project.join("goodfetch.toml").display().to_string()
}

/// Calls `plugin.add` for `kettle` as a person at the CLI, with `approve`.
fn added(env: &TestEnv, approve: bool) -> Value {
    let project = placed(env);
    attached_on(
        &engine_with(Product::Collie, None),
        env,
        |mut client| async move {
            client
            .send(&[json!({
                "jsonrpc": "2.0", "id": 0, "method": "hello",
                "params": { "protocol_version": "0.1", "surface": "cli", "needs": "collie", "product_version": "1.4.0" },
            })])
            .await;
            client.next().await.unwrap();
            let mut params = json!({
                "reference": format!("registry.example.com/goodfetch-plugin-kettle@{DIGEST}"),
                "project": { "path": project, "settings": {} },
            });
            if approve {
                params["approve"] = json!(true);
            }
            call(&mut client, 1, "plugin.add", params).await
        },
    )
}

/// The Operator policy file of `env`.
fn policy(env: &TestEnv) -> std::path::PathBuf {
    per_major(env).join(OPERATOR_POLICY_FILE)
}

/// The Audit log's `plugin_approved` entries, each its details.
fn approvals(env: &TestEnv) -> Vec<Value> {
    audit_entries(env)
        .into_iter()
        .filter(|entry| entry["kind"] == "plugin_approved")
        .map(|entry| entry["details"].clone())
        .collect()
}

/// The answer states each permission with its limits, a Secret's value
/// included; approved, the Operator policy allows the Plugin, pins its
/// digest and approves each permission on its own, the entry's own
/// settings kept, and the Audit log records the approval.
///
/// Covers: story:7-16, story:7-17, story:7-18, story:7-20
#[test]
fn an_approval_is_written_in_the_operator_policy_and_audited() {
    let env = TestEnv::new();
    std::fs::create_dir_all(per_major(&env)).unwrap();
    std::fs::write(
        policy(&env),
        "version = \"0.1\"\n\n[settings.plugins.kettle.settings]\nkettle_size = 2\n",
    )
    .unwrap();

    let answer = added(&env, true);

    let result = &answer["result"];
    let permissions: Vec<&Value> = result["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stated| &stated["permission"])
        .collect();
    assert_eq!(
        permissions,
        [
            &json!({ "permission": "effects", "domain": "shop.test" }),
            &json!({ "permission": "secret", "reference": "env:TOKEN" }),
            &json!({ "permission": "secret_value", "reference": "env:TOKEN" }),
        ],
        "{answer}"
    );
    assert_eq!(
        result["approval"],
        json!({ "outcome": "written", "policy": policy(&env).display().to_string() })
    );
    let written: toml_edit::DocumentMut = std::fs::read_to_string(policy(&env))
        .unwrap()
        .parse()
        .unwrap();
    let settings = json_of_table(written["settings"].as_table().unwrap());
    assert_eq!(
        settings["plugins"]["kettle"],
        json!({
            "settings": { "kettle_size": 2 },
            "allow": true,
            "digest": DIGEST,
            "approved": {
                "effects": ["shop.test"],
                "secrets": ["env:TOKEN"],
                "secret_values": ["env:TOKEN"],
            },
        })
    );
    assert_eq!(
        approvals(&env),
        [json!({ "plugin": "kettle", "digest": DIGEST, "permissions": result["permissions"] })]
    );
}

/// Without `approve`, the answer states the permissions and the Operator
/// policy is not written.
///
/// Covers: story:7-18
#[test]
fn an_add_without_approval_writes_no_policy() {
    let env = TestEnv::new();

    let answer = added(&env, false);

    assert_eq!(
        answer["result"]["permissions"].as_array().map(Vec::len),
        Some(3)
    );
    assert_eq!(answer["result"].get("approval"), None);
    assert!(!policy(&env).exists());
    assert_eq!(approvals(&env), Vec::<Value>::new());
}

/// An Operator policy managed elsewhere is left as it is: the answer
/// carries the entry to add there, and nothing is audited as written.
///
/// Covers: story:7-109
#[test]
fn a_managed_policy_is_left_as_it_is_and_the_entry_answered() {
    let env = TestEnv::new();
    let managed = "version = \"0.1\"\n\n[managed]\n";
    std::fs::create_dir_all(per_major(&env)).unwrap();
    std::fs::write(policy(&env), managed).unwrap();

    let answer = added(&env, true);

    let approval = &answer["result"]["approval"];
    assert_eq!(approval["outcome"], "managed", "{answer}");
    let entry = approval["entry"].as_str().unwrap();
    assert!(
        entry.contains("[settings.plugins]")
            && entry.contains("allow = true")
            && entry.contains(DIGEST),
        "{entry}"
    );
    assert_eq!(std::fs::read_to_string(policy(&env)).unwrap(), managed);
    assert_eq!(approvals(&env), Vec::<Value>::new());
}
