//! The Daemon's health and the runtime's metrics through the engine's
//! public interface over the in-memory network (spec #213's Seam 3), on the
//! paused `Clock` (ADR 0050): `daemon.health` answers live and ready, or
//! why the Daemon cannot take a Run, as its ops listener's probes check;
//! and the meter the engine is given records its Clients, its Runs and its
//! drain, which the Prometheus exporter the executables use renders.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use rstest::rstest;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::daemon::{cli_hello, daemon, draining, opened, stuck, the_pick};
use super::run::{call, run_folder};
use super::{engine, engine_with};
use crate::daemon::DaemonOptions;
use crate::metrics::{METER, Metrics};
use crate::ops::{ConfiguredBrowsers, Ops};

mod listener;

const MINUTE: Duration = Duration::from_secs(60);

/// Browsers whose installation the test fixes.
struct Browsers(bool);

impl ConfiguredBrowsers for Browsers {
    fn installed(&self) -> bool {
        self.0
    }
}

/// `options`, its configured browsers `installed` or not.
fn with_browsers(installed: bool, options: DaemonOptions) -> DaemonOptions {
    DaemonOptions {
        ops: Ops {
            listener: None,
            browsers: Some(Arc::new(Browsers(installed))),
        },
        ..options
    }
}

/// What `daemon.health` answers a Client of a Daemon run in `env` with
/// `options`.
fn health_of(env: &TestEnv, options: DaemonOptions) -> Value {
    let (_, health) = daemon(
        &engine(Product::Collie),
        env,
        options,
        |dialer| async move {
            let mut client = opened(&dialer, cli_hello()).await;
            let health = call(&mut client, 1, "daemon.health", json!({})).await;
            client.close().await;
            health["result"].clone()
        },
    );
    health
}

/// Covers: story:3-102, story:3-104, operation:daemon.health
#[test]
fn a_daemon_that_can_take_a_run_is_live_and_ready() {
    let health = health_of(&TestEnv::new(), with_browsers(true, the_pick()));

    assert_eq!(
        health,
        json!({ "live": true, "ready": true, "unready": [] })
    );
}

/// What keeps a Daemon from taking a Run.
#[derive(Clone, Copy, Debug)]
enum Fault {
    BrowsersMissing,
    RunStateNotWritable,
}

/// A live Daemon that cannot take a Run says why, as `/readyz` checks it.
///
/// Covers: story:3-102, story:3-104, operation:daemon.health
#[rstest]
#[case::browsers_missing(Fault::BrowsersMissing, "browsers_missing")]
#[case::run_state_not_writable(Fault::RunStateNotWritable, "run_state_not_writable")]
fn a_daemon_that_cannot_take_a_run_is_live_and_not_ready_and_says_why(
    #[case] fault: Fault,
    #[case] reason: &str,
) {
    let env = TestEnv::new();
    let installed = match fault {
        Fault::BrowsersMissing => false,
        Fault::RunStateNotWritable => {
            // A file where the run folders go: no Run can be written there.
            let runs = run_folder(&env, "any").parent().unwrap().to_owned();
            std::fs::create_dir_all(runs.parent().unwrap()).unwrap();
            std::fs::write(&runs, "not a folder").unwrap();
            true
        }
    };

    let health = health_of(&env, with_browsers(installed, the_pick()));

    assert_eq!(
        health,
        json!({ "live": true, "ready": false, "unready": [reason] })
    );
}

/// A draining Daemon is live but takes no Run (ADR 0049), so it is not
/// ready.
///
/// Covers: story:3-102, operation:daemon.health
#[test]
fn a_draining_daemon_is_live_and_not_ready() {
    let engine = engine_with(Product::Collie, Some(stuck()));
    let asked = CancellationToken::new();
    let options = with_browsers(true, draining(&asked));

    let (_, health) = daemon(&engine, &TestEnv::new(), options, |dialer| async move {
        let mut client = opened(&dialer, cli_hello()).await;
        call(&mut client, 1, "test.stuck", json!({ "run_id": "stuck" })).await;
        asked.cancel();
        // The drain began, and waits for the stuck Run.
        tokio::time::sleep(MINUTE).await;
        let health = call(&mut client, 2, "daemon.health", json!({})).await;
        while client.next().await.is_some() {}
        health["result"].clone()
    });

    assert_eq!(
        health,
        json!({ "live": true, "ready": false, "unready": ["draining"] })
    );
}

/// Every series of `registry`, as `name{label="value",…} value`.
fn series(registry: &prometheus::Registry) -> BTreeSet<String> {
    registry
        .gather()
        .iter()
        .flat_map(|family| {
            family.get_metric().iter().map(move |metric| {
                let labels: Vec<String> = metric
                    .get_label()
                    .iter()
                    .map(|label| format!("{}=\"{}\"", label.name(), label.value()))
                    .collect();
                let value = match family.get_field_type() {
                    prometheus::proto::MetricType::COUNTER => metric.get_counter().get_value(),
                    _ => metric.get_gauge().get_value(),
                };
                format!("{}{{{}}} {value}", family.name(), labels.join(","))
            })
        })
        .collect()
}

/// The meter the engine is given records each Client while it is
/// connected, each Run while it is live and each Run's end by its three
/// axes, and whether the Daemon drains; Prometheus renders them under
/// `goodfetch_*`.
///
/// Covers: story:3-103
#[test]
fn the_daemon_records_its_clients_its_runs_and_its_drain_in_the_runtimes_metrics() {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let engine = engine(Product::Collie).with_metrics(Metrics::new(&provider.meter(METER)));
    let asked = CancellationToken::new();
    let scraped = registry.clone();

    let (_, while_running) = daemon(
        &engine,
        &TestEnv::new(),
        draining(&asked),
        |dialer| async move {
            let mut client = opened(&dialer, cli_hello()).await;
            call(&mut client, 1, "run.start", json!({ "run_id": "finished" })).await;
            call(
                &mut client,
                2,
                "run.finish",
                json!({ "run_id": "finished" }),
            )
            .await;
            call(&mut client, 3, "run.start", json!({ "run_id": "drained" })).await;
            let while_running = series(&scraped);
            asked.cancel();
            while client.next().await.is_some() {}
            while_running
        },
    );

    assert_eq!(
        (while_running, series(&registry)),
        (
            BTreeSet::from([
                r#"goodfetch_client_connected{surface="cli"} 1"#.to_owned(),
                "goodfetch_daemon_draining{} 0".to_owned(),
                r#"goodfetch_run_active{run_kind="collie"} 1"#.to_owned(),
                r#"goodfetch_run_ended_total{completeness="complete",lifecycle="finished",run_kind="collie",verdict="verified"} 1"#.to_owned(),
            ]),
            BTreeSet::from([
                r#"goodfetch_client_connected{surface="cli"} 0"#.to_owned(),
                "goodfetch_daemon_draining{} 1".to_owned(),
                r#"goodfetch_run_active{run_kind="collie"} 0"#.to_owned(),
                r#"goodfetch_run_ended_total{completeness="complete",lifecycle="finished",run_kind="collie",verdict="verified"} 1"#.to_owned(),
                r#"goodfetch_run_ended_total{completeness="incomplete",lifecycle="stopped",run_kind="collie",verdict="verified"} 1"#.to_owned(),
            ]),
        )
    );
}
