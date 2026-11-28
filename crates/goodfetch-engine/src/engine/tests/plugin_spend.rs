//! A Plugin's spend outside the engine: raw units reported with
//! `usage.report`, reserved at `max_per_call * price` against a currency
//! Limit on `plugin_units`, unpriced units refused with `plugin_price_unknown`
//! under a currency Limit, and over-maximum reports recorded in full, flagged
//! in the report, and stopping the Run when they cross a Limit (ADR 0033,
//! issue 940).

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::settings::{PLUGINS, PluginEntry, SpendOverride};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::run::warning::{PLUGIN_FIGURES, PLUGIN_QUANTITY_ABOVE_MAXIMUM};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::usage::{INSTALL, PLUGIN_UNITS};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::run::call;
use super::{attached_on, engine_with};
use crate::composition::{Contribution, Handler};
use crate::plugin_gate::PluginFetch;
use crate::plugin_host::instance::{
    CallFailure, InMemoryPeer, InMemoryPlugin, PluginInstance, Scope,
};
use crate::plugin_host::spend::PluginSpend;
use crate::runs::RunSpendAccountant;
use crate::settings::{Layers, Policy, SettingsError, Values, resolve};
use crate::sync::Mutex;

/// One scripted reply the in-memory Plugin makes to an incoming call:
/// optional `usage.report` calls back, followed by the JSON result.
#[derive(Clone)]
struct ScriptedCall {
    reports: Vec<(String, u64)>,
}

async fn send_json(to: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, msg: Value) {
    let mut bytes = serde_json::to_vec(&msg).unwrap();
    bytes.push(b'\n');
    to.write_all(&bytes).await.unwrap();
    to.flush().await.unwrap();
}

/// Starts an in-memory Plugin peer that accepts `hello` matching `manifest`,
/// then serves each call by sending its scripted `usage.report` call backs
/// (verifying each succeeds) and returning `{ "ok": true }`.
fn spawn_scripted_peer(
    owner: &crate::tasks::TaskOwner,
    peer: InMemoryPeer,
    manifest: PluginManifest,
    script: Vec<ScriptedCall>,
    calls_received: Arc<Mutex<usize>>,
) {
    owner.spawn(async move {
        let (from_engine, _) = tokio::io::split(peer.from_engine);
        let (_, mut to_engine) = tokio::io::split(peer.to_engine);
        let mut lines = BufReader::new(from_engine).lines();

        // 1. Accept hello.
        let Some(Ok(hello_line)) = lines.next_line().await.transpose() else {
            return;
        };
        let hello: Value = serde_json::from_str(&hello_line).unwrap();
        let kinds: Vec<Value> = manifest
            .kinds
            .iter()
            .map(|kind| json!({ "kind": kind.as_str(), "capabilities": [] }))
            .collect();
        send_json(
            &mut to_engine,
            json!({
                "jsonrpc": "2.0",
                "id": hello["id"],
                "result": {
                    "protocol_version": "0.1",
                    "name": manifest.name,
                    "version": manifest.version,
                    "kinds": kinds,
                    "network_level": manifest.network_level.as_str(),
                },
            }),
        )
        .await;

        // 2. Serve each call in script order.
        let mut report_id = 100_i64;
        for step in script {
            let Some(Ok(line)) = lines.next_line().await.transpose() else {
                return;
            };
            let req: Value = serde_json::from_str(&line).unwrap();
            let call_id = req["id"].clone();
            *calls_received.lock().unwrap() += 1;
            for (unit, quantity) in step.reports {
                report_id += 1;
                send_json(
                    &mut to_engine,
                    json!({
                        "jsonrpc": "2.0",
                        "id": report_id,
                        "method": "usage.report",
                        "params": {
                            "call_id": call_id,
                            "unit": unit,
                            "quantity": quantity,
                        },
                    }),
                )
                .await;
                let Some(Ok(ack_line)) = lines.next_line().await.transpose() else {
                    return;
                };
                let ack: Value = serde_json::from_str(&ack_line).unwrap();
                assert!(
                    ack.get("result").is_some(),
                    "expected usage.report ok, got {ack}"
                );
            }
            send_json(
                &mut to_engine,
                json!({
                    "jsonrpc": "2.0",
                    "id": call_id,
                    "result": { "ok": true },
                }),
            )
            .await;
        }
    });
}

