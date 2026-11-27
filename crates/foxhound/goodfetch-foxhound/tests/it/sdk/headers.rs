//! Request headers carried from seeds and `enqueue` across HTTP, browser-fetch
//! and browser tiers, across checkpoint and resume, excluded from Frontier
//! deduplication, and forbidden-header enforcement (spec #611's #3660; ADRs
//! 0033, 0093 and 0104).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests collect handler observations on the real runtime, not under loom"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_foxhound::settings::{Settings, fetch, politeness, session};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, Enqueued, ErrorCode, Foxhound, NotAddedReason, RunId,
    Seed, blocking,
};
use goodfetch_protocol::run::{Lifecycle, StartedAs};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache, real_time_runtime,
};
use rstest::rstest;
use tokio::sync::Notify;

use super::crawl::site_settings;

const SEED: u64 = 3660;
const HOST: &str = "access.test";

type SeenHeaders = Arc<Mutex<Vec<(String, BTreeMap<String, String>)>>>;

fn tier_options(env: &TestEnv, site: &ProvingGround, tier: fetch::Tier) -> ConnectOptions {
    let mut env_os = env.env_os();
    if matches!(tier, fetch::Tier::BrowserFetch | fetch::Tier::Browser) {
        let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
        env_os.insert(
            OsString::from(BROWSERS_PATH_VARIABLE),
            cache.into_os_string(),
        );
    }
    let settings = site_settings(site)
        .set(fetch::TIER, tier)
        .set(session::pool::MAX_SIZE, 1)
        .set(politeness::domain::CONCURRENCY, 8)
        .set(politeness::domain::DELAY, Duration::ZERO);
    ConnectOptions::default()
        .environment(env_os)
        .settings(settings)
}

fn observed_header(site: &ProvingGround, target: &str, name: &str) -> Option<String> {
    let expected = name.to_ascii_lowercase();
    site.received_requests()
        .unwrap()
        .of_the_case()
        .find(|req| req.host == HOST && req.target == target)
        .and_then(|req| {
            req.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&expected))
                .map(|(_, v)| v.clone())
        })
}

/// Covers: story:11-1019, operation:crawl_request.enqueue
#[rstest]
#[case::http(fetch::Tier::Http)]
#[case::browser_fetch(fetch::Tier::BrowserFetch)]
#[case::browser(fetch::Tier::Browser)]
fn seed_and_enqueue_headers_reach_proving_ground_on_every_tier_and_deduplicate(
    #[case] tier: fetch::Tier,
) {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let root_url = site.url(HOST, "/corpus/article/1");
    let child_url = site.url(HOST, "/corpus/article/2");

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(tier_options(&env, &site, tier))
            .await
            .unwrap();
        let seen_headers: SeenHeaders = Arc::default();
        let duplicate_answer: Arc<Mutex<Option<Enqueued>>> = Arc::default();

        let mut crawl = Crawl::new();
        let root = crawl.label::<()>("root").unwrap();
        let child = crawl.label::<()>("child").unwrap();

        let seen_root = Arc::clone(&seen_headers);
        let dup_slot = Arc::clone(&duplicate_answer);
        let child_label = child.clone();
        let child_for_root = child_url.clone();
        crawl
            .handle(&root, move |ctx: CrawlContext<()>| {
                let seen_root = Arc::clone(&seen_root);
                let dup_slot = Arc::clone(&dup_slot);
                let child_label = child_label.clone();
                let child_for_root = child_for_root.clone();
                async move {
                    seen_root
                        .lock()
                        .unwrap()
                        .push((ctx.label().to_owned(), ctx.headers().clone()));
                    let first = ctx
                        .enqueue(&child_for_root, &child_label, ())
                        .header("X-Enqueue-Tag", "from-enqueue")
                        .await?;
                    assert!(matches!(first, Enqueued::Added { .. }), "{first:?}");
                    let second = ctx
                        .enqueue(&child_for_root, &child_label, ())
                        .header("X-Enqueue-Tag", "different-value")
                        .await?;
                    *dup_slot.lock().unwrap() = Some(second);
                    Ok(())
                }
            })
            .unwrap();

        let seen_child = Arc::clone(&seen_headers);
        crawl
            .handle(&child, move |ctx: CrawlContext<()>| {
                let seen_child = Arc::clone(&seen_child);
                async move {
                    seen_child
                        .lock()
                        .unwrap()
                        .push((ctx.label().to_owned(), ctx.headers().clone()));
                    Ok(())
                }
            })
            .unwrap();

        let run = foxhound
            .start_crawl(
                &crawl,
                [Seed::new(root_url, &root, ()).header("X-Seed-Tag", "from-seed")],
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        foxhound.close().await.unwrap();

        assert_eq!(
            observed_header(&site, "/corpus/article/1", "x-seed-tag").as_deref(),
            Some("from-seed"),
            "seed header must reach Proving ground on tier {tier:?}"
        );
        assert_eq!(
            observed_header(&site, "/corpus/article/2", "x-enqueue-tag").as_deref(),
            Some("from-enqueue"),
            "enqueued header must reach Proving ground on tier {tier:?}"
        );
        assert!(matches!(
            duplicate_answer.lock().unwrap().as_ref(),
            Some(Enqueued::NotAdded {
                reason: NotAddedReason::Duplicate,
                ..
            })
        ));
        let seen = seen_headers.lock().unwrap().clone();
        assert_eq!(
            seen.iter()
                .find(|(label, _)| label == "root")
                .and_then(|(_, h)| h.get("X-Seed-Tag"))
                .map(String::as_str),
            Some("from-seed")
        );
        assert_eq!(
            seen.iter()
                .find(|(label, _)| label == "child")
                .and_then(|(_, h)| h.get("X-Enqueue-Tag"))
                .map(String::as_str),
            Some("from-enqueue")
        );
    });
}

