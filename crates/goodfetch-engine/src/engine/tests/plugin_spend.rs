//! A Plugin's spend outside the engine: raw units reported with
//! `usage.report`, reserved at `max_per_call * price` against a currency
//! Limit on `plugin_units`, unpriced units refused with `plugin_price_unknown`
//! under a currency Limit, and over-maximum reports recorded in full, flagged
//! in the report, and stopping the Run when they cross a Limit (ADR 0033,
//! issue 940).

use std::sync::Arc;

use crate::sync::Mutex;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::run::warning::{PLUGIN_FIGURES, PLUGIN_QUANTITY_ABOVE_MAXIMUM};
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
use crate::runs::RunSpendAccountant;
use crate::settings::{Layers, resolve};

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
                let (instance, peer) = PluginInstance::start_in_memory(
                    owner,
                    InMemoryPlugin {
                        manifest: manifest.clone(),
                        product: Product::Collie,
                        fetch,
                        clock,
                        spend: Some(RunSpendAccountant::new(
                            parts.charging.clone(),
                            parts.warnings.clone(),
                        )),
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
    let env = TestEnv::new();
    let calls_received = Arc::new(Mutex::new(0));
    let contribution =
        spend_contribution(manifest, script, calls_to_make, Arc::clone(&calls_received));
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
