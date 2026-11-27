//! A crawl's outcome log, current failed requests and `retry_failed` in Rust
//! (ADRs 0102 and 0117; issue 1495): `outcomes()` yields every ending in
//! commit order with engine-side `kinds`, `labels` and `codes` filters and
//! `after_now()`, a stalled reader never slows the crawl, `failed_requests()`
//! matches what `retry_failed()` re-admits (with and without `blocked`), and
//! `retry_failed()` returns an ended Run to `running` whose new outcome
//! supersedes the earlier one by `request.id`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests track per-request handler attempts outside loom"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::TryStreamExt as _;
use goodfetch_foxhound::settings::Settings;
use goodfetch_foxhound::settings::crawl::{handler, retry};
use goodfetch_foxhound::{
    Crawl, CrawlContext, CrawlRequestOutcome, CrawlRun, Error, Foxhound, Label, OutcomeKind, RunId,
    Seed, blocking, error_code,
};
use goodfetch_protocol::foxhound::outcomes::Ending;
use goodfetch_protocol::page::BLOCKED;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::crawl::{options, site_settings};

/// The Proving ground's seed for these tests.
const SEED: u64 = 1495;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Note {
    url: String,
}

/// Builds a `blocked` Error with cause `captcha` so a handler produces a
/// genuine `blocked` outcome without external sites.
fn captcha_blocked() -> Error {
    Error::of_sdk(
        &BLOCKED,
        "blocked by captcha",
        "solve the captcha",
        &json!({ "cause": "captcha" }),
    )
}

/// Asserts commit order, all four outcome kinds, and engine-side `kinds`,
/// `labels`, and `codes` filters; returns `(last_position, fail_request_id)`.
async fn assert_initial_outcomes(
    run: &CrawlRun,
    ok_url: &str,
    skip_url: &str,
    fail_url: &str,
    block_url: &str,
    extra_label: &Label<()>,
) -> (u64, String) {
    let all: Vec<CrawlRequestOutcome> = run.outcomes().try_collect().await.unwrap();
    assert_eq!(all.len(), 4);
    assert!(all.windows(2).all(|w| w[0].position < w[1].position));

    let kinds_by_url: BTreeMap<&str, OutcomeKind> = all
        .iter()
        .map(|o| (o.request.url.as_str(), o.ending.kind()))
        .collect();
    assert_eq!(kinds_by_url[ok_url], OutcomeKind::Succeeded);
    assert_eq!(kinds_by_url[skip_url], OutcomeKind::Skipped);
    assert_eq!(kinds_by_url[fail_url], OutcomeKind::Failed);
    assert_eq!(kinds_by_url[block_url], OutcomeKind::Blocked);

    let only_bad: Vec<CrawlRequestOutcome> = run
        .outcomes()
        .kinds([OutcomeKind::Failed, OutcomeKind::Blocked])
        .try_collect()
        .await
        .unwrap();
    assert_eq!(only_bad.len(), 2);

    let only_extra: Vec<CrawlRequestOutcome> = run
        .outcomes()
        .labels([extra_label.name()])
        .try_collect()
        .await
        .unwrap();
    assert_eq!(only_extra.len(), 2);
    assert!(only_extra.iter().all(|o| o.request.label == "extra"));

    let only_handler_failed: Vec<CrawlRequestOutcome> = run
        .outcomes()
        .codes([error_code::HANDLER_FAILED])
        .try_collect()
        .await
        .unwrap();
    assert_eq!(only_handler_failed.len(), 1);
    assert_eq!(only_handler_failed[0].request.url, fail_url);
    assert_eq!(only_handler_failed[0].attempts, 1);
    assert_eq!(only_handler_failed[0].errors.len(), 1);
    assert_eq!(
        only_handler_failed[0].errors[0].code,
        error_code::HANDLER_FAILED.as_str()
    );

    let failed_only = run.failed_requests().await.unwrap();
    assert_eq!(failed_only.len(), 1);
    assert_eq!(failed_only[0].request.url, fail_url);
    let failed_and_blocked = run.failed_requests().blocked(true).await.unwrap();
    assert_eq!(failed_and_blocked.len(), 2);

    (
        all.last().unwrap().position,
        failed_only[0].request.id.clone(),
    )
}