fn manifest_with_spend(name: &str, spend: &Value) -> PluginManifest {
    serde_json::from_value(json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": name,
        "network_level": "none",
        "spend": spend,
    }))
    .unwrap()
}

fn plugin_units_usd_limit(amount: u64) -> Value {
    json!({
        "scope": INSTALL,
        "usage_kind": PLUGIN_UNITS,
        "currency": "USD",
        "amount": amount,
    })
}

fn spend_contribution(
    manifest: PluginManifest,
    overrides: BTreeMap<String, SpendOverride>,
    script: Vec<ScriptedCall>,
    calls_to_make: usize,
    calls_received: Arc<Mutex<usize>>,
) -> Contribution {
    let op: &'static Operation = Box::leak(Box::new(Operation {
        name: "test.plugin_spend",
        description: "Runs scripted Plugin calls under Run spend accounting.",
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
    Contribution::new("test-plugin-spend").with_handler(Handler::new(
        op,
        move |params: Value, context| {
            let manifest = manifest.clone();
            let overrides = overrides.clone();
            let script = script.clone();
            let calls_received = Arc::clone(&calls_received);
            async move {
                let id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
                let parts = context.runs().live(id.clone()).await.unwrap();
                let clock = context.engine().clock();
                let owner = context.engine().owner();
                let resolved =
                    resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
                let fetch = Arc::new(
                    PluginFetch::of(&resolved, &manifest.name, clock, owner.blocking_threads())
                        .unwrap(),
                );
                let accountant =
                    RunSpendAccountant::new(parts.charging.clone(), parts.warnings.clone());
                let (instance, peer) = PluginInstance::start_in_memory(
                    owner,
                    InMemoryPlugin {
                        manifest: manifest.clone(),
                        product: Product::Collie,
                        fetch,
                        clock,
                        spend: Some(PluginSpend::new(accountant).with_overrides(overrides)),
                    },
                    Scope::Run(id),
                );
                spawn_scripted_peer(owner, peer, manifest, script, calls_received);

                let mut outcomes = Vec::new();
                for _ in 0..calls_to_make {
                    match instance.call("extract.content", &json!({})).await {
                        Ok(_) => outcomes.push(json!({ "ok": true })),
                        Err(CallFailure::Refused(error)) => outcomes.push(json!({
                            "error": error.code(),
                            "details": error.to_error_object().data.details,
                        })),
                        Err(other) => outcomes.push(json!({ "other": other.to_string() })),
                    }
                }
                instance.stop().await;
                Ok::<_, crate::CallError>(json!({ "outcomes": outcomes }))
            }
        },
    ))
}

/// Runs `calls_to_make` calls against an in-memory Plugin with `manifest` and
/// `script` inside a live Run (optionally preceded by `limit_usd`), returning
/// each call's outcome, the Run's final report, and how many calls actually
/// reached the Plugin.
fn run_plugin_spend_scenario(
    manifest: PluginManifest,
    limit_usd: Option<u64>,
    script: Vec<ScriptedCall>,
    calls_to_make: usize,
) -> (Vec<Result<(), String>>, Value, usize) {
    run_plugin_spend_with_overrides(manifest, BTreeMap::new(), limit_usd, script, calls_to_make)
}

