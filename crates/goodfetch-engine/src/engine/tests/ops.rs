//! The Daemon's health and the runtime's metrics through the engine's
//! public interface over the in-memory network (spec #213's Seam 3), on the
//! paused `Clock` (ADR 0050): `daemon.health` answers live and ready, or
//! why the Daemon cannot take a Run, as its ops listener's probes check,
//! reading the filesystems of its essential writes (run state's, the
//! install database's in the per-install layout, and the Goodfetch home's)
//! afresh;
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
use super::disk_view::DiskView;
use super::layout::install_file;
use super::run::{call, run_folder};
use super::{engine, engine_with};
use crate::daemon::DaemonOptions;
use crate::metrics::{METER, Metrics};
use crate::ops::{ConfiguredBrowsers, Ops};
use crate::run_state::RunStateOptions;
use crate::run_state::install::InstallDatabase;

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

/// `goodfetch.counted_work`'s labels take exactly the values of the rows it
/// counts (ADR 0140), so a new row declares its path and its work.
#[test]
fn the_counted_works_labels_are_the_protocols_own() {
    use goodfetch_counted_work::Row;
    use goodfetch_protocol::ops::COUNTED_WORK;

    let declared = |label: &str| -> BTreeSet<&str> {
        COUNTED_WORK
            .labels
            .iter()
            .find(|declared| declared.name == label)
            .unwrap()
            .values
            .iter()
            .copied()
            .collect()
    };

    assert_eq!(
        (
            Row::ALL.map(Row::path).into_iter().collect::<BTreeSet<_>>(),
            Row::ALL.map(Row::work).into_iter().collect::<BTreeSet<_>>(),
        ),
        (declared("shared_path"), declared("work")),
    );
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

const MB: u64 = 1_000_000;

/// Where a Daemon's storage readiness tests put the filesystems: the
/// Goodfetch home, its per-major folder, which holds the install database,
/// and the run folders, each on a filesystem of its own.
#[derive(Clone, Copy, Debug)]
enum Critical {
    /// The per-major folder's filesystem, in the per-install layout.
    InstallDatabase,
    /// The Goodfetch home's, in the per-run layout.
    Home,
}

/// A Daemon whose essential writes are on a filesystem at `critical` is
/// not ready, and its start of a Run is refused, naming that folder,
/// wherever the run folders are (ADRs 0050 and 0122): in the per-install
/// layout the filesystem of the install database, which holds run state,
/// and in either layout the Goodfetch home's, which holds the Usage ledger
/// and the Politeness file, without which no Run goes on.
///
/// Covers: story:3-102, story:3-104, operation:daemon.health, setting:storage.critical_free
#[rstest]
#[case::install_database(Critical::InstallDatabase)]
#[case::home(Critical::Home)]
fn a_daemon_whose_essential_writes_are_on_a_critical_filesystem_is_not_ready(
    #[case] critical: Critical,
) {
    let env = TestEnv::new();
    // One sample a minute: the Daemon stays through its idle timeout on the
    // paused clock, and each sample is real work a loaded host pays for.
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings]\ngovernor.sample_interval = 60000\n\
         storage.low_free = 2000000\nstorage.critical_free = 1000000\n",
    )
    .unwrap();
    let view = DiskView::default();
    let (home, per_major) = (env.home().to_owned(), env.home().join("1"));
    let runs = per_major.join("runs");
    let (critical_folder, install) = match critical {
        Critical::InstallDatabase => {
            view.mount(&home, 100 * MB, 50 * MB);
            view.mount(&per_major, 100 * MB, MB / 2);
            std::fs::create_dir_all(&runs).unwrap();
            let install =
                InstallDatabase::open(&install_file(&env), &runs, &RunStateOptions::default())
                    .unwrap();
            (per_major.clone(), Some(install))
        }
        Critical::Home => {
            view.mount(&home, 100 * MB, MB / 2);
            view.mount(&per_major, 100 * MB, 50 * MB);
            (home.clone(), None)
        }
    };
    view.mount(&runs, 100 * MB, 50 * MB);
    let options = DaemonOptions {
        install,
        ..the_pick()
    };

    let (_, (health, started)) = daemon(
        &engine(Product::Collie).with_governor(view.system()),
        &env,
        options,
        |dialer| async move {
            let mut client = opened(&dialer, cli_hello()).await;
            tokio::time::sleep(Duration::from_secs(61)).await;
            let health = call(&mut client, 1, "daemon.health", json!({})).await;
            let started = call(&mut client, 2, "run.start", json!({ "run_id": "refused" })).await;
            if started["result"].is_object() {
                // A start this test expects refused, finished all the same.
                call(&mut client, 3, "run.finish", json!({ "run_id": "refused" })).await;
            }
            client.close().await;
            (health["result"].clone(), started)
        },
    );

    assert_eq!(
        (
            health,
            &started["error"]["data"]["code"],
            &started["error"]["data"]["path"],
        ),
        (
            json!({ "live": true, "ready": false, "unready": ["storage_critical"] }),
            &json!("resources_exhausted"),
            &json!(critical_folder.display().to_string()),
        ),
        "{started}"
    );
}

