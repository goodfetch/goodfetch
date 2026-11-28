//! Run-state Plugin recording and resume refusal on changed third-party
//! digests (ADRs 0002, 0033, 0099 and 0104; issue 1036).
//!
//! Covers: story:7-97, story:7-98, story:7-99, error:plugin_changed

use std::path::Path;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::plugin::PLUGIN_CHANGED;
use goodfetch_protocol::plugin::approvals::{Origin, PLUGIN_CHANGE_ACCEPTED};
use goodfetch_protocol::run::{PluginRun, RunId};
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::run::call;
use super::{attached_on, engine_with};
use crate::composition::{Contribution, Handler};
use crate::run_state::RUN_FILE;

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FIRST_PARTY_A: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const FIRST_PARTY_B: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";

/// Installs a minimal Plugin bundle in `<home>/plugins/` with the given
/// `name`, `version` and multi-platform image index `digest`.
fn install_stub_plugin(home: &Path, name: &str, version: &str, digest: &str, marker: &str) {
    let plugin_dir = home.join("plugins").join(name);
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir).unwrap();
    }
    let bundle = plugin_dir.join(version);
    std::fs::create_dir_all(&bundle).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": name,
        "version": version,
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "bin/plugin",
        "network_level": "none",
    });
    std::fs::write(
        bundle.join("goodfetch-plugin.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(bundle.join("bin")).unwrap();
    std::fs::write(bundle.join("bin").join("plugin"), marker).unwrap();
    std::fs::write(plugin_dir.join(format!("{version}.digest")), digest).unwrap();
}

/// Test-only contribution recording process Plugins and an SDK handler into
/// the live Run's `run_plugins` table via its real `RunState` / `RunPlugins`.
fn record_plugins_contribution() -> Contribution {
    let op: &'static Operation = Box::leak(Box::new(Operation {
        name: "test.record_resume_plugins",
        description: "Test-only: records process Plugins and an SDK handler in run_plugins.",
        class: OperationClass::Read,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params: schema_of::<Value>,
        result: schema_of::<Value>,
        events: &[],
        cancellable: false,
        input: None,
        bound: None,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    }));
    Contribution::new("test-record-resume-plugins").with_handler(Handler::new(
        op,
        |params: Value, context| async move {
            let id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(id).await.unwrap();
            let plugins: Vec<PluginRun> =
                serde_json::from_value(params["plugins"].clone()).unwrap();
            parts
                .state
                .write(move |connection| {
                    for plugin in &plugins {
                        let missing =
                            serde_json::to_string(&plugin.confinement_missing).unwrap();
                        connection.execute(
                            "INSERT OR REPLACE INTO run_plugins \
                             (name, form, origin, digest, network_level, confinement, confinement_missing) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            (
                                &plugin.name,
                                plugin.form.as_str(),
                                plugin.origin.map(Origin::as_str),
                                &plugin.digest,
                                &plugin.network_level,
                                plugin.confinement.as_str(),
                                missing,
                            ),
                        )?;
                    }
                    Ok(())
                })
                .await
                .unwrap();
            if let Some(handler) = params["sdk_handler"].as_str() {
                parts.plugins.record_sdk_handler(handler).await;
            }
            Ok::<_, crate::CallError>(json!({ "ok": true }))
        },
    ))
}

fn hello_message(surface: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": surface,
            "needs": "collie",
            "product_version": "1.4.0",
        },
    })
}

fn run_file_of(home: &Path, run_id: &str) -> std::path::PathBuf {
    home.join("1").join("runs").join(run_id).join(RUN_FILE)
}

fn read_acceptances(path: &Path) -> Vec<(i64, String, String, Option<String>)> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT run_epoch, plugin, previous_digest, new_digest \
             FROM plugin_change_acceptances ORDER BY run_epoch, plugin",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

