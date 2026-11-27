//! Seeds of a running crawl (`seeds()`, `add_seed`, `add_seeds`, and
//! `crawl.keep_alive`, spec #611's #1494; ADRs 0024, 0082, 0093 and 0102).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests collect handler observations on the real runtime, not under loom"
)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use goodfetch_foxhound::settings::{Settings, crawl as crawl_settings, fetch, politeness, timeout};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, Enqueued, Foxhound, LabelDeclaration, NotAddedReason,
    RunId, Schema, Seed, blocking, error_code,
};
use goodfetch_protocol::foxhound::frontier::PayloadInvalidDetails;
use goodfetch_protocol::run::{Lifecycle, StartedAs, StopReason};
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::crawl::site_settings;

const SEED: u64 = 1494;
const HOST: &str = "access.test";

#[derive(Debug, Serialize, Deserialize)]
struct Shelf {
    number: u32,
}

fn fast_options(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
    let settings = site_settings(site)
        .set(fetch::TIER, fetch::Tier::Http)
        .set(politeness::domain::CONCURRENCY, 16)
        .set(politeness::domain::DELAY, Duration::ZERO);
    ConnectOptions::default()
        .environment(env.env_os())
        .settings(settings)
}

/// Covers: story:6-208, story:6-209, story:6-210, story:11-1017, story:11-1018, operation:run.seeds, operation:run.add_seed, operation:run.add_seeds, setting:crawl.keep_alive
#[test]
fn seeds_answers_without_round_trip_including_resume_and_all_refused_behavior() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(fast_options(&env, &site)).await.unwrap();
        verify_seeds_and_resume(&foxhound, &site).await;
        verify_all_refused_and_keep_alive(&foxhound, &site).await;
        foxhound.close().await.unwrap();
    });
}

