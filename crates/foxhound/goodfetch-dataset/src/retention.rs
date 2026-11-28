//! Dataset retention applied by the engine (ADRs 0016 and 0140): reads
//! `datasets.<name>.retention` from a crawl's resolved settings, runs an
//! initial prune pass when the Run starts and then on a timer task of the
//! Run's `tasks`, through the Run's `run_state` writer, never on a Record's
//! commit or an attempt. When no Dataset sets a retention period, no task is
//! spawned and no prune pass runs.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::Row;
use goodfetch_engine::StoredTime;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::{RunState, RunStateError};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::dataset::{DATASETS, read_dataset_settings};
use rusqlite::Connection;

/// Shortest interval between retention prune passes so a zero or very short
/// retention never busy-loops.
const MIN_PRUNE_INTERVAL: Duration = Duration::from_millis(250);

/// Longest interval between retention prune passes (daily, matching
/// `AuditLog::start_pruning`).
const MAX_PRUNE_INTERVAL: Duration = Duration::from_hours(24);

/// Extracts each Dataset's configured retention period from `settings`.
/// Datasets with no `retention` are omitted.
#[must_use]
pub fn configured(settings: &Resolved) -> BTreeMap<String, Duration> {
    let Some(entries) = settings.value(&DATASETS).as_object() else {
        return BTreeMap::new();
    };
    entries
        .iter()
        .filter_map(|(name, value)| {
            let entry = read_dataset_settings(value).ok()?;
            let retention = entry.retention?;
            Some((name.clone(), Duration::from_millis(retention)))
        })
        .collect()
}

/// Deletes committed Records older than each Dataset's retention period at
/// `now`, together with their `record_search` full-text index rows (with
/// `secure-delete` enabled on the index), and answers how many Records were
/// deleted.
///
/// # Errors
///
/// The run file's, when SQLite fails.
pub fn prune_expired(
    connection: &Connection,
    retentions: &BTreeMap<String, Duration>,
    now: SystemTime,
) -> rusqlite::Result<u64> {
    let mut deleted = 0_u64;
    let has_records: bool = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'records'")?
        .exists([])?;
    if !has_records {
        return Ok(0);
    }
    let has_search: bool = connection
        .prepare_cached(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'record_search'",
        )?
        .exists([])?;
    let mut secured = false;
    for (dataset, retention) in retentions {
        let Some(cutoff) = now.checked_sub(*retention) else {
            continue;
        };
        let expired: Vec<i64> = {
            let mut select = connection
                .prepare_cached("SELECT number, committed_at FROM records WHERE dataset = ?1")?;
            let rows = select.query_map([dataset.as_str()], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, StoredTime>(1)?.0))
            })?;
            let mut numbers = Vec::new();
            for row in rows {
                let (number, committed_at) = row?;
                if committed_at < cutoff {
                    numbers.push(number);
                }
            }
            numbers
        };
        if expired.is_empty() {
            continue;
        }
        if has_search && !secured {
            connection
                .prepare_cached(
                    "INSERT INTO record_search(record_search, rank) VALUES ('secure-delete', 1)",
                )?
                .execute([])?;
            secured = true;
        }
        let mut delete_record =
            connection.prepare_cached("DELETE FROM records WHERE dataset = ?1 AND number = ?2")?;
        let mut delete_search =
            if has_search {
                Some(connection.prepare_cached(
                    "DELETE FROM record_search WHERE dataset = ?1 AND number = ?2",
                )?)
            } else {
                None
            };
        for number in expired {
            if let Some(delete_search) = &mut delete_search {
                delete_search.execute((dataset.as_str(), number))?;
            }
            let rows = delete_record.execute((dataset.as_str(), number))?;
            deleted += u64::try_from(rows).unwrap_or(u64::MAX);
        }
    }
    Ok(deleted)
}