/// After a run, the run state lists each Plugin with its digest and each
/// SDK handler as `sdk_handler`.
///
/// Covers: story:7-97
#[test]
fn run_state_records_plugins_with_digest_and_sdk_handlers_as_sdk_handler() {
    let env = TestEnv::new();
    install_stub_plugin(env.home(), "acme-extract", "1.0.0", DIGEST_A, "linux-elf");
    let engine = engine_with(Product::Collie, Some(record_plugins_contribution()));

    let report = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello_message("cli")]).await;
        client.next().await.unwrap();
        let started = call(&mut client, 1, "run.start", json!({ "run_id": "rec-1" })).await;
        assert!(started.get("result").is_some(), "run.start: {started}");
        let rec = call(
            &mut client,
            2,
            "test.record_resume_plugins",
            json!({
                "run_id": "rec-1",
                "plugins": [{
                    "name": "acme-extract",
                    "form": "process",
                    "origin": "third_party",
                    "digest": DIGEST_A,
                    "network_level": "none",
                    "confinement": "full"
                }],
                "sdk_handler": "inline-extract"
            }),
        )
        .await;
        assert!(rec.get("result").is_some(), "record: {rec}");
        let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "rec-1" })).await;
        finished["result"].clone()
    });
    assert_eq!(report["plugins"].as_array().unwrap().len(), 2);

    let conn = Connection::open(run_file_of(env.home(), "rec-1")).unwrap();
    let mut stmt = conn
        .prepare("SELECT name, form, origin, digest FROM run_plugins ORDER BY name")
        .unwrap();
    let rows: Vec<(String, String, Option<String>, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "acme-extract".to_owned(),
                "process".to_owned(),
                Some("third_party".to_owned()),
                Some(DIGEST_A.to_owned()),
            ),
            (
                "inline-extract".to_owned(),
                "sdk_handler".to_owned(),
                None,
                None,
            ),
        ]
    );
}

fn start_record_and_pause_two_plugins(env: &TestEnv) {
    let engine = engine_with(Product::Collie, Some(record_plugins_contribution()));
    attached_on(&engine, env, |mut client| async move {
        client.send(&[hello_message("cli")]).await;
        client.next().await.unwrap();
        let started = call(&mut client, 1, "run.start", json!({ "run_id": "res-1" })).await;
        assert!(started.get("result").is_some(), "run.start: {started}");
        let rec = call(
            &mut client,
            2,
            "test.record_resume_plugins",
            json!({
                "run_id": "res-1",
                "plugins": [
                    {
                        "name": "acme-extract",
                        "form": "process",
                        "origin": "third_party",
                        "digest": DIGEST_A,
                        "network_level": "none",
                        "confinement": "full"
                    },
                    {
                        "name": "goodfetch-official",
                        "form": "process",
                        "origin": "first_party",
                        "digest": FIRST_PARTY_A,
                        "network_level": "none",
                        "confinement": "full"
                    }
                ]
            }),
        )
        .await;
        assert!(rec.get("result").is_some(), "record: {rec}");
        let paused = call(&mut client, 3, "run.pause", json!({ "run_id": "res-1" })).await;
        assert!(paused.get("result").is_some(), "run.pause: {paused}");
    });
}