async fn verify_seeds_and_resume(foxhound: &Foxhound, site: &ProvingGround) {
    let root_url = site.url(HOST, "/corpus/article/1");
    let run_id = RunId::parse("live-seeds-resume-1494").unwrap();
    let entered = Arc::new(Notify::new());
    let entered_in = Arc::clone(&entered);

    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    crawl
        .handle(&page, move |_ctx: CrawlContext<()>| {
            let entered = Arc::clone(&entered_in);
            async move {
                entered.notify_one();
                tokio::time::sleep(Duration::from_millis(250)).await;
                Ok(())
            }
        })
        .unwrap();

    let run = foxhound
        .start_crawl(
            &crawl,
            [
                Seed::new(&root_url, &page, ()),
                Seed::new(&root_url, &page, ()),
                Seed::new("/relative-not-a-url", &page, ()),
            ],
        )
        .run_id(&run_id)
        .await
        .unwrap();
    assert_eq!(run.started_as(), StartedAs::New);
    let initial_answers = run.seeds().to_vec();
    assert_eq!(initial_answers.len(), 3);
    assert!(matches!(initial_answers[0], Enqueued::Added { .. }));
    assert!(matches!(
        initial_answers[1],
        Enqueued::NotAdded {
            reason: NotAddedReason::Duplicate,
            ..
        }
    ));
    assert!(matches!(
        initial_answers[2],
        Enqueued::NotAdded {
            reason: NotAddedReason::InvalidUrl,
            ..
        }
    ));

    let (served, stop_report) = tokio::join!(crawl.serve(&run), async {
        entered.notified().await;
        run.stop().await.unwrap()
    });
    served.unwrap();
    assert_eq!(stop_report.report.lifecycle, Lifecycle::Stopped);

    // Resume under the same Run ID: `seeds()` returns the same 3 original answers.
    let mut resumed_crawl = Crawl::new();
    let resumed_page = resumed_crawl.label::<()>("page").unwrap();
    resumed_crawl
        .handle(&resumed_page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();
    let resumed = foxhound
        .start_crawl(
            &resumed_crawl,
            [
                Seed::new(&root_url, &resumed_page, ()),
                Seed::new(&root_url, &resumed_page, ()),
                Seed::new("/relative-not-a-url", &resumed_page, ()),
            ],
        )
        .run_id(&run_id)
        .await
        .unwrap();
    assert_eq!(resumed.started_as(), StartedAs::Resumed);
    assert_eq!(resumed.seeds(), initial_answers.as_slice());
    let (served_resumed, finished) = tokio::join!(resumed_crawl.serve(&resumed), resumed.finish());
    served_resumed.unwrap();
    assert_eq!(finished.unwrap().report.lifecycle, Lifecycle::Finished);
}

async fn verify_all_refused_and_keep_alive(foxhound: &Foxhound, site: &ProvingGround) {
    let second_url = site.url(HOST, "/corpus/article/2");

    // 1. All seeds refused -> finishes at once unless `crawl.keep_alive`.
    let mut empty_crawl = Crawl::new();
    let empty_page = empty_crawl.label::<()>("page").unwrap();
    empty_crawl
        .handle(&empty_page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();
    let refused_run = foxhound
        .start_crawl(
            &empty_crawl,
            [
                Seed::new("/relative-1", &empty_page, ()),
                Seed::new("/relative-2", &empty_page, ()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(refused_run.seeds().len(), 2);
    assert!(refused_run.seeds().iter().all(|ans| matches!(
        ans,
        Enqueued::NotAdded {
            reason: NotAddedReason::InvalidUrl,
            ..
        }
    )));
    let (served_empty, report_empty) =
        tokio::join!(empty_crawl.serve(&refused_run), refused_run.finish());
    served_empty.unwrap();
    assert_eq!(report_empty.unwrap().report.lifecycle, Lifecycle::Finished);

    // 2. All seeds refused under `crawl.keep_alive`: stays running and `idle: true`,
    // accepts `add_seed`, and finishes only after `finish()`.
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen_in = Arc::clone(&seen);
    let mut keep_crawl = Crawl::new();
    let keep_page = keep_crawl.label::<()>("page").unwrap();
    keep_crawl
        .handle(&keep_page, move |ctx: CrawlContext<()>| {
            let seen = Arc::clone(&seen_in);
            async move {
                seen.lock().unwrap().push(ctx.url().to_owned());
                Ok(())
            }
        })
        .unwrap();
    let keep_run = foxhound
        .start_crawl(&keep_crawl, [Seed::new("/relative-only", &keep_page, ())])
        .settings(Settings::new().set(crawl_settings::KEEP_ALIVE, true))
        .await
        .unwrap();
    assert_eq!(keep_run.seeds().len(), 1);
    assert!(matches!(
        keep_run.seeds()[0],
        Enqueued::NotAdded {
            reason: NotAddedReason::InvalidUrl,
            ..
        }
    ));

    let (served_keep, keep_report) = tokio::join!(keep_crawl.serve(&keep_run), async {
        let mut progress = keep_run.progress();
        let first = progress.next().await.unwrap().unwrap();
        assert_eq!(first.progress.lifecycle, Lifecycle::Running);
        assert_eq!(
            first.idle,
            Some(true),
            "keep-alive crawl with all seeds refused must be idle"
        );
        drop(progress);

        let added = keep_run
            .add_seed(Seed::new(&second_url, &keep_page, ()))
            .await
            .unwrap();
        assert!(matches!(added, Enqueued::Added { .. }));

        keep_run.finish().await.unwrap()
    });
    served_keep.unwrap();
    assert_eq!(keep_report.report.lifecycle, Lifecycle::Finished);
    assert_eq!(*seen.lock().unwrap(), vec![second_url]);
}

/// Covers: story:6-210, operation:run.add_seed, setting:crawl.keep_alive
#[test]
fn keep_alive_idle_time_counts_toward_timeout_run() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let root_url = site.url(HOST, "/corpus/article/1");

    real_time_runtime().block_on(async {
        let settings = site_settings(&site)
            .set(fetch::TIER, fetch::Tier::Http)
            .set(politeness::domain::CONCURRENCY, 16)
            .set(politeness::domain::DELAY, Duration::ZERO)
            .set(timeout::RUN, Duration::from_millis(600));
        let foxhound = Foxhound::connect(
            ConnectOptions::default()
                .environment(env.env_os())
                .settings(settings),
        )
        .await
        .unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&root_url, &page, ())])
            .settings(Settings::new().set(crawl_settings::KEEP_ALIVE, true))
            .await
            .unwrap();

        // Wait on progress without calling `finish()`: the crawl becomes idle and
        // ends with `stop_reason == Timeout` once `timeout.run` elapses.
        let (served, last_progress) = tokio::join!(crawl.serve(&run), async {
            let mut progress = run.progress();
            let mut saw_idle = false;
            let mut last = None;
            while let Some(step) = progress.next().await {
                let step = step.unwrap();
                if step.idle == Some(true) {
                    saw_idle = true;
                }
                if step.progress.last {
                    last = Some(step);
                    break;
                }
            }
            assert!(
                saw_idle,
                "keep-alive crawl should become idle before timing out"
            );
            last.expect("progress stream yields a final value on timeout")
        });
        served.unwrap();
        assert_eq!(last_progress.progress.lifecycle, Lifecycle::Stopped);
        assert_eq!(
            last_progress.progress.stop_reason,
            Some(StopReason::Timeout)
        );

        foxhound.close().await.unwrap();
    });
}

/// Covers: operation:run.add_seed, operation:run.add_seeds
#[test]
fn attempt_add_seed_and_add_seeds_stage_with_attempt() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let root_url = site.url(HOST, "/corpus/article/1");
    let discarded_single = site.url(HOST, "/corpus/article/10");
    let discarded_stream = site.url(HOST, "/corpus/article/11");
    let committed_single = site.url(HOST, "/corpus/article/12");
    let committed_stream = site.url(HOST, "/corpus/article/13");

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(fast_options(&env, &site)).await.unwrap();
        let attempts = Arc::new(AtomicU32::new(0));
        let handled: Arc<Mutex<Vec<String>>> = Arc::default();

        let mut crawl = Crawl::new();
        let root = crawl.label::<()>("root").unwrap();
        let child = crawl.label::<()>("child").unwrap();

        let child_for_root = child.clone();
        let attempts_in = Arc::clone(&attempts);
        let d_single = discarded_single.clone();
        let d_stream = discarded_stream.clone();
        let c_single = committed_single.clone();
        let c_stream = committed_stream.clone();
        crawl
            .handle(&root, move |ctx: CrawlContext<()>| {
                let child = child_for_root.clone();
                let attempts = Arc::clone(&attempts_in);
                let d_single = d_single.clone();
                let d_stream = d_stream.clone();
                let c_single = c_single.clone();
                let c_stream = c_stream.clone();
                async move {
                    let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                    if n == 1 {
                        let ans = ctx.run().add_seed(Seed::new(&d_single, &child, ())).await?;
                        assert!(matches!(ans, Enqueued::Added { .. }));
                        let streamed: Vec<_> = ctx
                            .run()
                            .add_seeds(futures_util::stream::iter([Seed::new(
                                &d_stream,
                                &child,
                                (),
                            )]))
                            .collect()
                            .await;
                        assert_eq!(streamed.len(), 1);
                        assert!(matches!(streamed[0], Ok(Enqueued::Added { .. })));
                        return Err("intentional attempt 1 failure".into());
                    }
                    let ans = ctx.run().add_seed(Seed::new(&c_single, &child, ())).await?;
                    assert!(matches!(ans, Enqueued::Added { .. }));
                    let streamed: Vec<_> = ctx
                        .run()
                        .add_seeds(futures_util::stream::iter([Seed::new(
                            &c_stream,
                            &child,
                            (),
                        )]))
                        .collect()
                        .await;
                    assert_eq!(streamed.len(), 1);
                    assert!(matches!(streamed[0], Ok(Enqueued::Added { .. })));
                    Ok(())
                }
            })
            .unwrap();

        let handled_in = Arc::clone(&handled);
        crawl
            .handle(&child, move |ctx: CrawlContext<()>| {
                let handled = Arc::clone(&handled_in);
                async move {
                    handled.lock().unwrap().push(ctx.url().to_owned());
                    Ok(())
                }
            })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&root_url, &root, ())])
            .settings(
                Settings::new()
                    .set(crawl_settings::retry::MAX, 2)
                    .set(crawl_settings::retry::HANDLER_FAILED, true),
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);

        let mut seen = handled.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, vec![committed_single, committed_stream]);

        foxhound.close().await.unwrap();
    });
}