/// Covers: story:11-1019, operation:crawl_request.enqueue
#[test]
fn request_headers_survive_checkpoint_and_resume() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let root_url = site.url(HOST, "/corpus/article/1");
    let child_url = site.url(HOST, "/corpus/article/2");
    let run_id = RunId::parse("headers-resume").unwrap();

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(tier_options(&env, &site, fetch::Tier::Http))
            .await
            .unwrap();
        let enqueued_child = Arc::new(Notify::new());
        let child_seen: Arc<Mutex<Option<BTreeMap<String, String>>>> = Arc::default();

        let mut crawl = Crawl::new();
        let root = crawl.label::<()>("root").unwrap();
        let child = crawl.label::<()>("child").unwrap();

        let child_label = child.clone();
        let child_for_root = child_url.clone();
        let enqueued_notify = Arc::clone(&enqueued_child);
        crawl
            .handle(&root, move |ctx: CrawlContext<()>| {
                let child_label = child_label.clone();
                let child_for_root = child_for_root.clone();
                let enqueued_notify = Arc::clone(&enqueued_notify);
                async move {
                    let ans = ctx
                        .enqueue(&child_for_root, &child_label, ())
                        .header("X-Resume-Header", "kept-across-stop")
                        .await?;
                    assert!(matches!(ans, Enqueued::Added { .. }), "{ans:?}");
                    enqueued_notify.notify_one();
                    // Wait briefly so run.stop() reaches the engine before root commits,
                    // ensuring the engine checkpoints with child pending.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok(())
                }
            })
            .unwrap();

        let seen_slot = Arc::clone(&child_seen);
        crawl
            .handle(&child, move |ctx: CrawlContext<()>| {
                let seen_slot = Arc::clone(&seen_slot);
                async move {
                    *seen_slot.lock().unwrap() = Some(ctx.headers().clone());
                    Ok(())
                }
            })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&root_url, &root, ())])
            .run_id(&run_id)
            .settings(Settings::new().set(politeness::domain::CONCURRENCY, 1))
            .await
            .unwrap();
        let stop_handle = run.clone();
        let (stopped, _) = tokio::join!(crawl.serve(&run), async move {
            enqueued_child.notified().await;
            stop_handle.stop().await.unwrap()
        });
        let stopped = stopped.unwrap();
        assert_eq!(stopped.report.lifecycle, Lifecycle::Stopped);
        assert!(
            child_seen.lock().unwrap().is_none(),
            "child must remain pending across stop()"
        );

        // Resume with the same Run ID and seed; the pending child executes and delivers its header.
        let resumed = foxhound
            .start_crawl(&crawl, [Seed::new(&root_url, &root, ())])
            .run_id(&run_id)
            .await
            .unwrap();
        assert_eq!(resumed.started_as(), StartedAs::Resumed);
        let finished = crawl.serve(&resumed).await.unwrap();
        assert_eq!(finished.report.lifecycle, Lifecycle::Finished);
        foxhound.close().await.unwrap();

        assert_eq!(
            observed_header(&site, "/corpus/article/2", "x-resume-header").as_deref(),
            Some("kept-across-stop")
        );
        assert_eq!(
            child_seen
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|h| h.get("X-Resume-Header"))
                .map(String::as_str),
            Some("kept-across-stop")
        );
    });
}

