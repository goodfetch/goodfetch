//! A crawl's Run ID names it for good across restarts, stops, crashes and
//! refusals (ADRs 0093, 0102, 0103 and 0142; issue 1493): a clean `stop()`
//! resumes with `started_as() == StartedAs::Resumed` (logged at info) and
//! counts no `handler_lost`, an ended Run returns unchanged with
//! `StartedAs::Ended` (logged at warning), an in-flight attempt after a
//! crash at `full` durability counts `handler_lost` against `crawl.retry.max`
//! with no Record committed twice, and invalid resumes or conflicts arrive
//! as their typed error codes with details before any work.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests collect handler counts and tracing events on the real runtime, not under loom"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::TryStreamExt as _;
use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::Settings;
use goodfetch_foxhound::settings::crawl::{handler, retry, scope};
use goodfetch_foxhound::settings::run_state::{self, Durability};
use goodfetch_foxhound::{
    Crawl, CrawlContext, Dataset, DatasetDeclaration, Error, ErrorCode, Foxhound, Label, Limits,
    RunId, Schema, Seed, UsageKind, error_code,
};
use goodfetch_protocol::home::{RUN_FILE, RUNS, per_major};
use goodfetch_protocol::run::{Lifecycle, ReachedLimit, RunConflictDetails, StartInput, StartedAs};
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

use super::crawl::{options, site_settings};

/// The Proving ground's seed for these tests.
const SEED: u64 = 1493;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Item {
    id: String,
    title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct ExtendedItem {
    id: String,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

/// Captures `goodfetch::client::run` tracing events `(level, run_id, started_as)`.
#[derive(Clone, Default)]
struct RunEventCapture(Arc<Mutex<Vec<(Level, String, String)>>>);

impl RunEventCapture {
    fn events(&self) -> Vec<(Level, String, String)> {
        self.0.lock().unwrap().clone()
    }
}

struct FieldCollector {
    run_id: String,
    started_as: String,
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "run_id" {
            value.clone_into(&mut self.run_id);
        } else if field.name() == "started_as" {
            value.clone_into(&mut self.started_as);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "run_id" {
            formatted.trim_matches('"').clone_into(&mut self.run_id);
        } else if field.name() == "started_as" {
            formatted.trim_matches('"').clone_into(&mut self.started_as);
        }
    }
}

impl Subscriber for RunEventCapture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::INFO
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != "goodfetch::client::run" {
            return;
        }
        let mut fields = FieldCollector {
            run_id: String::new(),
            started_as: String::new(),
        };
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), fields.run_id, fields.started_as));
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// Finds `run.sqlite` for `run_id` inside `env`'s Goodfetch home.
fn run_file_path(env: &TestEnv, run_id: &str) -> PathBuf {
    let version =
        goodfetch_protocol::connection::ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    per_major(env.home(), &version)
        .join(RUNS)
        .join(run_id)
        .join(RUN_FILE)
}