/// Covers: story:6-208, story:6-209, story:11-1017, story:11-1018, operation:run.add_seed, operation:run.add_seeds
#[test]
fn add_seeds_streams_100_in_order_validates_payload_and_blocking_works() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();

    real_time_runtime().block_on(verify_async_stream_and_payloads(&env, &site));
    verify_blocking_live_seeds(&env, &site);
}

async fn verify_async_stream_and_payloads(env: &TestEnv, site: &ProvingGround) {
    let url = |path: &str| site.url(HOST, path);
    let root_url = url("/corpus/article/1");
    let foxhound = Foxhound::connect(fast_options(env, site)).await.unwrap();
    let handled_numbers: Arc<Mutex<Vec<u32>>> = Arc::default();
    let handled_in = Arc::clone(&handled_numbers);

    let mut crawl = Crawl::new();
    let shelf = crawl.label::<Shelf>("shelf").unwrap();
    let raw_schema = Schema::json(json!({
        "type": "object",
        "properties": { "code": { "type": "string" } },
        "required": ["code"],
        "additionalProperties": false,
    }))
    .unwrap();
    let raw = crawl
        .label(LabelDeclaration::<Value>::new("raw").payload(&raw_schema))
        .unwrap();
    crawl
        .handle(&shelf, move |ctx: CrawlContext<Shelf>| {
            let handled = Arc::clone(&handled_in);
            async move {
                handled.lock().unwrap().push(ctx.payload().number);
                Ok(())
            }
        })
        .unwrap();
    crawl
        .handle(&raw, |_ctx: CrawlContext<Value>| async { Ok(()) })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&root_url, &shelf, Shelf { number: 0 })])
        .settings(
            Settings::new()
                .set(crawl_settings::KEEP_ALIVE, true)
                .set(crawl_settings::payload::MAX_SIZE, 64),
        )
        .await
        .unwrap();

    let (served, report) = tokio::join!(crawl.serve(&run), async {
        assert_payload_invalid_errors(&run, site, &raw).await;

        let seeds_100: Vec<Seed> = (1..=100_u32)
            .map(|i| {
                Seed::new(
                    url(&format!("/corpus/article/1?seed={i}")),
                    &shelf,
                    Shelf { number: i },
                )
            })
            .collect();
        let mut stream = run.add_seeds(futures_util::stream::iter(seeds_100));
        let mut answers = Vec::with_capacity(100);
        while let Some(item) = stream.next().await {
            answers.push(item.unwrap());
        }
        assert_eq!(answers.len(), 100);
        for (idx, answer) in answers.iter().enumerate() {
            let expected_query = format!("?seed={}", idx + 1);
            match answer {
                Enqueued::Added { url: added_url, .. } => {
                    assert!(
                        added_url.ends_with(&expected_query),
                        "answer {idx} out of order: got {added_url}"
                    );
                }
                other @ Enqueued::NotAdded { .. } => {
                    panic!("seed {idx} was not added: {other:?}")
                }
            }
        }

        run.finish().await.unwrap()
    });
    served.unwrap();
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    assert_eq!(handled_numbers.lock().unwrap().len(), 101);
    foxhound.close().await.unwrap();
}