/// Asserts `retry_failed` (without and with `blocked`), `after_now()` live
/// tailing, and superseding outcomes by `request.id`.
async fn assert_retries_and_superseded(
    crawl: &Crawl<'_>,
    run: &CrawlRun,
    last_before: u64,
    fail_request_id: &str,
    block_url: &str,
    retry_entered: &tokio::sync::Notify,
    allow_retry_commit: &tokio::sync::Notify,
) {
    assert_eq!(run.retry_failed().await.unwrap(), 1);
    let after_now = run.outcomes().after_now();
    let (served, later, ()) =
        tokio::join!(crawl.serve(run), after_now.try_collect::<Vec<_>>(), async {
            retry_entered.notified().await;
            // Flush `run.outcomes(after: now)` onto the wire before committing the retry.
            let _ = run.failed_requests().await.unwrap();
            allow_retry_commit.notify_one();
        });
    assert_eq!(served.unwrap().report.lifecycle, Lifecycle::Finished);
    let later = later.unwrap();
    assert_eq!(
        later.len(),
        1,
        "after_now() must yield the 1 later outcome: {later:?}"
    );
    assert!(later[0].position > last_before);
    assert_eq!(later[0].request.id, fail_request_id);
    assert_eq!(later[0].request.depth, 0);
    assert_eq!(later[0].ending.kind(), OutcomeKind::Succeeded);
    assert_eq!(later[0].attempts, 2);
    match &later[0].ending {
        Ending::Succeeded {
            records,
            nothing_committed,
            ..
        } => {
            assert_eq!(*nothing_committed, None);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].dataset, "notes");
            assert_eq!(records[0].committed, 1);
        }
        other => panic!("expected succeeded ending with records, got {other:?}"),
    }

    assert_eq!(run.failed_requests().await.unwrap(), []);
    assert_eq!(run.retry_failed().await.unwrap(), 0);

    let blocked_left = run.failed_requests().blocked(true).await.unwrap();
    assert_eq!(blocked_left.len(), 1);
    assert_eq!(blocked_left[0].request.url, block_url);
    assert_eq!(run.retry_failed().blocked(true).await.unwrap(), 1);
    let final_report = crawl.serve(run).await.unwrap();
    assert_eq!(final_report.report.lifecycle, Lifecycle::Finished);
    assert_eq!(run.failed_requests().blocked(true).await.unwrap(), []);
    assert_eq!(run.retry_failed().blocked(true).await.unwrap(), 0);

    let full_log: Vec<CrawlRequestOutcome> = run.outcomes().try_collect().await.unwrap();
    let fail_history: Vec<OutcomeKind> = full_log
        .iter()
        .filter(|o| o.request.id == fail_request_id)
        .map(|o| o.ending.kind())
        .collect();
    assert_eq!(
        fail_history,
        vec![OutcomeKind::Failed, OutcomeKind::Succeeded]
    );
}

fn build_outcomes_crawl(
    skip_url: String,
    fail_url: String,
    retry_entered: Arc<tokio::sync::Notify>,
    allow_retry_commit: Arc<tokio::sync::Notify>,
) -> (Crawl<'static>, Label<()>, Label<()>) {
    let fail_hits = Arc::new(AtomicU32::new(0));
    let block_hits = Arc::new(AtomicU32::new(0));
    let mut crawl = Crawl::new();
    let notes = crawl.dataset::<Note>("notes").unwrap();
    let main_label = crawl.label::<()>("main").unwrap();
    let extra_label = crawl.label::<()>("extra").unwrap();

    let notes_main = notes.clone();
    crawl
        .handle(&main_label, move |ctx: CrawlContext<()>| {
            let notes = notes_main.clone();
            let fail_url = fail_url.clone();
            let fail_hits = Arc::clone(&fail_hits);
            let retry_entered = Arc::clone(&retry_entered);
            let allow_retry_commit = Arc::clone(&allow_retry_commit);
            async move {
                if ctx.url() == fail_url {
                    if fail_hits.fetch_add(1, Ordering::SeqCst) == 0 {
                        return Err("transient boom".into());
                    }
                    retry_entered.notify_one();
                    allow_retry_commit.notified().await;
                }
                let note = Note {
                    url: ctx.url().to_owned(),
                };
                ctx.push(&notes, &note).await?;
                Ok(())
            }
        })
        .unwrap();

    crawl
        .handle(&extra_label, move |ctx: CrawlContext<()>| {
            let notes = notes.clone();
            let skip_url = skip_url.clone();
            let block_hits = Arc::clone(&block_hits);
            async move {
                if ctx.url() == skip_url {
                    return Err(ctx.skip("not wanted"));
                }
                if block_hits.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(captcha_blocked().into());
                }
                let note = Note {
                    url: ctx.url().to_owned(),
                };
                ctx.push(&notes, &note).await?;
                Ok(())
            }
        })
        .unwrap();

    (crawl, main_label, extra_label)
}