/// Starts Dataset retention pruning for a live crawl Run when any Dataset has
/// a `datasets.<name>.retention` period configured: runs an initial pass at
/// Run start and then periodically on a timer task of `identity`'s tasks,
/// through `state`'s writer (ADRs 0016 and 0140). When no Dataset sets a
/// retention period, spawns no task and runs no transaction.
pub fn start_pruning(settings: &Resolved, state: RunState, identity: &RunIdentity) {
    let retentions = configured(settings);
    if retentions.is_empty() {
        return;
    }
    let clock = identity.clock();
    let starter = identity.owner().clone();
    let run_id = identity.id().clone();
    let work = identity.metrics().counted_work().clone();
    identity.spawn(prune_task(retentions, state, clock, starter, run_id, work));
}

/// The periodic retention pruning loop run on a Run's task owner.
async fn prune_task(
    retentions: BTreeMap<String, Duration>,
    state: RunState,
    clock: goodfetch_engine::clock::Clock,
    starter: goodfetch_engine::tasks::TaskStarter,
    run_id: goodfetch_protocol::run::RunId,
    work: goodfetch_counted_work::CountedWork,
) {
    let shortest = retentions
        .values()
        .copied()
        .min()
        .unwrap_or(MAX_PRUNE_INTERVAL);
    let interval = (shortest / 2).clamp(MIN_PRUNE_INTERVAL, MAX_PRUNE_INTERVAL);
    let prune_loop = async {
        loop {
            let now = clock.wall();
            let retentions_pass = retentions.clone();
            let work_pass = work.clone();
            match state
                .write(move |connection| {
                    work_pass.count(Row::Prunes);
                    prune_expired(connection, &retentions_pass, now)
                })
                .await
            {
                Ok(_) => {}
                Err(RunStateError::Closed | RunStateError::Fenced { .. }) => break,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        %run_id,
                        "Dataset retention pruning could not run; it tries again at the next interval"
                    );
                }
            }
            clock.sleep(interval).await;
        }
    };
    tokio::select! {
        () = starter.cancelled() => {}
        () = prune_loop => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use goodfetch_counted_work::{CountedWork, Row};
    use goodfetch_engine::clock::Clock;
    use goodfetch_engine::mask::Masks;
    use goodfetch_engine::run_state::{RunState, RunStateOptions};
    use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
    use goodfetch_protocol::foxhound::crawl::PushedRecord;
    use goodfetch_protocol::foxhound::dataset::{DatasetDeclaration, RecordData};
    use goodfetch_protocol::foxhound::provenance::RecordProvenance;
    use goodfetch_protocol::run::RunId;
    use serde_json::json;

    use super::prune_task;
    use crate::{Committing, Datasets, Query, TABLES, commit, read};

    fn test_provenance() -> RecordProvenance {
        serde_json::from_value(json!({ "request_id": "req-1" })).unwrap()
    }

    async fn push_record(
        state: &RunState,
        datasets: &Datasets,
        dataset: &str,
        email: &str,
        at: SystemTime,
    ) {
        let checked = datasets
            .check(
                vec![PushedRecord {
                    dataset: dataset.to_owned(),
                    data: RecordData::of(&json!({ "email": email })).unwrap(),
                }],
                &CountedWork::default(),
            )
            .unwrap();
        state
            .write(move |connection| {
                let provenance = test_provenance();
                let masks = Masks::default();
                let work = CountedWork::default();
                commit(
                    connection,
                    Committing {
                        request_id: "req-1",
                        url: "https://example.test/a",
                        at: &at,
                        provenance: &provenance,
                        masks: &masks,
                        work: &work,
                        index: true,
                    },
                    &checked,
                )
                .map(drop)
            })
            .await
            .unwrap();
    }

    async fn count_records(state: &RunState, dataset: &'static str) -> (usize, i64) {
        state
            .write(move |connection| {
                let rows = read(
                    connection,
                    Query {
                        dataset,
                        after: 0,
                        through: None,
                        wall: None,
                    },
                    100,
                )?;
                let indexed: i64 = connection
                    .prepare_cached("SELECT COUNT(*) FROM record_search WHERE dataset = ?1")?
                    .query_row([dataset], |row| row.get(0))?;
                Ok((rows.len(), indexed))
            })
            .await
            .unwrap()
    }

    /// Records older than a Dataset's `datasets.<name>.retention` are deleted
    /// by the engine's timer task on a paused clock (together with their
    /// full-text index rows), while newer Records and Datasets with no
    /// retention are untouched; without any retention configured, no prune
    /// pass runs (`F2.prunes == 0`).
    ///
    /// Covers: story:4-120, setting:datasets
    #[test]
    fn records_older_than_dataset_retention_are_deleted_on_paused_clock_and_none_without_retention()
    {
        let env = goodfetch_test_support::TestEnv::in_memory();
        let file = env.folder().join("run.sqlite");
        let start_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let declare = |name: &str| DatasetDeclaration {
            name: name.to_owned(),
            schema: None,
            key: Vec::new(),
        };
        let datasets = Datasets::declare(&[declare("contacts"), declare("archive")]).unwrap();

        let (at_start, after_first_expiry, after_second_expiry, prunes_count) =
            goodfetch_test_support::runtime().block_on(async {
                let owner = TaskOwner::root(BlockingPolicy::Inline);
                #[expect(
                    clippy::disallowed_methods,
                    reason = "unit test runs on a paused clock (ADR 0059)"
                )]
                let clock = Clock::paused(start_time);
                let work = CountedWork::default();
                let options = RunStateOptions {
                    clock,
                    work: work.clone(),
                    ..RunStateOptions::default()
                };
                let state = RunState::open(file, &options, vec![TABLES], 1, &owner)
                    .await
                    .unwrap();
                let decls = datasets.clone();
                state
                    .write(move |connection| decls.record(connection))
                    .await
                    .unwrap();
                push_record(
                    &state,
                    &datasets,
                    "contacts",
                    "old@example.test",
                    clock.wall(),
                )
                .await;
                push_record(
                    &state,
                    &datasets,
                    "archive",
                    "keep@example.test",
                    clock.wall(),
                )
                .await;

                let retentions = BTreeMap::from([("contacts".to_owned(), Duration::from_secs(10))]);
                let starter = (*owner).clone();
                owner.spawn(prune_task(
                    retentions,
                    state.clone(),
                    clock,
                    starter,
                    RunId::parse("retained").unwrap(),
                    work.clone(),
                ));

                // Initial pass at t=0: neither Record is older than 10s.
                clock.sleep(Duration::from_secs(1)).await;
                let at_start = (
                    count_records(&state, "contacts").await,
                    count_records(&state, "archive").await,
                );

                // Commit a fresh Record at t=8s, then advance to t=13s:
                // the t=0 Record of `contacts` is 13s old (>10s) and deleted,
                // while the t=8s Record (5s old) and `archive` (no retention) stay.
                clock.sleep(Duration::from_secs(7)).await;
                push_record(
                    &state,
                    &datasets,
                    "contacts",
                    "new@example.test",
                    clock.wall(),
                )
                .await;
                clock.sleep(Duration::from_secs(8)).await;
                let after_first_expiry = (
                    count_records(&state, "contacts").await,
                    count_records(&state, "archive").await,
                );

                // Advance another 10s (to t=26s): the t=8s Record is now 18s old and deleted;
                // `archive` still holds its Record.
                clock.sleep(Duration::from_secs(10)).await;
                let after_second_expiry = (
                    count_records(&state, "contacts").await,
                    count_records(&state, "archive").await,
                );

                let prunes = work.read(Row::Prunes);
                drop(state);
                owner.stop().await;
                (at_start, after_first_expiry, after_second_expiry, prunes)
            });

        assert_eq!(at_start, ((1, 1), (1, 1)));
        assert_eq!(after_first_expiry, ((1, 1), (1, 1)));
        assert_eq!(after_second_expiry, ((0, 0), (1, 1)));
        assert!(
            prunes_count >= 2,
            "expected periodic prune passes, got {prunes_count}"
        );
    }

    #[test]
    fn configured_is_empty_when_no_dataset_sets_retention() {
        let empty = goodfetch_engine::settings::resolve(
            &goodfetch_engine::settings::Layers::default(),
            goodfetch_protocol::catalog::Product::Foxhound,
            goodfetch_protocol::catalog::Product::Foxhound,
        )
        .unwrap();
        assert!(super::configured(&empty).is_empty());
    }
}
