//! `plugin.list`, `plugin.remove`, shared Plugin store across Products, and
//! `not_in_product` refusal when Collie asks for a Foxhound-only Plugin
//! (ADRs 0033, 0094 and 0098; issue 928).

use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::OPERATOR_POLICY_FILE;
use goodfetch_protocol::plugin::approvals::Origin;
use goodfetch_protocol::plugin::index::{
    ConformanceResult, IndexEntry, PluginIndex, SignerIdentity,
};
use goodfetch_protocol::plugin::lockfile::LOCKFILE_NAME;
use goodfetch_protocol::plugin::{PluginKindName, PluginManifest};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::consent_walls::audit_entries;
use super::run::call;
use super::{attached_on, engine_with, per_major};
use crate::plugin_gate::PluginFetch;
use crate::plugin_host::instance::{InMemoryPlugin, PluginInstance, Scope};
use crate::plugin_index::PluginIndexStore;
use crate::plugin_lockfile;
use crate::settings::{Layers, json_of_table, resolve};
use crate::signed_set::SignedSetSigner;

const DIGEST_KETTLE: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const DIGEST_SINK: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const DIGEST_FETCH: &str =
    "sha256:3333333333333333333333333333333333333333333333333333333333333333";

fn place_plugin(
    env: &TestEnv,
    name: &str,
    kinds: &[&str],
    digest: &str,
    network_level: &str,
    settings_schema: Option<Value>,
) {
    let plugin = env.home().join("plugins").join(name);
    let bundle = plugin.join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    let mut manifest = json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": kinds,
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": name,
        "network_level": network_level,
    });
    if let Some(schema) = settings_schema {
        manifest["settings_schema"] = schema;
    }
    std::fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(plugin.join("1.0.0.digest"), digest).unwrap();
}

fn seed_index_for_kettle(env: &TestEnv) {
    let (signer, trust) = SignedSetSigner::for_root(
        "k1",
        "https://token.actions.githubusercontent.com",
        "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main",
        [0x77; 32],
    );
    let now: Timestamp = "2026-01-05T09:00:00Z".parse().unwrap();
    let exp = Timestamp::from_second(now.as_second() + 14 * 86_400).unwrap();
    let index = PluginIndex {
        version: 1,
        signed_at: now.to_string(),
        expires_at: exp.to_string(),
        plugins: vec![IndexEntry {
            name: "kettle".to_owned(),
            reference: "ghcr.io/vendor/goodfetch-plugin-kettle".to_owned(),
            digest: DIGEST_KETTLE.to_owned(),
            kinds: vec![PluginKindName::EXTRACT],
            signer: SignerIdentity {
                issuer: "https://token.actions.githubusercontent.com".to_owned(),
                workflow: "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0"
                    .to_owned(),
                repository: "github.com/vendor/kettle".to_owned(),
            },
            origin: Origin::ThirdParty,
            conformance: vec![ConformanceResult {
                suite: "extract".to_owned(),
                passed: true,
                engine_version: "0.1.0".to_owned(),
            }],
        }],
        revocations: Vec::new(),
    };
    let bundle = signer.sign(index).unwrap();
    PluginIndexStore::new(env.home().join("plugin-index"))
        .apply(bundle, &trust, now)
        .unwrap();
}

fn hello_for(surface: &str, needs: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": { "protocol_version": "0.1", "surface": surface, "needs": needs, "product_version": "1.4.0" },
    })
}

fn seed_kettle_project_and_policy(
    env: &TestEnv,
    signer: Option<&str>,
) -> (std::path::PathBuf, String, std::path::PathBuf) {
    let project = env.home().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let lockfile_path = project.join(LOCKFILE_NAME);
    let signer_line = signer.map_or_else(String::new, |s| format!("signer = \"{s}\"\n"));
    std::fs::write(
        &lockfile_path,
        format!(
            "version = \"0.1\"\n\n[[plugin]]\nname = \"kettle\"\nversion = \"1.0.0\"\ndigest = \"{DIGEST_KETTLE}\"\n{signer_line}"
        ),
    )
    .unwrap();
    let project_path = project.join("goodfetch.toml").display().to_string();

    let policy_path = per_major(env).join(OPERATOR_POLICY_FILE);
    std::fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
    std::fs::write(
        &policy_path,
        format!(
            "version = \"0.1\"\n\n[settings.plugins.kettle]\nallow = true\ndigest = \"{DIGEST_KETTLE}\"\n"
        ),
    )
    .unwrap();
    (lockfile_path, project_path, policy_path)
}