/// A resume with a changed third-party digest fails with `plugin_changed`
/// whose `fix` names `--accept-plugin-change`; with the flag it resumes and
/// records the acceptance in run state and the Audit log; a first-party
/// Plugin's digest change does not refuse; and agent surfaces cannot pass
/// `accept_plugin_change` (ADR 0099).
///
/// Covers: story:7-98, error:plugin_changed
#[test]
fn resume_refuses_changed_third_party_plugin_until_accepted_and_allows_first_party_change() {
    let env = TestEnv::new();
    install_stub_plugin(env.home(), "acme-extract", "1.0.0", DIGEST_A, "v1");
    install_stub_plugin(
        env.home(),
        "goodfetch-official",
        "1.0.0",
        FIRST_PARTY_A,
        "v1",
    );
    let home_path = env.home().to_path_buf();
    start_record_and_pause_two_plugins(&env);

    // Replace BOTH Plugins with new digests: first-party must NOT refuse,
    // third-party MUST refuse.
    install_stub_plugin(
        &home_path,
        "goodfetch-official",
        "1.0.1",
        FIRST_PARTY_B,
        "v2",
    );
    install_stub_plugin(&home_path, "acme-extract", "1.0.1", DIGEST_B, "v2");

    // 1. Agent surface (`mcp`) cannot pass `accept_plugin_change` (ADR 0099).
    let engine_mcp = engine_with(Product::Collie, None);
    let agent_refused = attached_on(&engine_mcp, &env, |mut client| async move {
        client.send(&[hello_message("mcp")]).await;
        client.next().await.unwrap();
        call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "res-1", "accept_plugin_change": ["acme-extract"] }),
        )
        .await
    });
    assert_eq!(
        agent_refused["error"]["data"]["code"], "person_required",
        "agent surface must be refused: {agent_refused}"
    );

    // 2. Resume without `--accept-plugin-change` fails with `plugin_changed`,
    // then with `accept_plugin_change: ["acme-extract"]` resumes and audits.
    let engine2 = engine_with(Product::Collie, None);
    let (refused, resumed, audit) = attached_on(&engine2, &env, |mut client| async move {
        client.send(&[hello_message("cli")]).await;
        client.next().await.unwrap();
        let refused = call(&mut client, 1, "run.start", json!({ "run_id": "res-1" })).await;
        let resumed = call(
            &mut client,
            2,
            "run.start",
            json!({ "run_id": "res-1", "accept_plugin_change": ["acme-extract"] }),
        )
        .await;
        let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "res-1" })).await;
        assert!(finished.get("result").is_some(), "run.finish: {finished}");
        let audit = call(&mut client, 4, "audit.export", json!({})).await;
        (refused, resumed, audit)
    });

    let err = &refused["error"];
    assert_eq!(err["data"]["code"], PLUGIN_CHANGED.name, "got {refused}");
    assert_eq!(err["data"]["plugin"], "acme-extract");
    assert_eq!(err["data"]["recorded_digest"], DIGEST_A);
    assert_eq!(err["data"]["current_digest"], DIGEST_B);
    let fix = err["data"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("--accept-plugin-change"),
        "fix must name --accept-plugin-change: {fix}"
    );

    assert_eq!(resumed["result"]["started_as"], "resumed", "got {resumed}");

    let acceptances = read_acceptances(&run_file_of(env.home(), "res-1"));
    assert_eq!(
        acceptances,
        vec![(
            2,
            "acme-extract".to_owned(),
            DIGEST_A.to_owned(),
            Some(DIGEST_B.to_owned()),
        )]
    );

    let entries: Vec<&Value> = audit["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == PLUGIN_CHANGE_ACCEPTED.name)
        .collect();
    assert_eq!(entries.len(), 1, "expected 1 audit entry, got {entries:?}");
    assert_eq!(entries[0]["details"]["plugin"], "acme-extract");
    assert_eq!(entries[0]["details"]["previous_digest"], DIGEST_A);
    assert_eq!(entries[0]["details"]["new_digest"], DIGEST_B);
}