fn run_plugin_spend_with_overrides(
    manifest: PluginManifest,
    overrides: BTreeMap<String, SpendOverride>,
    limit_usd: Option<u64>,
    script: Vec<ScriptedCall>,
    calls_to_make: usize,
) -> (Vec<Result<(), String>>, Value, usize) {
    let env = TestEnv::new();
    let calls_received = Arc::new(Mutex::new(0));
    let contribution = spend_contribution(
        manifest,
        overrides,
        script,
        calls_to_make,
        Arc::clone(&calls_received),
    );
    let engine = engine_with(Product::Collie, Some(contribution));
    let (outcomes_json, report) = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "hello",
                "params": {
                    "protocol_version": "0.1",
                    "surface": "cli",
                    "needs": "collie",
                    "product_version": "1.4.0",
                },
            })])
            .await;
        client.next().await.unwrap();
        let mut id = 1_i64;
        if let Some(amount) = limit_usd {
            let set = call(&mut client, id, "usage.set", plugin_units_usd_limit(amount)).await;
            assert!(set.get("result").is_some(), "usage.set failed: {set}");
            id += 1;
        }
        let started = call(&mut client, id, "run.start", json!({ "run_id": "spend" })).await;
        assert!(
            started.get("result").is_some(),
            "run.start failed: {started}"
        );
        id += 1;
        let ran = call(
            &mut client,
            id,
            "test.plugin_spend",
            json!({ "run_id": "spend" }),
        )
        .await;
        assert!(
            ran.get("result").is_some(),
            "test.plugin_spend failed: {ran}"
        );
        id += 1;
        let finished = call(&mut client, id, "run.finish", json!({ "run_id": "spend" })).await;
        assert!(
            finished.get("result").is_some(),
            "run.finish failed: {finished}"
        );
        (
            ran["result"]["outcomes"].clone(),
            finished["result"].clone(),
        )
    });

    let outcomes = outcomes_json
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            if entry.get("ok").is_some() {
                Ok(())
            } else {
                Err(entry["error"]
                    .as_str()
                    .map_or_else(|| entry.to_string(), str::to_owned))
            }
        })
        .collect();
    let received = *calls_received.lock().unwrap();
    (outcomes, report, received)
}

/// Covers: story:7-42, error:limit_reached
#[test]
fn a_priced_call_reserves_declared_maximum_settles_at_reported_and_refuses_when_limit_crossed() {
    // Declared max_per_call = 5 at 0.002 USD -> 10_000 millionths worst case per call.
    // Currency Limit = 15_000 millionths (0.015 USD).
    // Call 1 reports 2 solves (4_000) -> settles, 11_000 left (>= 10_000).
    // Call 2 reports 3 solves (6_000) -> settles, 5_000 left (< 10_000).
    // Call 3 needs 10_000 reservation -> refused BEFORE reaching the Plugin!
    let manifest = manifest_with_spend(
        "solver",
        &json!({ "solves": { "max_per_call": 5, "price": "0.002 USD" } }),
    );
    let (outcomes, report, received) = run_plugin_spend_scenario(
        manifest,
        Some(15_000),
        vec![
            ScriptedCall {
                reports: vec![("solves".to_owned(), 2)],
            },
            ScriptedCall {
                reports: vec![("solves".to_owned(), 3)],
            },
            ScriptedCall {
                reports: vec![("solves".to_owned(), 1)],
            },
        ],
        3,
    );

    assert_eq!(
        outcomes,
        vec![Ok(()), Ok(()), Err("limit_reached".to_owned())]
    );
    assert_eq!(
        received, 2,
        "the refused 3rd call must never reach the Plugin"
    );
    assert_eq!(
        report["usage"]["plugin_units"],
        json!([{
            "plugin": "solver",
            "unit": "solves",
            "quantity": 5,
            "price": { "currency": "USD", "amount": 10_000 },
            "from_plugin": true,
        }])
    );
    assert!(
        report["usage"].get("other").is_none(),
        "plugin_units must not leak into usage.other: {}",
        report["usage"]
    );
    let warning_codes: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|warning| warning["code"].as_str())
        .collect();
    assert_eq!(warning_codes, vec![PLUGIN_FIGURES]);
    assert_eq!(report["lifecycle"], json!("stopped"));
    assert_eq!(report["stop_reason"]["kind"], json!("limit"));
}