fn engine_with_use_kettle_op() -> crate::Engine {
    let mark_using = goodfetch_protocol::catalog::Operation {
        name: "test.use_kettle",
        description: "Marks a live Run as using the Plugin `kettle`.",
        class: goodfetch_protocol::catalog::OperationClass::Read,
        kind: goodfetch_protocol::catalog::OperationKind::RoundTrip,
        reach: goodfetch_protocol::catalog::Reach::AllSurfaces,
        params: goodfetch_protocol::catalog::schema_of::<Value>,
        result: goodfetch_protocol::catalog::schema_of::<Value>,
        events: &[],
        cancellable: false,
        input: None,
        bound: None,
        stability: goodfetch_protocol::catalog::Stability::Experimental,
        deprecated: None,
        arrived_in: goodfetch_protocol::catalog::ProtocolVersion { major: 0, minor: 1 },
    };
    let mark_op: &'static goodfetch_protocol::catalog::Operation = Box::leak(Box::new(mark_using));
    let contribution = crate::composition::Contribution::new("test-use-kettle").with_handler(
        crate::composition::Handler::new(mark_op, |params: Value, context| async move {
            let id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(id.clone()).await.unwrap();
            let clock = context.engine().clock();
            let owner = context.engine().owner();
            let resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
            let fetch = Arc::new(
                PluginFetch::of(&resolved, "kettle", clock, owner.blocking_threads()).unwrap(),
            );
            let manifest: PluginManifest = serde_json::from_value(json!({
                "manifest_version": "0.1",
                "name": "kettle",
                "version": "1.0.0",
                "kinds": ["extract"],
                "protocol": { "min": "0.1", "max": "0.1" },
                "entrypoint": "kettle",
                "network_level": "none",
            }))
            .unwrap();
            let (instance, _peer) = PluginInstance::start_in_memory(
                owner,
                InMemoryPlugin {
                    manifest,
                    product: Product::Collie,
                    fetch,
                    clock,
                },
                Scope::Run(id),
            );
            parts
                .plugins
                .track_instance(
                    "kettle".to_owned(),
                    Some(DIGEST_KETTLE.to_owned()),
                    None,
                    instance,
                )
                .await;
            Ok::<_, crate::CallError>(json!({ "ok": true }))
        }),
    );
    engine_with(Product::Collie, Some(contribution))
}