/// Copies `src` over `dst` and removes any SQLite WAL/SHM sidecars beside
/// `dst`, restoring `dst` to the exact committed state in `src`.
#[expect(
    clippy::disallowed_methods,
    reason = "integration test replaces the test Run's SQLite file in TestEnv"
)]
fn restore_run_file(src: &Path, dst: &Path) {
    let _ = std::fs::remove_file(dst.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(dst.with_extension("sqlite-shm"));
    std::fs::copy(src, dst).unwrap();
}

/// Builds a two-Label crawl where `first` commits Record `"one"` and enqueues
/// `second_url`, and `second` signals `on_second` then parks in flight.
fn stopping_crawl(
    second_url: String,
    on_second: Arc<dyn Fn() + Send + Sync>,
) -> (Crawl<'static>, Label<()>) {
    let mut crawl = Crawl::new();
    let items = crawl
        .dataset(
            DatasetDeclaration::<Item>::new("items")
                .schema(&Schema::derived())
                .key(&["id"]),
        )
        .unwrap();
    let first_label = crawl.label::<()>("first").unwrap();
    let second_label = crawl.label::<()>("second").unwrap();
    let second_for_enqueue = second_label.clone();

    crawl
        .handle(&first_label, move |ctx: CrawlContext<()>| {
            let items = items.clone();
            let second_label = second_for_enqueue.clone();
            let second_url = second_url.clone();
            async move {
                ctx.push(
                    &items,
                    &Item {
                        id: "one".to_owned(),
                        title: "First".to_owned(),
                    },
                )
                .await?;
                ctx.enqueue(&second_url, &second_label, ()).await?;
                Ok(())
            }
        })
        .unwrap();
    crawl
        .handle(&second_label, move |_ctx: CrawlContext<()>| {
            let on_second = Arc::clone(&on_second);
            async move {
                on_second();
                std::future::pending::<()>().await;
                Ok(())
            }
        })
        .unwrap();
    (crawl, first_label)
}

/// Builds a resuming two-Label crawl recording `first` calls and `second`'s
/// attempt number, pushing Record `"two"` when `second` runs.
fn resuming_crawl() -> (
    Crawl<'static>,
    Label<()>,
    Dataset<Item>,
    Arc<AtomicU32>,
    Arc<AtomicU32>,
) {
    let first_calls = Arc::new(AtomicU32::new(0));
    let second_attempt = Arc::new(AtomicU32::new(0));
    let first_calls_h = Arc::clone(&first_calls);
    let second_attempt_h = Arc::clone(&second_attempt);

    let mut crawl = Crawl::new();
    let items = crawl
        .dataset(
            DatasetDeclaration::<Item>::new("items")
                .schema(&Schema::derived())
                .key(&["id"]),
        )
        .unwrap();
    let items_second = items.clone();
    let first_label = crawl.label::<()>("first").unwrap();
    let second_label = crawl.label::<()>("second").unwrap();

    crawl
        .handle(&first_label, move |_ctx: CrawlContext<()>| {
            let first_calls = Arc::clone(&first_calls_h);
            async move {
                first_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .unwrap();
    crawl
        .handle(&second_label, move |ctx: CrawlContext<()>| {
            let items = items_second.clone();
            let second_attempt = Arc::clone(&second_attempt_h);
            async move {
                second_attempt.store(ctx.attempt(), Ordering::SeqCst);
                ctx.push(
                    &items,
                    &Item {
                        id: "two".to_owned(),
                        title: "Second".to_owned(),
                    },
                )
                .await?;
                Ok(())
            }
        })
        .unwrap();
    (crawl, first_label, items, first_calls, second_attempt)
}

/// Covers: story:11-127, story:3-176, story:3-177, operation:crawl.start, operation:run.started_as, operation:run.stop
#[test]
fn clean_stop_resumes_without_handler_lost_and_ended_logs_warning() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let first_url = site.url("shop.test", "/book");
    let second_url = site.url("shop.test", "/empty");
    let run_id = RunId::parse("resume-clean-stop").unwrap();

    let logs = RunEventCapture::default();
    let subscriber = logs.clone();

    real_time_runtime().block_on(async {
        let _guard = tracing::subscriber::set_default(subscriber);
        let run_settings = Settings::new()
            .set(run_state::DURABILITY, Durability::Full)
            .set(handler::CONCURRENCY, 1)
            .set(retry::MAX, 2);

        // 1. Start fresh, commit page 1, stop cleanly while page 2 is in flight.
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let in_flight = Arc::new(Notify::new());
        let in_flight_tx = Arc::clone(&in_flight);
        let (crawl, first_label) = stopping_crawl(
            second_url.clone(),
            Arc::new(move || in_flight_tx.notify_one()),
        );
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &first_label, ())])
            .run_id(&run_id)
            .settings(run_settings.clone())
            .await
            .unwrap();
        assert_eq!(run.started_as(), StartedAs::New);

        tokio::select! {
            served = crawl.serve(&run) => panic!("crawl ended unexpectedly: {served:?}"),
            () = in_flight.notified() => {}
        }
        let stop_report = run.stop().await.unwrap().into_inner();
        assert_eq!(stop_report.report.lifecycle, Lifecycle::Stopped);
        foxhound.close().await.unwrap();

        // 2. Resume under the same Run ID: `started_as == Resumed`, info logged,
        // page 1 is NOT fetched again, and page 2 runs on attempt 1 (no `handler_lost`).
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl2, first2, items2, first_calls, second_attempt) = resuming_crawl();
        let resumed = foxhound
            .start_crawl(&crawl2, [Seed::new(&first_url, &first2, ())])
            .run_id(&run_id)
            .settings(run_settings.clone())
            .await
            .unwrap();
        assert_eq!(resumed.started_as(), StartedAs::Resumed);

        let finished = crawl2.serve(&resumed).await.unwrap();
        assert_eq!(finished.report.lifecycle, Lifecycle::Finished);
        assert_eq!(first_calls.load(Ordering::SeqCst), 0);
        assert_eq!(second_attempt.load(Ordering::SeqCst), 1);
        let by_error = &finished.requests.as_ref().unwrap().by_error_code;
        assert!(
            !by_error.iter().any(|e| e.name == "handler_lost"),
            "clean stop must count no handler_lost: {by_error:?}"
        );

        let records: Vec<String> = resumed
            .records(&items2)
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.value.id)
            .collect();
        assert_eq!(records, vec!["one".to_owned(), "two".to_owned()]);

        // 3. Starting again under the ended Run's ID returns it unchanged with
        // `started_as == Ended`, logs a warning, and invokes no handler.
        let (crawl3, first3, _items3, ended_first, ended_second) = resuming_crawl();
        let ended = foxhound
            .start_crawl(&crawl3, [Seed::new(&first_url, &first3, ())])
            .run_id(&run_id)
            .settings(run_settings)
            .await
            .unwrap();
        assert_eq!(ended.started_as(), StartedAs::Ended);
        let ended_report = ended.report().await.unwrap().into_inner();
        assert_eq!(ended_report.report.lifecycle, Lifecycle::Finished);
        assert_eq!(ended_first.load(Ordering::SeqCst), 0);
        assert_eq!(ended_second.load(Ordering::SeqCst), 0);

        foxhound.close().await.unwrap();
    });

    assert_eq!(
        logs.events(),
        vec![
            (
                Level::INFO,
                "resume-clean-stop".to_owned(),
                "resumed".to_owned(),
            ),
            (
                Level::WARN,
                "resume-clean-stop".to_owned(),
                "ended".to_owned(),
            ),
        ]
    );
}