/// Covers: story:7-42, error:plugin_price_unknown
#[test]
fn an_unpriced_unit_under_a_currency_limit_fails_with_plugin_price_unknown_and_runs_without_one() {
    let manifest = manifest_with_spend(
        "unpriced-solver",
        &json!({ "solves": { "max_per_call": 5 } }),
    );

    // With a currency Limit on plugin_units: refused before reaching the Plugin.
    let (outcomes_limited, _report_limited, received_limited) = run_plugin_spend_scenario(
        manifest.clone(),
        Some(50_000),
        vec![ScriptedCall {
            reports: vec![("solves".to_owned(), 1)],
        }],
        1,
    );
    assert_eq!(
        outcomes_limited,
        vec![Err("plugin_price_unknown".to_owned())]
    );
    assert_eq!(
        received_limited, 0,
        "an unpriced unit under a currency Limit must not call the Plugin"
    );

    // Without a currency Limit on plugin_units: runs and records raw units without price.
    let (outcomes_free, report_free, received_free) = run_plugin_spend_scenario(
        manifest,
        None,
        vec![ScriptedCall {
            reports: vec![("solves".to_owned(), 3)],
        }],
        1,
    );
    assert_eq!(outcomes_free, vec![Ok(())]);
    assert_eq!(received_free, 1);
    assert_eq!(
        report_free["usage"]["plugin_units"],
        json!([{
            "plugin": "unpriced-solver",
            "unit": "solves",
            "quantity": 3,
            "from_plugin": true,
        }])
    );
    assert_eq!(report_free["lifecycle"], json!("finished"));
}

/// Covers: story:7-42, error:limit_reached
#[test]
fn an_over_maximum_report_is_recorded_in_full_flagged_and_stops_the_run_when_crossing_the_limit() {
    // max_per_call = 2 at 0.002 USD -> worst reservation = 4_000 (fits within 10_000 Limit).
    // Plugin reports 7 solves (14_000 USD) -> exceeds max_per_call AND crosses the 10_000 Limit!
    let manifest = manifest_with_spend(
        "overrun-solver",
        &json!({ "solves": { "max_per_call": 2, "price": "0.002 USD" } }),
    );
    let (outcomes, report, received) = run_plugin_spend_scenario(
        manifest.clone(),
        Some(10_000),
        vec![ScriptedCall {
            reports: vec![("solves".to_owned(), 7)],
        }],
        1,
    );

    assert_eq!(outcomes, vec![Err("limit_reached".to_owned())]);
    assert_eq!(received, 1);
    // Recorded in full: 7 solves and 14_000 millionths USD, never capped.
    assert_eq!(
        report["usage"]["plugin_units"],
        json!([{
            "plugin": "overrun-solver",
            "unit": "solves",
            "quantity": 7,
            "price": { "currency": "USD", "amount": 14_000 },
            "from_plugin": true,
        }])
    );
    // Flagged with both plugin_figures and plugin_quantity_above_maximum.
    let warning_codes: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|warning| warning["code"].as_str())
        .collect();
    assert!(
        warning_codes.contains(&PLUGIN_FIGURES),
        "missing plugin_figures warning: {warning_codes:?}"
    );
    assert!(
        warning_codes.contains(&PLUGIN_QUANTITY_ABOVE_MAXIMUM),
        "missing plugin_quantity_above_maximum warning: {warning_codes:?}"
    );
    // Stops the Run naming the Limit.
    assert_eq!(report["lifecycle"], json!("stopped"));
    assert_eq!(report["stop_reason"]["kind"], json!("limit"));
    assert_eq!(report["stop_reason"]["usage_kind"], json!(PLUGIN_UNITS));

    // When an over-maximum report stays within the Limit, it still records in
    // full and flags plugin_quantity_above_maximum without stopping the Run.
    let (outcomes_ok, report_ok, _) = run_plugin_spend_scenario(
        manifest,
        Some(50_000),
        vec![ScriptedCall {
            reports: vec![("solves".to_owned(), 4)],
        }],
        1,
    );
    assert_eq!(outcomes_ok, vec![Ok(())]);
    assert_eq!(report_ok["lifecycle"], json!("finished"));
    assert_eq!(
        report_ok["usage"]["plugin_units"],
        json!([{
            "plugin": "overrun-solver",
            "unit": "solves",
            "quantity": 4,
            "price": { "currency": "USD", "amount": 8_000 },
            "from_plugin": true,
        }])
    );
    let ok_codes: Vec<&str> = report_ok["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|warning| warning["code"].as_str())
        .collect();
    assert!(ok_codes.contains(&PLUGIN_QUANTITY_ABOVE_MAXIMUM));
    assert!(ok_codes.contains(&PLUGIN_FIGURES));
}