/// `plugin.list` returns every installed Plugin's kinds, digest, signer,
/// network level, approvals, conformance results and `settings_schema`,
/// marks Sink and Fetch backend Plugins as unavailable in Collie while
/// keeping them untouched across Collie operations, and marks them available
/// in Foxhound on the same Goodfetch home.
///
/// Covers: story:7-13, story:7-14
#[test]
fn plugin_list_shows_all_fields_settings_schema_and_marks_foxhound_plugins_on_collie() {
    let env = TestEnv::new();
    let schema = json!({
        "type": "object",
        "properties": { "language": { "type": "string", "default": "en" } }
    });
    place_plugin(
        &env,
        "kettle",
        &["extract"],
        DIGEST_KETTLE,
        "none",
        Some(schema.clone()),
    );
    place_plugin(&env, "lake-sink", &["sink"], DIGEST_SINK, "direct", None);
    place_plugin(
        &env,
        "edge-fetch",
        &["fetch_backend"],
        DIGEST_FETCH,
        "proxy_tcp",
        None,
    );
    seed_index_for_kettle(&env);
    let (_, project_path, _) =
        seed_kettle_project_and_policy(&env, Some("github.com/vendor/kettle (release.yml)"));

    let collie_list = attached_on(&engine_with(Product::Collie, None), &env, |mut client| {
        let project_path = project_path.clone();
        async move {
            client.send(&[hello_for("sdk_typescript", "collie")]).await;
            client.next().await.unwrap();
            call(
                &mut client,
                1,
                "plugin.list",
                json!({ "project": { "path": project_path, "settings": {} } }),
            )
            .await
        }
    });

    assert_eq!(collie_list["result"]["product"], json!("collie"));
    let plugins = collie_list["result"]["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 3);
    assert_eq!(plugins[0]["name"], json!("edge-fetch"));
    assert_eq!(plugins[0]["available_in_product"], json!(false));
    assert_eq!(plugins[0]["unsupported_kinds"], json!(["fetch_backend"]));

    let kettle = &plugins[1];
    assert_eq!(kettle["name"], json!("kettle"));
    assert_eq!(kettle["kinds"], json!(["extract"]));
    assert_eq!(kettle["digest"], json!(DIGEST_KETTLE));
    assert_eq!(
        kettle["signer"],
        json!("github.com/vendor/kettle (release.yml)")
    );
    assert_eq!(kettle["network_level"], json!("none"));
    assert_eq!(kettle["approvals"]["allow"], json!(true));
    assert_eq!(kettle["approvals"]["digest"], json!(DIGEST_KETTLE));
    assert_eq!(kettle["conformance"][0]["suite"], json!("extract"));
    assert_eq!(kettle["conformance"][0]["passed"], json!(true));
    assert_eq!(kettle["settings_schema"], schema);
    assert_eq!(kettle["available_in_product"], json!(true));

    assert_eq!(plugins[2]["name"], json!("lake-sink"));
    assert_eq!(plugins[2]["available_in_product"], json!(false));
    assert_eq!(plugins[2]["unsupported_kinds"], json!(["sink"]));
    assert!(env.home().join("plugins").join("lake-sink").exists());
    assert!(env.home().join("plugins").join("edge-fetch").exists());

    let foxhound_list = attached_on(
        &engine_with(Product::Foxhound, None),
        &env,
        |mut client| async move {
            client
                .send(&[hello_for("sdk_typescript", "foxhound")])
                .await;
            client.next().await.unwrap();
            call(&mut client, 1, "plugin.list", json!({})).await
        },
    );
    assert_eq!(foxhound_list["result"]["product"], json!("foxhound"));
    for entry in foxhound_list["result"]["plugins"].as_array().unwrap() {
        assert_eq!(entry["available_in_product"], json!(true), "{entry}");
    }
}

/// A Collie Run asking for a Sink or Fetch backend Plugin fails with
/// `not_in_product` and keeps both Plugins untouched in the shared store.
///
/// Covers: story:7-14
#[test]
fn collie_run_asking_for_sink_or_fetch_backend_plugin_fails_with_not_in_product() {
    let env = TestEnv::new();
    place_plugin(&env, "lake-sink", &["sink"], DIGEST_SINK, "direct", None);
    place_plugin(
        &env,
        "edge-fetch",
        &["fetch_backend"],
        DIGEST_FETCH,
        "proxy_tcp",
        None,
    );

    let (sink_err, fetch_err) = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client
                .send(&[json!({
                    "jsonrpc": "2.0", "id": 0, "method": "hello",
                    "params": { "protocol_version": "0.1", "surface": "sdk_typescript", "needs": "collie", "product_version": "1.4.0" },
                })])
                .await;
            client.next().await.unwrap();
            let sink_err = call(
                &mut client,
                1,
                "run.start",
                json!({
                    "settings": { "plugins": { "lake-sink": { "settings": {} } } }
                }),
            )
            .await;
            let fetch_err = call(
                &mut client,
                2,
                "run.start",
                json!({
                    "settings": { "plugins": { "edge-fetch": { "settings": {} } } }
                }),
            )
            .await;
            (sink_err, fetch_err)
        },
    );

    assert_eq!(
        sink_err["error"]["data"]["code"],
        json!("not_in_product"),
        "{sink_err}"
    );
    assert_eq!(
        sink_err["error"]["data"]["needs"],
        json!("foxhound"),
        "{sink_err}"
    );
    assert_eq!(fetch_err["error"]["data"]["code"], json!("not_in_product"));
    assert_eq!(fetch_err["error"]["data"]["needs"], json!("foxhound"));

    // Neither Plugin was deleted from the shared store.
    assert!(env.home().join("plugins").join("lake-sink").exists());
    assert!(env.home().join("plugins").join("edge-fetch").exists());
}