/// Covers: story:3-176, story:11-127, operation:crawl.start, error:handler_lost
#[test]
fn crash_in_flight_attempt_counts_handler_lost_against_retry_max() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let first_url = site.url("shop.test", "/book");
    let second_url = site.url("shop.test", "/empty");
    let run_id = RunId::parse("resume-crash-lost").unwrap();
    let checkpoint_path = env.folder().join("in-flight-checkpoint.sqlite");

    real_time_runtime().block_on(async {
        let base_settings = Settings::new()
            .set(run_state::DURABILITY, Durability::Full)
            .set(handler::CONCURRENCY, 1)
            .set(retry::MAX, 1);

        // 1. Capture committed run state at `full` durability while page 2 is in flight.
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (in_flight_tx, in_flight_rx) = oneshot::channel::<()>();
        let in_flight_tx = Arc::new(Mutex::new(Some(in_flight_tx)));
        let (crawl, first_label) = stopping_crawl(
            second_url.clone(),
            Arc::new(move || {
                if let Some(tx) = in_flight_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }),
        );
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &first_label, ())])
            .run_id(&run_id)
            .settings(base_settings.clone())
            .await
            .unwrap();

        let cp_loc = checkpoint_path.display().to_string();
        tokio::select! {
            served = crawl.serve(&run) => panic!("crawl ended unexpectedly: {served:?}"),
            () = async {
                in_flight_rx.await.unwrap();
                let _: Value = foxhound
                    .call("run.checkpoint")
                    .params(json!({ "run_id": run_id.as_str(), "location": cp_loc }))
                    .await
                    .unwrap();
            } => {}
        }
        run.run().close().await.unwrap();
        foxhound.close().await.unwrap();

        let run_sqlite = run_file_path(&env, run_id.as_str());
        restore_run_file(&checkpoint_path, &run_sqlite);

        // 2. Resume with `crawl.retry.max = 1`: attempt 1 counts as `handler_lost`,
        // page 2 retries on attempt 2, and page 1 is never committed twice.
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl2, first2, items2, first_calls, second_attempt) = resuming_crawl();
        let resumed = foxhound
            .start_crawl(&crawl2, [Seed::new(&first_url, &first2, ())])
            .run_id(&run_id)
            .settings(base_settings)
            .await
            .unwrap();
        assert_eq!(resumed.started_as(), StartedAs::Resumed);
        let finished = crawl2.serve(&resumed).await.unwrap();
        assert_eq!(finished.report.lifecycle, Lifecycle::Finished);
        assert_eq!(first_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            second_attempt.load(Ordering::SeqCst),
            2,
            "in-flight attempt must count handler_lost against crawl.retry.max"
        );
        let records: Vec<Item> = resumed
            .records(&items2)
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.value)
            .collect();
        assert_eq!(records.len(), 2);
        foxhound.close().await.unwrap();

        // 3. Restore crashed state and resume with `crawl.retry.max = 0`:
        // the lost lease exhausts retries and fails page 2 with `handler_lost`.
        restore_run_file(&checkpoint_path, &run_sqlite);
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl3, first3, _items3, _fc, zero_attempt) = resuming_crawl();
        let no_retry = Settings::new()
            .set(run_state::DURABILITY, Durability::Full)
            .set(handler::CONCURRENCY, 1)
            .set(retry::MAX, 0);
        let resumed_zero = foxhound
            .start_crawl(&crawl3, [Seed::new(&first_url, &first3, ())])
            .run_id(&run_id)
            .settings(no_retry)
            .await
            .unwrap();
        assert_eq!(resumed_zero.started_as(), StartedAs::Resumed);
        let report_zero = crawl3.serve(&resumed_zero).await.unwrap();
        assert_eq!(zero_attempt.load(Ordering::SeqCst), 0);
        let by_error = &report_zero.requests.as_ref().unwrap().by_error_code;
        assert!(
            by_error
                .iter()
                .any(|e| e.name == "handler_lost" && e.count == 1),
            "expected handler_lost failure with retry.max=0: {by_error:?}"
        );
        foxhound.close().await.unwrap();
    });
}