/// Non-policy layers setting `plugins.<name>.spend` are refused with
/// `setting_not_applicable`, while Operator policy accepts it and preserves it
/// across nearer layers setting `plugins.<name>.settings`.
///
/// Covers: story:7-19, error:setting_not_applicable
#[test]
fn plugins_spend_override_is_operator_policy_only() {
    let spend_only_map = serde_json::from_value(json!({
        "plugins": {
            "solver": {
                "spend": {
                    "solves": { "price": "0.005 USD" },
                },
            },
        },
    }))
    .unwrap();
    for (layer, make_layers) in [
        (
            SettingsLayer::UserConfiguration,
            (|v| Layers {
                user_configuration: v,
                ..Layers::default()
            }) as fn(Values) -> Layers,
        ),
        (SettingsLayer::ProjectConfiguration, |v| Layers {
            project_configuration: v,
            ..Layers::default()
        }),
        (SettingsLayer::Run, |v| Layers {
            run: v,
            ..Layers::default()
        }),
    ] {
        let values = Values::read(layer, &spend_only_map, Product::Collie).unwrap();
        let err = resolve(&make_layers(values), Product::Collie, Product::Collie).unwrap_err();
        assert!(
            matches!(
                err,
                SettingsError::NotApplicable { ref key, .. } if key == "plugins.solver.spend"
            ),
            "expected NotApplicable for {layer:?}, got {err:?}"
        );
    }

    let operator = Values::read(
        SettingsLayer::OperatorPolicy,
        &spend_only_map,
        Product::Collie,
    )
    .unwrap();
    let user_map = serde_json::from_value(json!({
        "plugins": { "solver": { "settings": { "mode": "fast" } } },
    }))
    .unwrap();
    let user = Values::read(SettingsLayer::UserConfiguration, &user_map, Product::Collie).unwrap();
    let resolved = resolve(
        &Layers {
            operator_policy: Policy {
                settings: operator,
                ..Policy::default()
            },
            user_configuration: user,
            ..Layers::default()
        },
        Product::Collie,
        Product::Collie,
    )
    .unwrap();
    let entry = PluginEntry::read(&resolved.value(&PLUGINS)["solver"]).unwrap();
    assert_eq!(
        entry.spend.get("solves"),
        Some(&SpendOverride {
            price: Some("0.005 USD".to_owned()),
        })
    );
}

/// An operator's price override (`plugins.<name>.spend.<unit>.price`) replaces
/// the manifest's declared price (or supplies one for an unpriced unit) in
/// #940's reservation.
///
/// Covers: story:7-19, story:7-42
#[test]
fn an_operator_price_override_replaces_the_declared_price_in_reservation() {
    let overrides = BTreeMap::from([(
        "solves".to_owned(),
        SpendOverride {
            price: Some("0.005 USD".to_owned()),
        },
    )]);

    // Manifest declares 0.002 USD with max_per_call = 2 (worst = 4_000).
    // Operator overrides to 0.005 USD -> worst = 10_000!
    // Under a 12_000 USD Limit: Call 1 reports 2 solves (10_000 USD), leaving
    // 2_000 (< 10_000), so Call 2 is refused with `limit_reached` (whereas
    // 2 * 4_000 = 8_000 at the declared price would have succeeded).
    let manifest = manifest_with_spend(
        "solver",
        &json!({ "solves": { "max_per_call": 2, "price": "0.002 USD" } }),
    );
    let (outcomes, report, received) = run_plugin_spend_with_overrides(
        manifest,
        overrides.clone(),
        Some(12_000),
        vec![
            ScriptedCall {
                reports: vec![("solves".to_owned(), 2)],
            },
            ScriptedCall {
                reports: vec![("solves".to_owned(), 2)],
            },
        ],
        2,
    );
    assert_eq!(outcomes, vec![Ok(()), Err("limit_reached".to_owned())]);
    assert_eq!(received, 1);
    assert_eq!(
        report["usage"]["plugin_units"],
        json!([{
            "plugin": "solver",
            "unit": "solves",
            "quantity": 2,
            "price": { "currency": "USD", "amount": 10_000 },
            "from_plugin": true,
        }])
    );

    // Operator override also supplies a price for a manifest unit that had none.
    let unpriced_manifest =
        manifest_with_spend("solver", &json!({ "solves": { "max_per_call": 2 } }));
    let (unpriced_outcomes, unpriced_report, unpriced_received) = run_plugin_spend_with_overrides(
        unpriced_manifest,
        overrides,
        Some(20_000),
        vec![ScriptedCall {
            reports: vec![("solves".to_owned(), 1)],
        }],
        1,
    );
    assert_eq!(unpriced_outcomes, vec![Ok(())]);
    assert_eq!(unpriced_received, 1);
    assert_eq!(
        unpriced_report["usage"]["plugin_units"],
        json!([{
            "plugin": "solver",
            "unit": "solves",
            "quantity": 1,
            "price": { "currency": "USD", "amount": 5_000 },
            "from_plugin": true,
        }])
    );
}

