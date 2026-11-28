//! Reading a crawl Run another program started: `open_crawl_run` and
//! `CrawlRunView` (ADRs 0102 and 0103; issue 1489), plus Rust's `records_after`,
//! `rejected_records_after`, `export` and `failed_requests` on `CrawlRun` and
//! `CrawlRunView` (async and blocking).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::TryStreamExt as _;
use goodfetch_foxhound::blocking;
use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::crawl::{self, handler};
use goodfetch_foxhound::{
    Crawl, CrawlContext, CrawlRunView, Dataset, DatasetDeclaration, ErrorCode, ExportFormat,
    Foxhound, RunId, Schema, Seed, error_code,
};
use goodfetch_protocol::home::{RUN_FILE, RUNS, per_major};
use goodfetch_protocol::run::StartedAs;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::crawl::{options, site_settings};

const SEED: u64 = 1489;

fn run_file_path(env: &TestEnv, run_id: &str) -> PathBuf {
    let version =
        goodfetch_protocol::connection::ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    per_major(env.home(), &version)
        .join(RUNS)
        .join(run_id)
        .join(RUN_FILE)
}

#[expect(
    clippy::disallowed_methods,
    reason = "integration test restores a saved Checkpoint SQLite file in TestEnv"
)]
fn restore_run_file(src: &Path, dst: &Path) {
    let _ = std::fs::remove_file(dst.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(dst.with_extension("sqlite-shm"));
    std::fs::copy(src, dst).unwrap();
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Item {
    id: String,
    title: String,
}

async fn resume_and_finish(env: &TestEnv, ground: &ProvingGround, first_url: &str, run_id: &RunId) {
    let second_client = Foxhound::connect(options(env, ground)).await.unwrap();
    let mut resume_crawl = Crawl::new();
    let resume_page = resume_crawl.label::<()>("page").unwrap();
    let resume_items = resume_crawl
        .dataset(DatasetDeclaration::<Item>::new("items").schema(&Schema::derived()))
        .unwrap();
    resume_crawl
        .handle(&resume_page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();

    let resumed_run = second_client
        .start_crawl(&resume_crawl, [Seed::new(first_url, &resume_page, ())])
        .run_id(run_id)
        .settings(
            site_settings(ground)
                .set(handler::CONCURRENCY, 1)
                .set(crawl::KEEP_ALIVE, true),
        )
        .await
        .unwrap();
    assert_eq!(resumed_run.started_as(), StartedAs::Resumed);

    let (served_report, finished_report) =
        tokio::join!(resume_crawl.serve(&resumed_run), resumed_run.finish());
    assert_eq!(served_report.unwrap().report.run_id, *run_id);
    assert_eq!(finished_report.unwrap().report.run_id, *run_id);

    let run_after_one: Vec<_> = resumed_run
        .records_after(&resume_items, 1)
        .try_collect()
        .await
        .unwrap();
    assert_eq!(run_after_one.len(), 1);
    assert_eq!(run_after_one[0].number, 2);

    second_client.close().await.unwrap();
}

async fn assert_view_after_finish(view: &CrawlRunView, items: &Dataset<Item>, run_id: &RunId) {
    let viewed_report = view.report().await.unwrap();
    assert_eq!(viewed_report.report.run_id, *run_id);

    let after_one: Vec<_> = view.records_after(items, 1).try_collect().await.unwrap();
    assert_eq!(after_one.len(), 1);
    assert_eq!(after_one[0].number, 2);
    assert_eq!(after_one[0].value.id, "item-2");

    let exported = view.export(items, ExportFormat::Json).await.unwrap();
    assert_eq!(exported.records(), 2);

    let outcomes: Vec<_> = view.outcomes().try_collect().await.unwrap();
    assert_eq!(outcomes.len(), 1);

    let failed = view.failed_requests().await.unwrap();
    assert_eq!(failed.len(), 0);

    let _trace = view.trace().await.unwrap();
}

async fn assert_interrupted_view(view: &CrawlRunView, items: &Dataset<Item>, run_id: &RunId) {
    let interrupted_records: Vec<_> = view.records(items).try_collect().await.unwrap();
    assert_eq!(interrupted_records.len(), 2);

    let report_err = view.report().await.unwrap_err();
    assert_eq!(report_err.code(), ErrorCode::RUN_INTERRUPTED);
    assert!(
        report_err
            .fix()
            .is_some_and(|fix| fix.contains(run_id.as_str())),
        "expected fix naming Run ID, got: {:?}",
        report_err.fix()
    );

    let stage: Option<String> = view.key_value_store().get("stage").await.unwrap();
    assert_eq!(stage.as_deref(), Some("one"));
    let progress: Vec<_> = view.progress().try_collect().await.unwrap();
    assert_ne!(progress.len(), 0);
}

/// Covers: story:11-1068, story:11-1109
#[test]
fn open_crawl_run_reads_live_ends_on_interrupted_and_never_blocks_resume() {
    let env = TestEnv::new();
    let ground = ProvingGround::start(&env, SEED).unwrap();
    let first_url = ground.start_url("catalog/1").unwrap();
    let run_id = RunId::parse("open-crawl-run-live").unwrap();

    real_time_runtime().block_on(async {
        let writer = Foxhound::connect(options(&env, &ground)).await.unwrap();
        let reader = Foxhound::connect(options(&env, &ground)).await.unwrap();

        let first_committed = Arc::new(Notify::new());
        let first_committed_clone = Arc::clone(&first_committed);

        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        let items = crawl
            .dataset(DatasetDeclaration::<Item>::new("items").schema(&Schema::derived()))
            .unwrap();
        let items_for_handler = items.clone();
        crawl
            .handle(&page, move |ctx: CrawlContext<()>| {
                let items = items_for_handler.clone();
                let first_committed = Arc::clone(&first_committed_clone);
                async move {
                    ctx.push(
                        &items,
                        &Item {
                            id: "item-1".to_owned(),
                            title: "First".to_owned(),
                        },
                    )
                    .await?;
                    ctx.push(
                        &items,
                        &Item {
                            id: "item-2".to_owned(),
                            title: "Second".to_owned(),
                        },
                    )
                    .await?;
                    ctx.run()
                        .key_value_store()
                        .set("stage", &"one".to_owned())
                        .await?;
                    first_committed.notify_one();
                    Ok(())
                }
            })
            .unwrap();

        let run = writer
            .start_crawl(&crawl, [Seed::new(&first_url, &page, ())])
            .run_id(&run_id)
            .settings(
                site_settings(&ground)
                    .set(handler::CONCURRENCY, 1)
                    .set(crawl::KEEP_ALIVE, true),
            )
            .await
            .unwrap();

        let mut reader_crawl = Crawl::new();
        let reader_items = reader_crawl
            .dataset(DatasetDeclaration::<Item>::new("items").schema(&Schema::derived()))
            .unwrap();
        let view = reader.open_crawl_run(&run_id).unwrap();
        assert_eq!(view.id(), &run_id);

        let checkpoint_path = env.folder().join("interrupted-checkpoint.sqlite");
        let (_, ()) = tokio::join!(crawl.serve(&run), async {
            first_committed.notified().await;
            let mut live_records = view.records(&reader_items);
            let first_record = live_records.next().await.unwrap().unwrap();
            assert_eq!(first_record.number, 1);
            assert_eq!(first_record.value.id, "item-1");

            run.run()
                .checkpoint()
                .await
                .unwrap()
                .save_as(&checkpoint_path)
                .await
                .unwrap();

            let stop_report = run.stop().await.unwrap();
            assert_eq!(stop_report.report.run_id, run_id);

            let second_record = live_records.next().await.unwrap().unwrap();
            assert_eq!(second_record.number, 2);
            assert!(live_records.next().await.is_none());
        });
        writer.close().await.unwrap();

        restore_run_file(&checkpoint_path, &run_file_path(&env, run_id.as_str()));

        assert_interrupted_view(&view, &reader_items, &run_id).await;
        resume_and_finish(&env, &ground, &first_url, &run_id).await;
        assert_view_after_finish(&view, &reader_items, &run_id).await;

        view.close();
        reader.close().await.unwrap();
    });
}

fn check_blocking_view(env: &TestEnv, ground: &ProvingGround, run_id: &RunId) {
    let blocking_fox = blocking::Foxhound::connect(options(env, ground)).unwrap();
    let mut blocking_crawl = blocking::Crawl::new();
    let blocking_items = blocking_crawl
        .dataset(DatasetDeclaration::<serde_json::Value>::new("items").key(&["id"]))
        .unwrap();
    let blocking_view = blocking_fox.open_crawl_run(run_id).unwrap();
    assert_eq!(blocking_view.id(), run_id);

    let blocking_records: Vec<_> = blocking_view
        .records_after(&blocking_items, 1)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(blocking_records.len(), 1);
    assert_eq!(blocking_records[0].number, 2);

    let blocking_rejected: Vec<_> = blocking_view
        .rejected_records_after(&blocking_items, 1)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(blocking_rejected.len(), 1);
    assert_eq!(blocking_rejected[0].number, 2);

    let blocking_export = blocking_view
        .export(&blocking_items, ExportFormat::Json)
        .send()
        .unwrap();
    assert_eq!(blocking_export.records(), 2);

    let blocking_failed = blocking_view.failed_requests().send().unwrap();
    assert_eq!(blocking_failed.len(), 1);

    let blocking_report = blocking_view.report().send().unwrap();
    assert_eq!(blocking_report.report.run_id, *run_id);

    blocking_view.close();
    blocking_fox.close().unwrap();
}

/// Covers: story:11-1068, story:11-1109
#[test]
fn rust_crawl_run_and_view_after_rejected_export_and_failed_requests_async_and_blocking() {
    let env = TestEnv::new();
    let ground = ProvingGround::start(&env, SEED).unwrap();
    let ok_url = ground.start_url("catalog/1").unwrap();
    let fail_url = format!("{ok_url}?fail=1");
    let run_id = RunId::parse("open-crawl-run-methods").unwrap();

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &ground)).await.unwrap();
        let mut crawl = Crawl::new();
        let ok_label = crawl.label::<()>("ok").unwrap();
        let fail_label = crawl.label::<()>("fail").unwrap();
        let items = crawl
            .dataset(DatasetDeclaration::<serde_json::Value>::new("items").key(&["id"]))
            .unwrap();
        let items_h = items.clone();
        crawl
            .handle(&ok_label, move |ctx: CrawlContext<()>| {
                let items = items_h.clone();
                async move {
                    ctx.push(&items, &serde_json::json!({ "id": "a", "title": "A" }))
                        .await?;
                    let _ = ctx
                        .push(&items, &serde_json::json!({ "title": "missing-1" }))
                        .await;
                    ctx.push(&items, &serde_json::json!({ "id": "b", "title": "B" }))
                        .await?;
                    let _ = ctx
                        .push(&items, &serde_json::json!({ "title": "missing-2" }))
                        .await;
                    Ok(())
                }
            })
            .unwrap();
        crawl
            .handle(&fail_label, |_ctx: CrawlContext<()>| async {
                Err("intentional failure".into())
            })
            .unwrap();

        let run = foxhound
            .start_crawl(
                &crawl,
                [
                    Seed::new(&ok_url, &ok_label, ()),
                    Seed::new(&fail_url, &fail_label, ()),
                ],
            )
            .run_id(&run_id)
            .settings(site_settings(&ground).set(handler::CONCURRENCY, 1))
            .await
            .unwrap();
        let _report = crawl.serve(&run).await.unwrap();

        let records_after: Vec<_> = run.records_after(&items, 1).try_collect().await.unwrap();
        assert_eq!(records_after.len(), 1);
        assert_eq!(records_after[0].number, 2);

        let rejected_after: Vec<_> = run
            .rejected_records_after(&items, 1)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rejected_after.len(), 1);
        assert_eq!(rejected_after[0].number, 2);

        let run_export = run.export(&items, ExportFormat::Jsonl).await.unwrap();
        assert_eq!(run_export.records(), 2);

        let run_failed = run.failed_requests().await.unwrap();
        assert_eq!(run_failed.len(), 1);

        let view = foxhound.open_crawl_run(&run_id).unwrap();
        let view_rejected: Vec<_> = view
            .rejected_records_after(&items, 1)
            .try_collect()
            .await
            .unwrap();
        assert_eq!(view_rejected.len(), 1);
        assert_eq!(view_rejected[0].number, 2);

        let view_failed = view.failed_requests().await.unwrap();
        assert_eq!(view_failed.len(), 1);

        view.close();
        foxhound.close().await.unwrap();
    });

    check_blocking_view(&env, &ground, &run_id);
    let _ = error_code::HANDLER_FAILED;
}