/// Covers: story:3-167, story:3-177, operation:crawl.start, operation:run.start, error:run_in_use, error:run_conflict, error:run_state_unavailable, error:limit_reached
#[test]
fn start_refuses_in_use_conflict_unavailable_and_limit_reached() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let first_url = site.url("shop.test", "/book");
    let other_url = site.url("shop.test", "/empty");

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();

        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<()>| async {
                std::future::pending::<()>().await;
                Ok(())
            })
            .unwrap();

        // 1. `run_in_use` while the first start is live.
        let live_id = RunId::parse("live-conflict").unwrap();
        let live = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &page, ())])
            .run_id(&live_id)
            .await
            .unwrap();
        let in_use: Error = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &page, ())])
            .run_id(&live_id)
            .await
            .unwrap_err();
        assert_eq!(in_use.code(), ErrorCode::RUN_IN_USE);
        assert!(in_use.retryable());

        live.run().close().await.unwrap();

        // 2. `run_conflict` on changed seeds, listing `Seeds` in `differences`.
        let seed_conflict: Error = foxhound
            .start_crawl(&crawl, [Seed::new(&other_url, &page, ())])
            .run_id(&live_id)
            .await
            .unwrap_err();
        assert_eq!(seed_conflict.code(), ErrorCode::RUN_CONFLICT);
        let seed_details: RunConflictDetails = seed_conflict.details().unwrap();
        assert!(
            seed_details.differences.contains(&StartInput::Seeds),
            "expected seeds difference: {:?}",
            seed_details.differences
        );

        // 3. `run_conflict` on a changed Plan file.
        foxhound.close().await.unwrap();
        let plain = Foxhound::connect(super::options(&env)).await.unwrap();
        let plan_id = RunId::parse("plan-conflict").unwrap();
        let plan_run = plain
            .start()
            .run_id(&plan_id)
            .plan("first.plan.json")
            .await
            .unwrap();
        plan_run.stop().await.unwrap();
        let plan_conflict: Error = plain
            .start()
            .run_id(&plan_id)
            .plan("second.plan.json")
            .await
            .unwrap_err();
        assert_eq!(plan_conflict.code(), ErrorCode::RUN_CONFLICT);
        let plan_details: RunConflictDetails = plan_conflict.details().unwrap();
        assert!(
            plan_details.differences.contains(&StartInput::Plan),
            "expected plan difference: {:?}",
            plan_details.differences
        );
        plain.close().await.unwrap();

        // 4. `run_state_unavailable` when the run folder on disk is missing.
        let run_dir = run_file_path(&env, live_id.as_str())
            .parent()
            .unwrap()
            .to_path_buf();
        #[expect(
            clippy::disallowed_methods,
            reason = "integration test removes the Run folder in TestEnv to prove missing state refuses to start afresh"
        )]
        std::fs::remove_dir_all(&run_dir).unwrap();
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let unavailable: Error = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &page, ())])
            .run_id(&live_id)
            .await
            .unwrap_err();
        assert_eq!(unavailable.code(), ErrorCode::RUN_STATE_UNAVAILABLE);
        assert!(!unavailable.retryable());
        foxhound.close().await.unwrap();

        // 5. `limit_reached` with a used-up Limit carries `ReachedLimit` details.
        let capped_env = TestEnv::new();
        let capped = Foxhound::connect(
            options(&capped_env, &site).limits(Limits::new().count(UsageKind::REQUESTS, 0)),
        )
        .await
        .unwrap();
        let limit_err: Error = capped
            .start_crawl(&crawl, [Seed::new(&first_url, &page, ())])
            .await
            .unwrap_err();
        assert_eq!(limit_err.code(), ErrorCode::LIMIT_REACHED);
        let reached: ReachedLimit = limit_err.details().unwrap();
        assert_eq!(reached.usage_kind, "requests");
        capped.close().await.unwrap();
    });
}