fn record_stream_contribution() -> Contribution {
    use crate::plugin_gate::proxy::{StreamAccountant as _, StreamRecord, StreamTransport};
    use crate::runs::RunStreamAccountant;

    let op: &'static Operation = Box::leak(Box::new(Operation {
        name: "test.record_stream",
        description: "Test operation that records a Plugin stream",
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
    Contribution::new("test-plugin-streams").with_handler(Handler::new(
        op,
        |params: Value, context| async move {
            let id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(id).await.unwrap();
            let accountant = RunStreamAccountant::new(parts.charging.clone(), parts.trace.clone());
            accountant
                .record(StreamRecord {
                    plugin: "kettle".to_owned(),
                    destination: "api.shop.test:443".to_owned(),
                    transport: StreamTransport::NetConnect,
                    bytes: 1_536,
                    duration_ms: 42,
                })
                .await;
            Ok::<_, crate::CallError>(json!({ "ok": true }))
        },
    ))
}

/// Recording a Plugin stream through `RunStreamAccountant` adds its bytes and
/// time to `run.report`'s `usage.plugin_streams` (without leaking into
/// `usage.other`) and writes a destination record to `trace/plugin-streams.jsonl`
/// through `RunTrace`'s writer (ADR 0033, issue 833).
///
/// Covers: story:7-55, story:7-61
#[test]
fn run_stream_accountant_records_bytes_and_time_in_usage_and_destination_in_run_trace() {
    use goodfetch_protocol::home::RUN_TRACE_PLUGIN_STREAMS;

    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(record_stream_contribution()));
    let report = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "hello",
                "params": {
                    "protocol_version": "0.1",
                    "surface": "cli",
                    "needs": "collie",
                    "product_version": "1.4.0",
                },
            })])
            .await;
        client.next().await.unwrap();
        let started = call(&mut client, 1, "run.start", json!({ "run_id": "streams" })).await;
        assert!(
            started.get("result").is_some(),
            "run.start failed: {started}"
        );
        let rec = call(
            &mut client,
            2,
            "test.record_stream",
            json!({ "run_id": "streams" }),
        )
        .await;
        assert!(
            rec.get("result").is_some(),
            "test.record_stream failed: {rec}"
        );
        let finished = call(&mut client, 3, "run.finish", json!({ "run_id": "streams" })).await;
        assert!(
            finished.get("result").is_some(),
            "run.finish failed: {finished}"
        );
        finished["result"].clone()
    });

    assert_eq!(
        report["usage"]["plugin_streams"],
        json!([{
            "destination": "api.shop.test:443",
            "bytes": 1_536,
            "time": 42,
        }])
    );
    assert!(
        report["usage"].get("other").is_none()
            || report["usage"]["other"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "plugin_stream_bytes/time must not leak into usage.other: {}",
        report["usage"]
    );

    let trace_path = env
        .home()
        .join("1")
        .join("runs")
        .join("streams")
        .join(RUN_TRACE_PLUGIN_STREAMS);
    let trace_text = std::fs::read_to_string(&trace_path)
        .unwrap_or_else(|err| panic!("failed reading {}: {err}", trace_path.display()));
    let entry: Value = serde_json::from_str(trace_text.lines().next().unwrap()).unwrap();
    assert_eq!(entry["plugin"], json!("kettle"));
    assert_eq!(entry["destination"], json!("api.shop.test:443"));
    assert_eq!(entry["bytes"], json!(1_536));
    assert_eq!(entry["duration_ms"], json!(42));
}