async fn assert_payload_invalid_errors(
    run: &goodfetch_foxhound::CrawlRun,
    site: &ProvingGround,
    raw: &goodfetch_foxhound::Label<Value>,
) {
    let schema_err = run
        .add_seed(Seed::new(
            site.url(HOST, "/corpus/article/bad-schema"),
            raw,
            json!({ "code": 42 }),
        ))
        .await
        .expect_err("schema-violating payload must fail with payload_invalid");
    assert_eq!(schema_err.code(), error_code::PAYLOAD_INVALID);
    let details: PayloadInvalidDetails = schema_err
        .details()
        .expect("payload_invalid details decode as PayloadInvalidDetails");
    assert!(
        !details.issues.is_empty(),
        "payload_invalid must carry schema issues"
    );

    let oversized = "x".repeat(120);
    let size_err = run
        .add_seed(Seed::new(
            site.url(HOST, "/corpus/article/too-big"),
            raw,
            json!({ "code": oversized }),
        ))
        .await
        .expect_err("oversized payload must fail with payload_invalid");
    assert_eq!(size_err.code(), error_code::PAYLOAD_INVALID);
}

fn verify_blocking_live_seeds(env: &TestEnv, site: &ProvingGround) {
    let url = |path: &str| site.url(HOST, path);
    let root_url = url("/corpus/article/1");
    let foxhound = blocking::Foxhound::connect(fast_options(env, site)).unwrap();
    let seen_blocking: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen_in = Arc::clone(&seen_blocking);
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    crawl
        .handle(&page, move |ctx: blocking::CrawlContext<()>| {
            seen_in.lock().unwrap().push(ctx.url().to_owned());
            Ok(())
        })
        .unwrap();
    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&root_url, &page, ())])
        .settings(Settings::new().set(crawl_settings::KEEP_ALIVE, true))
        .send()
        .unwrap();
    assert_eq!(run.seeds().len(), 1);
    assert!(matches!(run.seeds()[0], Enqueued::Added { .. }));

    let b1 = url("/corpus/article/1?blocking=1");
    let b2 = url("/corpus/article/1?blocking=2");
    let b3 = url("/corpus/article/1?blocking=3");
    #[expect(
        clippy::disallowed_methods,
        reason = "blocking SDK twin drives add_seed/add_seeds/finish concurrently while crawl.serve blocks this thread"
    )]
    let blocking_report = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let one = run.add_seed(Seed::new(&b1, &page, ())).send().unwrap();
            assert!(matches!(one, Enqueued::Added { .. }));
            let streamed: Vec<_> = run
                .add_seeds([Seed::new(&b2, &page, ()), Seed::new(&b3, &page, ())])
                .map(Result::unwrap)
                .collect();
            assert_eq!(streamed.len(), 2);
            assert!(
                streamed
                    .iter()
                    .all(|ans| matches!(ans, Enqueued::Added { .. }))
            );
            run.finish().send().unwrap()
        });
        crawl.serve(&run).unwrap();
        driver.join().unwrap()
    });
    assert_eq!(blocking_report.report.lifecycle, Lifecycle::Finished);
    assert_eq!(seen_blocking.lock().unwrap().len(), 4);
    foxhound.close().unwrap();
}