/// Covers: story:6-272, story:3-172, operation:crawl.start, error:label_unknown, error:dataset_unknown, error:dataset_key_invalid, error:setting_not_applicable
#[test]
fn resume_validates_labels_datasets_keys_and_fixed_scope_settings() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let first_url = site.url("shop.test", "/book");
    let second_url = site.url("shop.test", "/empty");
    let run_id = RunId::parse("resume-validation").unwrap();
    let origin = first_url.rsplit_once('/').unwrap().0;
    let scope_include = vec![format!("{origin}/**")];

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let in_flight = Arc::new(Notify::new());
        let in_flight_tx = Arc::clone(&in_flight);
        let (crawl, first_label) = stopping_crawl(
            second_url.clone(),
            Arc::new(move || in_flight_tx.notify_one()),
        );

        let run_settings = site_settings(&site)
            .set(run_state::DURABILITY, Durability::Full)
            .set(handler::CONCURRENCY, 1)
            .set(scope::INCLUDE, scope_include);

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &first_label, ())])
            .run_id(&run_id)
            .settings(run_settings.clone())
            .await
            .unwrap();
        tokio::select! {
            served = crawl.serve(&run) => panic!("crawl ended unexpectedly: {served:?}"),
            () = in_flight.notified() => {}
        }
        let stop_report = run.stop().await.unwrap().into_inner();
        assert_eq!(stop_report.report.lifecycle, Lifecycle::Stopped);

        // 1. Omitting `second` (which has a pending request) -> `label_unknown`.
        let mut missing_label = Crawl::new();
        missing_label
            .dataset(
                DatasetDeclaration::<Item>::new("items")
                    .schema(&Schema::derived())
                    .key(&["id"]),
            )
            .unwrap();
        let fl = missing_label.label::<()>("first").unwrap();
        missing_label
            .handle(&fl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
        let err = foxhound
            .start_crawl(&missing_label, [Seed::new(&first_url, &fl, ())])
            .run_id(&run_id)
            .settings(run_settings.clone())
            .await
            .unwrap_err();
        assert_eq!(err.code(), error_code::LABEL_UNKNOWN);

        // 2. Omitting `items` (which holds committed Records) -> `dataset_unknown`.
        let mut missing_ds = Crawl::new();
        let fl = missing_ds.label::<()>("first").unwrap();
        let sl = missing_ds.label::<()>("second").unwrap();
        missing_ds
            .handle(&fl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
        missing_ds
            .handle(&sl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
        let err = foxhound
            .start_crawl(&missing_ds, [Seed::new(&first_url, &fl, ())])
            .run_id(&run_id)
            .settings(run_settings.clone())
            .await
            .unwrap_err();
        assert_eq!(err.code(), error_code::DATASET_UNKNOWN);

        // 3. Changing `items`'s key from `["id"]` to `["title"]` -> `dataset_key_invalid`.
        let mut wrong_key = Crawl::new();
        wrong_key
            .dataset(
                DatasetDeclaration::<Item>::new("items")
                    .schema(&Schema::derived())
                    .key(&["title"]),
            )
            .unwrap();
        let fl = wrong_key.label::<()>("first").unwrap();
        let sl = wrong_key.label::<()>("second").unwrap();
        wrong_key
            .handle(&fl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
        wrong_key
            .handle(&sl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
        let err = foxhound
            .start_crawl(&wrong_key, [Seed::new(&first_url, &fl, ())])
            .run_id(&run_id)
            .settings(run_settings.clone())
            .await
            .unwrap_err();
        assert_eq!(err.code(), error_code::DATASET_KEY_INVALID);

        // 4. Changing fixed `crawl.scope.include` -> `setting_not_applicable`.
        let changed_scope = site_settings(&site)
            .set(run_state::DURABILITY, Durability::Full)
            .set(handler::CONCURRENCY, 1)
            .set(scope::INCLUDE, vec!["https://other.test/**".to_owned()]);
        let err = foxhound
            .start_crawl(&crawl, [Seed::new(&first_url, &first_label, ())])
            .run_id(&run_id)
            .settings(changed_scope)
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::SETTING_NOT_APPLICABLE);

        // 5. Adding a new Label `third` and widening `items`'s schema succeeds!
        let mut widened = Crawl::new();
        let items_w = widened
            .dataset(
                DatasetDeclaration::<ExtendedItem>::new("items")
                    .schema(&Schema::derived())
                    .key(&["id"]),
            )
            .unwrap();
        let items_w_h = items_w.clone();
        let fl = widened.label::<()>("first").unwrap();
        let sl = widened.label::<()>("second").unwrap();
        let tl = widened.label::<()>("third").unwrap();
        widened
            .handle(&fl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
        widened
            .handle(&sl, move |ctx: CrawlContext<()>| {
                let items = items_w_h.clone();
                async move {
                    ctx.push(
                        &items,
                        &ExtendedItem {
                            id: "two".to_owned(),
                            title: "Second".to_owned(),
                            note: Some("widened".to_owned()),
                        },
                    )
                    .await?;
                    Ok(())
                }
            })
            .unwrap();
        widened
            .handle(&tl, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();

        let resumed = foxhound
            .start_crawl(&widened, [Seed::new(&first_url, &fl, ())])
            .run_id(&run_id)
            .settings(run_settings)
            .await
            .unwrap();
        assert_eq!(resumed.started_as(), StartedAs::Resumed);
        let finished = widened.serve(&resumed).await.unwrap();
        assert_eq!(finished.report.lifecycle, Lifecycle::Finished);

        let all: Vec<ExtendedItem> = resumed
            .records(&items_w)
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.value)
            .collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].note, None);
        assert_eq!(all[1].note.as_deref(), Some("widened"));
        foxhound.close().await.unwrap();
    });
}