async fn seed_run_report_plugins(parts: &crate::runs::RunParts) {
    use goodfetch_protocol::plugin::approvals::Origin;
    use goodfetch_protocol::run::{PluginConfinement, PluginForm, PluginRun};

    let rows = [
        PluginRun {
            name: "first-party-notify".to_owned(),
            form: PluginForm::Process,
            origin: Some(Origin::FirstParty),
            digest: Some(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned(),
            ),
            network_level: "direct".to_owned(),
            confinement: PluginConfinement::Full,
            confinement_missing: Vec::new(),
        },
        PluginRun {
            name: "third-party-extract".to_owned(),
            form: PluginForm::Process,
            origin: Some(Origin::ThirdParty),
            digest: Some(
                "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                    .to_owned(),
            ),
            network_level: "none".to_owned(),
            confinement: PluginConfinement::Full,
            confinement_missing: Vec::new(),
        },
        PluginRun {
            name: "reduced-first-party".to_owned(),
            form: PluginForm::Process,
            origin: Some(Origin::FirstParty),
            digest: Some(
                "sha256:3333333333333333333333333333333333333333333333333333333333333333"
                    .to_owned(),
            ),
            network_level: "none".to_owned(),
            confinement: PluginConfinement::Reduced,
            confinement_missing: vec!["signal_scope".to_owned(), "user_namespace".to_owned()],
        },
    ];
    parts
        .state
        .write(move |connection| {
            for plugin in &rows {
                let missing = serde_json::to_string(&plugin.confinement_missing).unwrap();
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
    parts.plugins.record_sdk_handler("my-sdk-hook").await;
}

async fn seed_run_report_plugins_and_usage(parts: crate::runs::RunParts) {
    use crate::plugin_gate::proxy::{StreamAccountant as _, StreamRecord, StreamTransport};
    use crate::plugin_kinds::{
        Accounts as _, DeclaredUnit, PluginCall, ReportedSpend, SpendAccountant as _,
    };
    use crate::run::accounts::RunAccounts;
    use crate::runs::{RunSpendAccountant, RunStreamAccountant};

    seed_run_report_plugins(&parts).await;

    let accounts = RunAccounts {
        identity: &parts.identity,
        charging: &parts.charging,
        trace: &parts.trace,
    };
    let call_info = PluginCall {
        kind: "extract",
        plugin: "third-party-extract",
        method: "extract.content",
        class: OperationClass::Read,
        time: 17,
        failed: None,
    };
    accounts.call_starts(&call_info).await.unwrap();
    accounts.call_ended(&call_info).await.unwrap();

    let streams = RunStreamAccountant::new(parts.charging.clone(), parts.trace.clone());
    streams
        .record(StreamRecord {
            plugin: "first-party-notify".to_owned(),
            destination: "hooks.example:443".to_owned(),
            transport: StreamTransport::NetConnect,
            bytes: 4_096,
            duration_ms: 88,
        })
        .await;

    let spend = RunSpendAccountant::new(parts.charging.clone(), parts.warnings.clone());
    let ticket = spend
        .reserve(
            "third-party-extract",
            vec![DeclaredUnit {
                unit: "pages".to_owned(),
                max_per_call: 10,
                price: Some(goodfetch_protocol::catalog::PriceValue {
                    millionths: 1_000,
                    currency: "USD".to_owned(),
                }),
            }],
        )
        .await
        .unwrap();
    let mut reported = ReportedSpend::default();
    reported.units.insert("pages".to_owned(), 3);
    reported.any = true;
    ticket.finish(true, reported).await.unwrap();
}

fn run_report_plugins_contribution() -> Contribution {
    let op: &'static Operation = Box::leak(Box::new(Operation {
        name: "test.record_run_plugins",
        description: "Test-only: records first-party, third-party, reduced-confinement and SDK-handler Plugins plus all Plugin usage kinds on a live Run.",
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
    Contribution::new("test-run-report-plugins").with_handler(Handler::new(
        op,
        |params: Value, context| async move {
            let id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
            let parts = context.runs().live(id).await.unwrap();
            seed_run_report_plugins_and_usage(parts).await;
            Ok::<_, crate::CallError>(json!({ "ok": true }))
        },
    ))
}

fn assert_run_report_plugin_usage(report: &Value) {
    assert_eq!(
        report["usage"]["plugins"],
        json!([{
            "plugin": "third-party-extract",
            "calls": 1,
            "time": 17,
        }])
    );
    assert_eq!(
        report["usage"]["plugin_streams"],
        json!([{
            "destination": "hooks.example:443",
            "bytes": 4_096,
            "time": 88,
        }])
    );
    assert_eq!(
        report["usage"]["plugin_units"],
        json!([{
            "plugin": "third-party-extract",
            "unit": "pages",
            "quantity": 3,
            "price": { "currency": "USD", "amount": 3_000 },
            "from_plugin": true,
        }])
    );
    assert!(
        report["usage"].get("other").is_none(),
        "Plugin usage kinds must not leak into usage.other: {}",
        report["usage"]
    );
    let warning_codes: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|warning| warning["code"].as_str())
        .collect();
    assert!(
        warning_codes.contains(&PLUGIN_FIGURES),
        "Plugin-produced figures must also emit `plugin_figures` warning: {warning_codes:?}"
    );
}

/// A Run report lists every first-party and third-party Plugin and SDK handler
/// used with its form, origin, digest, network level and confinement (including
/// reduced confinement's missing parts), carries `plugin_calls`, `plugin_time`,
/// `plugin_stream_bytes` and `plugin_units`, and marks Plugin-produced figures
/// both on the figure (`from_plugin: true`) and in `warnings` (`plugin_figures`).
///
/// Covers: story:7-25
#[test]
fn run_report_lists_plugins_confinement_usage_and_marks_plugin_produced_figures() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(run_report_plugins_contribution()));
    let report = attached_on(&engine, &env, |mut client| async move {
        client
            .send(&[json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "hello",
                "params": {
                    "protocol_version": "0.1",
                    "surface": "cli",
                    "needs": "collie",
                    "product_version": "1.4.0",
                },
            })])
            .await;
        client.next().await.unwrap();
        let started = call(
            &mut client,
            1,
            "run.start",
            json!({ "run_id": "plugins-report" }),
        )
        .await;
        assert!(
            started.get("result").is_some(),
            "run.start failed: {started}"
        );
        let rec = call(
            &mut client,
            2,
            "test.record_run_plugins",
            json!({ "run_id": "plugins-report" }),
        )
        .await;
        assert!(
            rec.get("result").is_some(),
            "test.record_run_plugins failed: {rec}"
        );
        let finished = call(
            &mut client,
            3,
            "run.finish",
            json!({ "run_id": "plugins-report" }),
        )
        .await;
        assert!(
            finished.get("result").is_some(),
            "run.finish failed: {finished}"
        );
        finished["result"].clone()
    });

    assert_eq!(
        report["plugins"],
        json!([
            {
                "name": "first-party-notify",
                "form": "process",
                "origin": "first_party",
                "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "network_level": "direct",
                "confinement": "full",
            },
            {
                "name": "my-sdk-hook",
                "form": "sdk_handler",
                "network_level": "unconfined",
                "confinement": "unconfined",
            },
            {
                "name": "reduced-first-party",
                "form": "process",
                "origin": "first_party",
                "digest": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
                "network_level": "none",
                "confinement": "reduced",
                "confinement_missing": ["signal_scope", "user_namespace"],
            },
            {
                "name": "third-party-extract",
                "form": "process",
                "origin": "third_party",
                "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                "network_level": "none",
                "confinement": "full",
            },
        ])
    );
    assert_run_report_plugin_usage(&report);
}