/// Covers: story:6-238, story:6-306, story:12-219, operation:run.outcomes, operation:run.failed_requests, operation:run.retry_failed
#[test]
fn outcomes_filter_stalled_reader_and_retry_failed_supersedes_by_request_id() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let ok_url = site.url("shop.test", "/book?case=ok");
    let skip_url = site.url("shop.test", "/book?case=skip");
    let fail_url = site.url("shop.test", "/book?case=fail");
    let block_url = site.url("shop.test", "/book?case=block");
    let run_id = RunId::parse("rust-outcomes-retry").unwrap();

    let retry_entered = Arc::new(tokio::sync::Notify::new());
    let allow_retry_commit = Arc::new(tokio::sync::Notify::new());
    let (crawl, main_label, extra_label) = build_outcomes_crawl(
        skip_url.clone(),
        fail_url.clone(),
        Arc::clone(&retry_entered),
        Arc::clone(&allow_retry_commit),
    );

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run_settings = Settings::new()
            .set(handler::CONCURRENCY, 1)
            .set(retry::MAX, 0);
        let run = foxhound
            .start_crawl(
                &crawl,
                [
                    Seed::new(&ok_url, &main_label, ()),
                    Seed::new(&skip_url, &extra_label, ()),
                    Seed::new(&fail_url, &main_label, ()),
                    Seed::new(&block_url, &extra_label, ()),
                ],
            )
            .run_id(&run_id)
            .settings(run_settings)
            .await
            .unwrap();

        // Open a reader that never polls while the crawl runs: must not slow or stall it.
        let _stalled = run.outcomes();
        let report = crawl.serve(&run).await.unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);

        let (last_before, fail_request_id) = assert_initial_outcomes(
            &run,
            &ok_url,
            &skip_url,
            &fail_url,
            &block_url,
            &extra_label,
        )
        .await;

        assert_retries_and_superseded(
            &crawl,
            &run,
            last_before,
            &fail_request_id,
            &block_url,
            &retry_entered,
            &allow_retry_commit,
        )
        .await;

        foxhound.close().await.unwrap();
    });
}

/// Covers: story:6-306, operation:run.outcomes, operation:run.failed_requests, operation:run.retry_failed
#[test]
fn blocking_outcomes_failed_requests_and_retry_failed_match() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let fail_url = site.url("shop.test", "/book?blocking=1");
    let hits = Arc::new(AtomicU32::new(0));
    let hits_h = Arc::clone(&hits);

    let foxhound = blocking::Foxhound::connect(options(&env, &site)).unwrap();
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    crawl
        .handle(&page, move |_ctx| {
            if hits_h.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err("first try failed".into());
            }
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&fail_url, &page, ())])
        .settings(site_settings(&site).set(retry::MAX, 0))
        .send()
        .unwrap();
    let first = crawl.serve(&run).unwrap();
    assert_eq!(first.report.lifecycle, Lifecycle::Finished);

    let failed = run.failed_requests().send().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(run.retry_failed().send().unwrap(), 1);
    let second = crawl.serve(&run).unwrap();
    assert_eq!(second.report.lifecycle, Lifecycle::Finished);
    assert_eq!(run.retry_failed().send().unwrap(), 0);

    let outcomes: Vec<OutcomeKind> = run
        .outcomes()
        .map(|item| item.unwrap().ending.kind())
        .collect();
    assert_eq!(outcomes, vec![OutcomeKind::Failed, OutcomeKind::Succeeded]);
    foxhound.close().unwrap();
}
