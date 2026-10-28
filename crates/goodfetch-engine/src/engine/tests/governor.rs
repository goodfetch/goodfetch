//! The engine's Resource governor through its public interface over the
//! in-memory network (spec #213's Seam 3), on the paused `Clock`, reading a
//! folder laid out as Linux's `/proc` that the test writes: its samples as
//! `engine.system_info` to the subscriptions that ask for them, its target,
//! pressure source and saturation as metrics, and its last resort in
//! `daemon.health` (ADRs 0040 and 0050).

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use serde_json::json;
use tempfile::TempDir;

use super::daemon::{cli_hello, daemon, opened, the_pick};
use super::events::credit;
use super::run::call;
use super::{attached_on, engine, hello};
use crate::engine::Engine;
use crate::governor::System;
use crate::metrics::{METER, Metrics};

const MIB: u64 = 1 << 20;

/// A `/proc` whose engine uses `used` of its cgroup's 256 MiB limit, on an
/// 8 GiB machine with 6 GiB available.
fn proc(used: u64) -> TempDir {
    let folder = TempDir::new().unwrap();
    let cgroup = folder.path().join("cgroup");
    let files = [
        (
            "proc/meminfo",
            "MemTotal: 8388608 kB\nMemAvailable: 6291456 kB\n".to_owned(),
        ),
        (
            "proc/self/smaps_rollup",
            format!("Pss: {} kB\n", used / 1024),
        ),
        (
            "proc/self/stat",
            "1 (engine) S 1 1 1 0 -1 0 0 0 0 0 0 0\n".to_owned(),
        ),
        ("proc/self/cgroup", "0::/pod.scope\n".to_owned()),
        (
            "proc/self/mountinfo",
            format!(
                "48 22 0:30 / {} rw - cgroup2 cgroup2 rw\n",
                cgroup.display()
            ),
        ),
        ("cgroup/pod.scope/memory.max", format!("{}\n", 256 * MIB)),
    ];
    for (path, text) in files {
        let path = folder.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    folder
}

/// Writes `settings` as the test home's user configuration, in the
/// folder of the tests' engines' major.
fn configure(env: &TestEnv, settings: &str) {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        format!("version = \"0.1\"\n\n[settings]\n{settings}"),
    )
    .unwrap();
}

fn governed(engine: Engine, proc: &Path) -> Engine {
    engine.with_governor(System::linux_files(proc.join("proc")))
}

/// A subscription that asks for `engine.system_info` takes each sample
/// with the governor's readings; one that does not never sees it, so its
/// Runs' axis changes come first.
///
/// Covers: operation:events.subscribe, event:engine.system_info
#[test]
fn a_subscription_that_asks_takes_the_governors_samples_and_no_other_does() {
    let env = TestEnv::new();
    configure(&env, "governor.sample_interval = 250\n");
    let proc = proc(100 * MIB);
    let engine = governed(engine(Product::Collie), proc.path());

    let (sample, plain) = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        let _ = client.next().await;
        client
            .send(&[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": { "system_info": true } }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "events.subscribe", "params": {} }),
                credit(1, 1),
                credit(2, 1),
            ])
            .await;
        let mut sample = None;
        while sample.is_none() {
            let message = client.next().await.unwrap();
            if message["params"]["id"] == 1 {
                sample = Some(message["params"]["events"][0].clone());
            }
            assert_ne!(message["params"]["id"], 2, "{message}");
        }
        client
            .send(&[json!({ "jsonrpc": "2.0", "id": 3, "method": "run.start", "params": { "run_id": "first" } })])
            .await;
        let plain = loop {
            let message = client.next().await.unwrap();
            if message["params"]["id"] == 2 {
                break message["params"]["events"][0]["event"].clone();
            }
        };
        client.close().await;
        while client.next().await.is_some() {}
        (sample.unwrap(), plain)
    });

    assert_eq!(
        (
            &sample["event"],
            &sample["data"]["level"],
            &sample["data"]["memory"]["allowance"],
            &sample["data"]["memory"]["allowance_source"],
            &sample["data"]["memory"]["used"],
            &sample["data"]["concurrency"]["browser_page"]["target"],
            plain,
        ),
        (
            &json!("engine.system_info"),
            &json!("normal"),
            &json!(256 * MIB),
            &json!("cgroup"),
            &json!(100 * MIB),
            &json!(32),
            json!("run.axes_changed"),
        )
    );
}

/// Every series of `registry` under `goodfetch_governor_`, and
/// `goodfetch_browser_memory_bytes`.
fn governor_series(registry: &prometheus::Registry) -> BTreeSet<String> {
    registry
        .gather()
        .iter()
        .filter(|family| {
            family.name().starts_with("goodfetch_governor_")
                || family.name() == "goodfetch_browser_memory_bytes"
        })
        .flat_map(|family| {
            family.get_metric().iter().map(move |metric| {
                let labels: Vec<String> = metric
                    .get_label()
                    .iter()
                    .map(|label| format!("{}=\"{}\"", label.name(), label.value()))
                    .collect();
                format!(
                    "{}{{{}}} {}",
                    family.name(),
                    labels.join(","),
                    metric.get_gauge().get_value()
                )
            })
        })
        .collect()
}

