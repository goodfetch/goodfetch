//! The run file's WAL under its bound (#2323), through the writer's
//! interface on real SQLite: a reader that holds an old snapshot blocks its
//! checkpoints, which the writer reports, and once the reader ends the
//! writer starts the log again and the file is truncated to its limit.

use goodfetch_test_support::TestEnv;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;

use super::{blob, insert, items, size_of, with_writer_options};
use crate::metrics::{METER, Metrics};
use crate::run_state::{RunStateOptions, open_reader};

/// The checkpoint threshold of these tests' run files, 16 pages.
const CHECKPOINT_AFTER: u64 = 65_536;

/// Options whose run file checkpoints past [`CHECKPOINT_AFTER`] and counts
/// on a meter `registry` renders, with the meter's provider.
fn metered() -> (RunStateOptions, prometheus::Registry, SdkMeterProvider) {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let options = RunStateOptions {
        wal_checkpoint_after: CHECKPOINT_AFTER,
        metrics: Some(Metrics::new(&provider.meter(METER))),
        ..RunStateOptions::default()
    };
    (options, registry, provider)
}

/// The value of the series `name` of `registry`, 0 without it.
fn value_of(registry: &prometheus::Registry, name: &str) -> f64 {
    registry
        .gather()
        .iter()
        .find(|family| family.name() == name)
        .and_then(|family| family.get_metric().first())
        .map_or(0.0, |metric| {
            if name.ends_with("_total") {
                metric.get_counter().get_value()
            } else {
                metric.get_gauge().get_value()
            }
        })
}

/// A reader that holds a read transaction across many commits keeps the
/// WAL from being copied whole or started again: the checkpoints it blocks
/// are counted while it holds, and once it ends the WAL is back under
/// twice its checkpoint threshold within two commits.
///
/// Covers: story:3-25
#[test]
fn the_wal_is_back_under_its_bound_once_a_reader_that_held_a_snapshot_ends() {
    let env = TestEnv::new();
    let (options, registry, _provider) = metered();
    let path = env.folder().join("run.sqlite");

    let (held, blocked, after) = with_writer_options(&env, &options, async |state| {
        state.write(insert(0)).await.unwrap();
        let reader = open_reader(&path, None).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        items(&reader).unwrap();
        for _ in 0..8 {
            state.write(blob(1)).await.unwrap();
            tokio::task::yield_now().await;
        }
        let held = size_of(&env, "run.sqlite-wal");
        let blocked = value_of(&registry, "goodfetch_run_state_blocked_checkpoints_total");
        reader.execute_batch("ROLLBACK").unwrap();
        drop(reader);
        for value in 1..=2 {
            state.write(insert(value)).await.unwrap();
        }
        (held, blocked, size_of(&env, "run.sqlite-wal"))
    });

    assert!(held >= 8 * 1_048_576, "{held}");
    assert!(blocked >= 1.0, "{blocked}");
    assert!(after <= 2 * CHECKPOINT_AFTER, "{after}");
}

/// A WAL that started again is truncated to its size limit, the checkpoint
/// threshold, at its next commit, and `goodfetch.run_state.wal.size`
/// follows it down.
#[test]
fn a_wal_that_started_again_is_truncated_to_its_size_limit() {
    let env = TestEnv::new();
    let (options, registry, _provider) = metered();

    let (grown, counted, truncated, after) = with_writer_options(&env, &options, async |state| {
        state.write(blob(2)).await.unwrap();
        let grown = size_of(&env, "run.sqlite-wal");
        let counted = value_of(&registry, "goodfetch_run_state_wal_size_bytes");
        state.write(insert(1)).await.unwrap();
        (
            grown,
            counted,
            size_of(&env, "run.sqlite-wal"),
            value_of(&registry, "goodfetch_run_state_wal_size_bytes"),
        )
    });

    assert!(grown >= 2 * 1_048_576, "{grown}");
    assert!(counted >= 2.0 * 1_048_576.0, "{counted}");
    assert_eq!(truncated, CHECKPOINT_AFTER);
    assert!(after <= 65_536.0, "{after}");
}