/// A Checkpoint taken on one OS resumes on another OS with the same
/// multi-platform image index pin, even when the unpacked platform binary
/// differs.
///
/// Covers: story:7-99
#[test]
fn checkpoint_resumes_on_another_os_with_the_same_multi_platform_index_pin() {
    let env_a = TestEnv::new();
    install_stub_plugin(
        env_a.home(),
        "acme-extract",
        "1.0.0",
        DIGEST_A,
        "linux-amd64-binary",
    );
    let engine_a = engine_with(Product::Collie, Some(record_plugins_contribution()));

    let checkpoint_path = attached_on(&engine_a, &env_a, |mut client| async move {
        client.send(&[hello_message("cli")]).await;
        client.next().await.unwrap();
        let started = call(&mut client, 1, "run.start", json!({ "run_id": "xplat-1" })).await;
        assert!(started.get("result").is_some(), "run.start: {started}");
        let rec = call(
            &mut client,
            2,
            "test.record_resume_plugins",
            json!({
                "run_id": "xplat-1",
                "plugins": [{
                    "name": "acme-extract",
                    "form": "process",
                    "origin": "third_party",
                    "digest": DIGEST_A,
                    "network_level": "none",
                    "confinement": "full"
                }]
            }),
        )
        .await;
        assert!(rec.get("result").is_some(), "record: {rec}");
        let paused = call(&mut client, 3, "run.pause", json!({ "run_id": "xplat-1" })).await;
        paused["result"]["checkpoint"]["location"]
            .as_str()
            .unwrap()
            .to_owned()
    });

    // Second engine representing another OS/platform: different unpacked binary
    // inside the bundle, same multi-platform OCI image index digest `DIGEST_A`.
    let env_b = TestEnv::new();
    install_stub_plugin(
        env_b.home(),
        "acme-extract",
        "1.0.0",
        DIGEST_A,
        "darwin-arm64-binary",
    );
    let engine_b = engine_with(Product::Collie, None);
    let (resumed, report) = attached_on(&engine_b, &env_b, |mut client| async move {
        client.send(&[hello_message("cli")]).await;
        client.next().await.unwrap();
        let resumed = call(
            &mut client,
            1,
            "run.resume",
            json!({ "run_id": "xplat-1", "checkpoint": checkpoint_path }),
        )
        .await;
        let finished = call(&mut client, 2, "run.finish", json!({ "run_id": "xplat-1" })).await;
        (resumed, finished)
    });

    assert_eq!(resumed["result"]["started_as"], "resumed", "got {resumed}");
    assert_eq!(report["result"]["plugins"][0]["name"], "acme-extract");
    assert_eq!(report["result"]["plugins"][0]["digest"], DIGEST_A);
}

fn hello_with_handlers(surface: &str, handlers: &Value, settings: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": surface,
            "needs": "collie",
            "product_version": "1.4.0",
            "handlers": handlers,
            "settings": settings,
        },
    })
}

fn run_epoch_of(home: &Path, run_id: &str) -> i64 {
    Connection::open(run_file_of(home, run_id))
        .unwrap()
        .query_row("SELECT epoch FROM run", [], |row| row.get(0))
        .unwrap()
}

/// `detached` on a Run that uses an SDK handler (chosen by `registered` or by
/// explicit settings) fails with `detach_unsupported` naming the choosing key
/// in both `details.key` and `details.fix`; overriding the key to a built-in
/// avoids the handler refusal (ADR 0104).
///
/// Covers: story:7-75, error:detach_unsupported
#[test]
fn detached_start_using_sdk_handler_fails_with_detach_unsupported_naming_choosing_key() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, None);

    // 1. Single handler chosen via `registered` -> detach_unsupported naming `content.extractor`.
    let (via_registered, overridden) = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[hello_with_handlers(
                "rust",
                &json!({ "extract": ["my_ext"] }),
                &json!({}),
            )])
            .await;
        let greeted = client.next().await.unwrap();
        assert!(greeted.get("result").is_some(), "hello: {greeted}");

        let via_registered = call(
            &mut client,
            1,
            "recipe.start",
            json!({ "recipe": "demo", "detached": true }),
        )
        .await;

        // Explicitly overriding `content.extractor` to a built-in does NOT fail
        // on `content.extractor` (fails on `engine` because this is an Attached engine).
        let overridden = call(
            &mut client,
            2,
            "recipe.start",
            json!({
                "recipe": "demo",
                "detached": true,
                "settings": { "content": { "extractor": "dom_smoothie" } },
            }),
        )
        .await;
        (via_registered, overridden)
    });
    assert_eq!(
        via_registered["error"]["data"]["code"], "detach_unsupported",
        "{via_registered}"
    );
    assert_eq!(via_registered["error"]["data"]["key"], "content.extractor");
    assert!(
        via_registered["error"]["data"]["fix"]
            .as_str()
            .unwrap()
            .contains("content.extractor"),
        "{via_registered}"
    );
    assert_eq!(
        overridden["error"]["data"]["code"], "detach_unsupported",
        "{overridden}"
    );
    assert_eq!(
        overridden["error"]["data"]["key"], "engine",
        "built-in override must clear the SDK handler check: {overridden}"
    );

    // 2. Two handlers with explicit `content.extractor` in settings -> detach_unsupported naming `content.extractor`.
    let via_settings = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[hello_with_handlers(
                "rust",
                &json!({ "extract": ["ext_a", "ext_b"] }),
                &json!({ "content": { "extractor": "ext_a" } }),
            )])
            .await;
        let greeted = client.next().await.unwrap();
        assert!(greeted.get("result").is_some(), "hello: {greeted}");
        call(
            &mut client,
            1,
            "catalog_run.start",
            json!({
                "target": { "kind": "agent", "url": "https://example.test", "task": "check" },
                "detached": true,
            }),
        )
        .await
    });
    assert_eq!(
        via_settings["error"]["data"]["code"], "detach_unsupported",
        "{via_settings}"
    );
    assert_eq!(via_settings["error"]["data"]["key"], "content.extractor");
    assert!(
        via_settings["error"]["data"]["fix"]
            .as_str()
            .unwrap()
            .contains("content.extractor"),
        "{via_settings}"
    );
}