/// Each sample records every kind's target, the reading that holds the
/// governor above normal, its saturation, the highest share of a kind's
/// target in use, and what its browsers use, none here: under memory
/// pressure, each target halved and the source is memory.
///
/// Covers: story:3-53
#[test]
fn each_sample_records_the_targets_the_pressure_source_and_the_saturation() {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let env = TestEnv::new();
    configure(&env, "governor.sample_interval = 250\n");
    let proc = proc(230 * MIB);
    let engine = governed(engine(Product::Collie), proc.path())
        .with_metrics(Metrics::new(&provider.meter(METER)));
    let scraped = registry.clone();

    let series = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        let _ = client.next().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let series = governor_series(&scraped);
        client.close().await;
        while client.next().await.is_some() {}
        series
    });

    assert_eq!(
        series,
        BTreeSet::from(
            [
                "goodfetch_browser_memory_bytes{} 0",
                r#"goodfetch_governor_pressure_source_ratio{source="cpu"} 0"#,
                r#"goodfetch_governor_pressure_source_ratio{source="host_memory"} 0"#,
                r#"goodfetch_governor_pressure_source_ratio{source="memory"} 1"#,
                r#"goodfetch_governor_pressure_source_ratio{source="tokio"} 0"#,
                "goodfetch_governor_saturation_ratio{} 0",
                r#"goodfetch_governor_target{kind="browser_page"} 16"#,
                r#"goodfetch_governor_target{kind="http"} 100"#,
                r#"goodfetch_governor_target{kind="model_call"} 8"#,
            ]
            .map(str::to_owned)
        )
    );
}

/// A Daemon whose memory stayed critical at the governor's floor past
/// `governor.last_resort_after` is live but not ready, as `/readyz` checks
/// it.
///
/// Covers: story:3-102, operation:daemon.health
#[test]
fn a_daemon_at_its_governors_last_resort_is_live_and_not_ready() {
    let env = TestEnv::new();
    configure(
        &env,
        "governor.sample_interval = 250\ngovernor.last_resort_after = 1000\n\
         governor.http.ceiling = 1\ngovernor.browser_page.ceiling = 1\ngovernor.model_call.ceiling = 1\n",
    );
    let proc = proc(250 * MIB);
    let engine = governed(engine(Product::Collie), proc.path());

    let (_, health) = daemon(&engine, &env, the_pick(), |dialer| async move {
        let mut client = opened(&dialer, cli_hello()).await;
        let before = call(&mut client, 1, "daemon.health", json!({})).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let after = call(&mut client, 2, "daemon.health", json!({})).await;
        client.close().await;
        [before["result"].clone(), after["result"].clone()]
    });

    assert_eq!(
        health,
        [
            json!({ "live": true, "ready": true, "unready": [] }),
            json!({ "live": true, "ready": false, "unready": ["governor_last_resort"] }),
        ]
    );
}

/// The last resort stops one Run, checkpointed and resumable: `stopped`
/// and `incomplete` with `resource: "memory"`, the first of equals when no
/// Run has a browser; the governor then measures again before it stops
/// another, so once memory is freed the other Run goes on and finishes.
///
/// Covers: story:3-59, setting:governor.last_resort_after, operation:run.resume
#[test]
fn the_last_resort_stops_one_run_resumably_and_measures_again_before_another() {
    let env = TestEnv::new();
    configure(
        &env,
        "governor.sample_interval = 250\ngovernor.last_resort_after = 1000\n\
         governor.http.ceiling = 1\ngovernor.browser_page.ceiling = 1\ngovernor.model_call.ceiling = 1\n",
    );
    let proc = proc(250 * MIB);
    let engine = governed(engine(Product::Collie), proc.path());
    let rollup = proc.path().join("proc/self/smaps_rollup");

    let (stopped, other, resumed) = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        let _ = client.next().await;
        call(&mut client, 1, "run.start", json!({ "run_id": "first" })).await;
        call(&mut client, 2, "run.start", json!({ "run_id": "second" })).await;
        // Critical at the floor from the first sample, at 0 s: the last
        // resort at 1 s stops the first Run.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        std::fs::write(&rollup, format!("Pss: {} kB\n", 100 * MIB / 1024)).unwrap();
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let stopped = call(&mut client, 3, "run.finish", json!({ "run_id": "first" })).await;
        let other = call(&mut client, 4, "run.finish", json!({ "run_id": "second" })).await;
        let resumed = call(&mut client, 5, "run.resume", json!({ "run_id": "first" })).await;
        client.close().await;
        while client.next().await.is_some() {}
        (stopped, other, resumed)
    });

    assert_eq!(
        (
            &stopped["result"]["lifecycle"],
            &stopped["result"]["completeness"],
            &stopped["result"]["stop_reason"],
            stopped["result"]["checkpoint"].is_object(),
            &other["result"]["lifecycle"],
            &resumed["result"]["started_as"],
        ),
        (
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "resources", "resource": "memory" }),
            true,
            &json!("finished"),
            &json!("resumed"),
        ),
        "{stopped}\n{other}\n{resumed}"
    );
}