/// Covers: story:11-1019, operation:crawl_request.enqueue, error:header_forbidden
#[test]
fn forbidden_headers_fail_with_header_forbidden_and_add_nothing() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let root_url = site.url(HOST, "/corpus/article/1");
    let blocked_url = site.url(HOST, "/corpus/article/3");
    let forbidden = [
        ("Host", "other.test"),
        ("Cookie", "session=disallowed"),
        ("Authorization", "disallowed"),
        ("Proxy-Authorization", "disallowed"),
        ("X-HTTP-Method-Override", "DELETE"),
    ];

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(tier_options(&env, &site, fetch::Tier::Http))
            .await
            .unwrap();

        let mut crawl = Crawl::new();
        let root = crawl.label::<()>("root").unwrap();
        let child = crawl.label::<()>("child").unwrap();
        let enqueue_errors: Arc<Mutex<Vec<(&'static str, ErrorCode)>>> = Arc::default();

        let errors_h = Arc::clone(&enqueue_errors);
        let child_label = child.clone();
        let blocked_for_handler = blocked_url.clone();
        crawl
            .handle(&root, move |ctx: CrawlContext<()>| {
                let errors_h = Arc::clone(&errors_h);
                let child_label = child_label.clone();
                let blocked_for_handler = blocked_for_handler.clone();
                async move {
                    for (name, value) in forbidden {
                        let err = ctx
                            .enqueue(&blocked_for_handler, &child_label, ())
                            .header(name, value)
                            .await
                            .unwrap_err();
                        errors_h.lock().unwrap().push((name, err.code()));
                    }
                    Ok(())
                }
            })
            .unwrap();
        crawl
            .handle(&child, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();

        // Every forbidden header on a seed fails start_crawl with header_forbidden.
        for (name, value) in forbidden {
            let err = foxhound
                .start_crawl(
                    &crawl,
                    [Seed::new(&blocked_url, &child, ()).header(name, value)],
                )
                .await
                .unwrap_err();
            assert_eq!(
                err.code(),
                ErrorCode::HEADER_FORBIDDEN,
                "seed header {name} must fail with header_forbidden"
            );
        }

        // Every forbidden header on enqueue fails with header_forbidden and adds nothing.
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&root_url, &root, ())])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        foxhound.close().await.unwrap();

        let seen_errors = enqueue_errors.lock().unwrap().clone();
        assert_eq!(seen_errors.len(), forbidden.len());
        for (name, code) in seen_errors {
            assert_eq!(
                code,
                ErrorCode::HEADER_FORBIDDEN,
                "enqueue header {name} must fail with header_forbidden"
            );
        }
        assert!(
            site.received_requests()
                .unwrap()
                .of_the_case()
                .all(|req| req.target != "/corpus/article/3"),
            "no forbidden seed or enqueue may be fetched"
        );
    });
}

/// Covers: story:11-1019, operation:crawl_request.enqueue
#[test]
fn blocking_seed_and_enqueue_headers_reach_proving_ground() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let root_url = site.url(HOST, "/corpus/article/1");
    let child_url = site.url(HOST, "/corpus/article/2");

    let foxhound =
        blocking::Foxhound::connect(tier_options(&env, &site, fetch::Tier::Http)).unwrap();
    let mut crawl = blocking::Crawl::new();
    let root = crawl.label::<()>("root").unwrap();
    let child = crawl.label::<()>("child").unwrap();
    let child_label = child.clone();
    let child_for_root = child_url.clone();
    crawl
        .handle(&root, move |ctx: blocking::CrawlContext<()>| {
            let ans = ctx
                .enqueue(&child_for_root, &child_label, ())
                .header("X-Blocking-Tag", "from-blocking")
                .send()?;
            assert!(matches!(ans, Enqueued::Added { .. }), "{ans:?}");
            Ok(())
        })
        .unwrap();
    crawl
        .handle(&child, |_ctx: blocking::CrawlContext<()>| Ok(()))
        .unwrap();

    let run = foxhound
        .start_crawl(
            &crawl,
            [Seed::new(root_url, &root, ()).header("X-Blocking-Seed", "seed-val")],
        )
        .send()
        .unwrap();
    let report = crawl.serve(&run).unwrap();
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    foxhound.close().unwrap();

    assert_eq!(
        observed_header(&site, "/corpus/article/1", "x-blocking-seed").as_deref(),
        Some("seed-val")
    );
    assert_eq!(
        observed_header(&site, "/corpus/article/2", "x-blocking-tag").as_deref(),
        Some("from-blocking")
    );
}