/// Resuming a Run without an SDK handler it used is refused before any work
/// (epoch stays unchanged); registering that handler allows the Run to resume
/// cleanly (ADR 0104).
///
/// Covers: story:7-75, error:setting_invalid
#[test]
fn resume_without_recorded_sdk_handler_is_refused_before_work_and_resumes_when_registered() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(record_plugins_contribution()));

    // Start Run with SDK handler `inline-extract`, record its use, and stop cleanly (epoch 1).
    attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[hello_with_handlers(
                "rust",
                &json!({ "extract": ["inline-extract"] }),
                &json!({}),
            )])
            .await;
        let greeted = client.next().await.unwrap();
        assert!(greeted.get("result").is_some(), "hello: {greeted}");
        let started = call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "sdk-res-1" }),
        )
        .await;
        assert!(started.get("result").is_some(), "run.start: {started}");
        let rec = call(
            &mut client,
            2,
            "test.record_resume_plugins",
            json!({
                "run_id": "sdk-res-1",
                "plugins": [],
                "sdk_handler": "inline-extract"
            }),
        )
        .await;
        assert!(rec.get("result").is_some(), "record: {rec}");
        let stopped = call(&mut client, 3, "run.stop", json!({ "run_id": "sdk-res-1" })).await;
        assert!(stopped.get("result").is_some(), "run.stop: {stopped}");
    });

    assert_eq!(run_epoch_of(env.home(), "sdk-res-1"), 1);

    // Reconnect WITHOUT `inline-extract` registered -> refused before any work.
    let (refused_start, refused_resume) = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello_message("rust")]).await;
        client.next().await.unwrap();
        let refused_start = call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "sdk-res-1" }),
        )
        .await;
        let refused_resume = call(
            &mut client,
            2,
            "run.resume",
            json!({ "run_id": "sdk-res-1" }),
        )
        .await;
        (refused_start, refused_resume)
    });
    for refused in [&refused_start, &refused_resume] {
        assert_eq!(
            refused["error"]["data"]["code"], "setting_invalid",
            "{refused}"
        );
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("inline-extract"),
            "{refused}"
        );
    }
    assert_eq!(run_epoch_of(env.home(), "sdk-res-1"), 1);

    // Reconnect WITH `inline-extract` registered -> resumes cleanly!
    let (resumed, finished) = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[hello_with_handlers(
                "rust",
                &json!({ "extract": ["inline-extract"] }),
                &json!({}),
            )])
            .await;
        let greeted = client.next().await.unwrap();
        assert!(greeted.get("result").is_some(), "hello: {greeted}");
        let resumed = call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "sdk-res-1" }),
        )
        .await;
        let finished = call(
            &mut client,
            2,
            "run.finish",
            json!({ "run_id": "sdk-res-1" }),
        )
        .await;
        (resumed, finished)
    });
    assert_eq!(resumed["result"]["started_as"], "resumed", "{resumed}");
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
}