/// `plugin.remove` deletes the Plugin from the store, its lockfile entry and
/// its Operator policy approvals (audited as `plugin_removed`), refuses
/// non-person surfaces with `person_required`, and is refused with `run_live`
/// while a live Run is using the Plugin.
///
/// Covers: story:7-15
#[test]
fn plugin_remove_deletes_store_lockfile_and_approvals_and_refuses_while_live_run_uses_it() {
    let env = TestEnv::new();
    place_plugin(&env, "kettle", &["extract"], DIGEST_KETTLE, "none", None);
    let (lockfile_path, project_path, policy_path) = seed_kettle_project_and_policy(&env, None);

    let sdk_refused = attached_on(&engine_with(Product::Collie, None), &env, |mut client| {
        let project_path = project_path.clone();
        async move {
            client.send(&[hello_for("sdk_typescript", "collie")]).await;
            client.next().await.unwrap();
            call(
                &mut client,
                1,
                "plugin.remove",
                json!({ "name": "kettle", "project": { "path": project_path, "settings": {} } }),
            )
            .await
        }
    });
    assert_eq!(
        sdk_refused["error"]["data"]["code"],
        json!("person_required")
    );

    let engine = engine_with_use_kettle_op();
    let (refused_while_live, removed_after_finish) = attached_on(&engine, &env, |mut client| {
        let project_path = project_path.clone();
        async move {
            client.send(&[hello_for("cli", "collie")]).await;
            client.next().await.unwrap();
            let started = call(&mut client, 1, "run.start", json!({})).await;
            let run_id = started["result"]["run_id"].clone();
            let marked = call(
                &mut client,
                2,
                "test.use_kettle",
                json!({ "run_id": run_id }),
            )
            .await;
            assert_eq!(marked["result"]["ok"], json!(true));

            let refused = call(
                &mut client,
                3,
                "plugin.remove",
                json!({ "name": "kettle", "project": { "path": project_path, "settings": {} } }),
            )
            .await;

            let finished = call(&mut client, 4, "run.finish", json!({ "run_id": run_id })).await;
            assert!(finished.get("result").is_some(), "{finished}");
            let removed = call(
                &mut client,
                5,
                "plugin.remove",
                json!({ "name": "kettle", "project": { "path": project_path, "settings": {} } }),
            )
            .await;
            (refused, removed)
        }
    });

    assert_eq!(
        refused_while_live["error"]["data"]["code"],
        json!("run_live")
    );
    assert_eq!(removed_after_finish["result"]["name"], json!("kettle"));
    assert_eq!(
        removed_after_finish["result"]["removed_from_store"],
        json!(true)
    );
    assert_eq!(
        removed_after_finish["result"]["removed_from_lockfile"],
        json!(true)
    );
    assert_eq!(
        removed_after_finish["result"]["approvals"]["outcome"],
        json!("removed")
    );
    assert!(!env.home().join("plugins").join("kettle").exists());

    let lock = plugin_lockfile::read(&std::fs::read_to_string(&lockfile_path).unwrap()).unwrap();
    assert_eq!(lock.locked("kettle"), None);

    let policy_toml: toml_edit::DocumentMut = std::fs::read_to_string(&policy_path)
        .unwrap()
        .parse()
        .unwrap();
    let policy_json = json_of_table(policy_toml.as_table());
    assert!(
        policy_json["settings"]["plugins"].get("kettle").is_none(),
        "{policy_json}"
    );
    let removed_audits: Vec<Value> = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == "plugin_removed")
        .collect();
    assert_eq!(removed_audits.len(), 1);
    assert_eq!(removed_audits[0]["details"]["plugin"], json!("kettle"));
}