/// Readiness reads run state's filesystem afresh, as a start does, rather
/// than wait for the governor's next sample: a filesystem that crossed
/// `storage.critical_free` since the last sample fails it at once.
///
/// Covers: story:3-102, operation:daemon.health, setting:storage.critical_free
#[test]
fn readiness_reads_run_states_filesystem_afresh_between_two_samples() {
    let env = TestEnv::new();
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings]\ngovernor.sample_interval = 3600000\n\
         storage.low_free = 2000000\nstorage.critical_free = 1000000\n",
    )
    .unwrap();
    let view = DiskView::default();
    let disk = view.mount(env.home(), 100 * MB, 50 * MB);
    let filled = view.clone();

    let (_, (before, after)) = daemon(
        &engine(Product::Collie).with_governor(view.system()),
        &env,
        the_pick(),
        |dialer| async move {
            let mut client = opened(&dialer, cli_hello()).await;
            let before = call(&mut client, 1, "daemon.health", json!({})).await;
            filled.set_free(disk, MB / 2);
            let after = call(&mut client, 2, "daemon.health", json!({})).await;
            client.close().await;
            (before["result"].clone(), after["result"].clone())
        },
    );

    assert_eq!(
        (before, after),
        (
            json!({ "live": true, "ready": true, "unready": [] }),
            json!({ "live": true, "ready": false, "unready": ["storage_critical"] }),
        )
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
pub(super) fn series(registry: &prometheus::Registry) -> BTreeSet<String> {
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

    // What the run file does is `xtask counts`' to hold, not this test's.
    let beside_the_run_file = |series: BTreeSet<String>| -> BTreeSet<String> {
        series
            .into_iter()
            .filter(|line| !line.contains(r#"shared_path="run_state""#))
            .collect()
    };
    assert_eq!(
        (
            beside_the_run_file(while_running),
            beside_the_run_file(series(&registry)),
        ),
        (
            BTreeSet::from([
                r#"goodfetch_client_connected{surface="cli"} 1"#.to_owned(),
                r#"goodfetch_counted_work_total{shared_path="page_analysis",work="parses"} 0"#.to_owned(),
                "goodfetch_daemon_draining{} 0".to_owned(),
                r#"goodfetch_run_active{run_kind="collie"} 1"#.to_owned(),
                r#"goodfetch_run_ended_total{completeness="complete",lifecycle="finished",run_kind="collie",verdict="verified"} 1"#.to_owned(),
            ]),
            BTreeSet::from([
                r#"goodfetch_client_connected{surface="cli"} 0"#.to_owned(),
                r#"goodfetch_counted_work_total{shared_path="page_analysis",work="parses"} 0"#.to_owned(),
                "goodfetch_daemon_draining{} 1".to_owned(),
                r#"goodfetch_run_active{run_kind="collie"} 0"#.to_owned(),
                r#"goodfetch_run_ended_total{completeness="complete",lifecycle="finished",run_kind="collie",verdict="verified"} 1"#.to_owned(),
                r#"goodfetch_run_ended_total{completeness="incomplete",lifecycle="stopped",run_kind="collie",verdict="verified"} 1"#.to_owned(),
            ]),
        )
    );
}
