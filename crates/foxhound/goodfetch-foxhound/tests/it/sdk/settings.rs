//! Crawl-space settings, URL-shape caps, and `explain_settings()` for
//! Foxhound's keys (spec #611's #1500; ADRs 0083, 0093 and 0120).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the handlers collect enqueue answers on the test's side"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "writes the test home's Operator policy file"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_foxhound::settings::{Settings, crawl, politeness};
use goodfetch_foxhound::{Crawl, CrawlContext, Enqueued, Foxhound, NotAddedReason, Seed};
use goodfetch_protocol::run::{Completeness, Lifecycle};
use goodfetch_protocol::settings::{ExplainedSetting, SettingsLayer};
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde_json::json;

use super::crawl::site_settings;

const SEED: u64 = 1500;

fn fast_options(env: &TestEnv, site: &ProvingGround) -> goodfetch_foxhound::ConnectOptions {
    super::options(env).settings(
        site_settings(site)
            .set(politeness::domain::CONCURRENCY, 8)
            .set(politeness::domain::DELAY, Duration::ZERO),
    )
}

/// Covers: story:6-227, setting:crawl.max_depth, operation:crawl_request.enqueue
#[test]
fn crawl_max_depth_refuses_deeper_enqueues_and_none_turns_it_off() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let base = |path: &str| site.url("traps.test", path);

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(fast_options(&env, &site)).await.unwrap();
        let answers: Arc<Mutex<Vec<Enqueued>>> = Arc::default();
        let answers_h = Arc::clone(&answers);
        let mut crawl = Crawl::new();
        let page = crawl.label::<u32>("page").unwrap();
        let page_h = page.clone();
        let d1 = base("/archive/more/");
        let d2 = base("/archive/more/more/");
        crawl
            .handle(&page, move |ctx: CrawlContext<u32>| {
                let answers = Arc::clone(&answers_h);
                let page = page_h.clone();
                let d1 = d1.clone();
                let d2 = d2.clone();
                async move {
                    let target = match *ctx.payload() {
                        0 => Some((d1, 1)),
                        1 => Some((d2, 2)),
                        _ => None,
                    };
                    if let Some((next_url, next_depth)) = target {
                        let ans = ctx.enqueue(&next_url, &page, next_depth).await?;
                        answers.lock().unwrap().push(ans);
                    }
                    Ok(())
                }
            })
            .unwrap();

        let capped = foxhound
            .start_crawl(&crawl, [Seed::new(base("/archive/"), &page, 0)])
            .settings(Settings::new().set(crawl::MAX_DEPTH, Some(1)))
            .await
            .unwrap();
        let report = crawl.serve(&capped).await.unwrap();
        let seen = answers.lock().unwrap().clone();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        assert_eq!(report.report.completeness, Completeness::Incomplete);
        assert!(matches!(seen.first(), Some(Enqueued::Added { .. })));
        assert!(matches!(
            seen.get(1),
            Some(Enqueued::NotAdded {
                reason: NotAddedReason::MaxDepth,
                ..
            })
        ));
        assert!(
            report
                .report
                .completeness_reasons
                .iter()
                .any(|reason| reason.code == "crawl.max_depth")
        );

        answers.lock().unwrap().clear();
        let uncapped = foxhound
            .start_crawl(&crawl, [Seed::new(base("/archive/"), &page, 0)])
            .settings(
                Settings::new()
                    .set(crawl::MAX_DEPTH, None)
                    .set(crawl::MAX_SEGMENT_REPEATS, None),
            )
            .await
            .unwrap();
        let uncapped_report = crawl.serve(&uncapped).await.unwrap();
        assert_eq!(uncapped_report.report.completeness, Completeness::Complete);
        assert_eq!(
            answers
                .lock()
                .unwrap()
                .iter()
                .filter(|ans| matches!(ans, Enqueued::Added { .. }))
                .count(),
            2
        );
        foxhound.close().await.unwrap();
    });
}

/// Covers: story:6-227, setting:crawl.max_requests, operation:crawl_request.enqueue
#[test]
fn crawl_max_requests_refuses_excess_enqueues_and_none_turns_it_off() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let base = |path: &str| site.url("traps.test", path);

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(fast_options(&env, &site)).await.unwrap();
        let answers: Arc<Mutex<Vec<Enqueued>>> = Arc::default();
        let answers_h = Arc::clone(&answers);
        let mut crawl = Crawl::new();
        let page = crawl.label::<bool>("page").unwrap();
        let page_h = page.clone();
        let first_url = base("/archive/a");
        let second_url = base("/archive/b");
        crawl
            .handle(&page, move |ctx: CrawlContext<bool>| {
                let answers = Arc::clone(&answers_h);
                let page = page_h.clone();
                let first_url = first_url.clone();
                let second_url = second_url.clone();
                async move {
                    if *ctx.payload() {
                        let first = ctx.enqueue(&first_url, &page, false).await?;
                        let second = ctx.enqueue(&second_url, &page, false).await?;
                        let mut seen = answers.lock().unwrap();
                        seen.push(first);
                        seen.push(second);
                    }
                    Ok(())
                }
            })
            .unwrap();

        let capped = foxhound
            .start_crawl(&crawl, [Seed::new(base("/archive/"), &page, true)])
            .settings(Settings::new().set(crawl::MAX_REQUESTS, Some(2)))
            .await
            .unwrap();
        let report = crawl.serve(&capped).await.unwrap();
        assert_eq!(report.report.completeness, Completeness::Incomplete);
        assert!(matches!(
            answers.lock().unwrap().as_slice(),
            [
                Enqueued::Added { .. },
                Enqueued::NotAdded {
                    reason: NotAddedReason::MaxRequests,
                    ..
                }
            ]
        ));
        assert!(
            report
                .report
                .completeness_reasons
                .iter()
                .any(|reason| reason.code == "crawl.max_requests")
        );

        answers.lock().unwrap().clear();
        let uncapped = foxhound
            .start_crawl(&crawl, [Seed::new(base("/archive/"), &page, true)])
            .settings(Settings::new().set(crawl::MAX_REQUESTS, None))
            .await
            .unwrap();
        let uncapped_report = crawl.serve(&uncapped).await.unwrap();
        assert_eq!(uncapped_report.report.completeness, Completeness::Complete);
        assert!(
            answers
                .lock()
                .unwrap()
                .iter()
                .all(|ans| matches!(ans, Enqueued::Added { .. }))
        );
        foxhound.close().await.unwrap();
    });
}

/// Covers: story:6-228, setting:crawl.max_url_length, setting:crawl.max_path_segments, setting:crawl.max_segment_repeats, setting:crawl.scope.strategy, setting:crawl.scope.exclude, operation:crawl_request.enqueue
#[test]
fn crawl_url_caps_refuse_with_not_added_reasons_and_none_turns_them_off() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let base = |path: &str| site.url("traps.test", path);

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(fast_options(&env, &site)).await.unwrap();
        let answers: Arc<Mutex<Vec<Enqueued>>> = Arc::default();
        let answers_h = Arc::clone(&answers);
        let mut crawl = Crawl::new();
        let page = crawl.label::<bool>("page").unwrap();
        let page_h = page.clone();
        let candidates = [
            base("/archive/very-long-segment-that-exceeds-the-configured-length-cap"),
            base("/a/b/c/d"),
            base("/x/x/y"),
            base("/skip/me"),
        ];
        crawl
            .handle(&page, move |ctx: CrawlContext<bool>| {
                let answers = Arc::clone(&answers_h);
                let page = page_h.clone();
                let candidates = candidates.clone();
                async move {
                    if *ctx.payload() {
                        for url in candidates {
                            let ans = ctx.enqueue(&url, &page, false).await?;
                            answers.lock().unwrap().push(ans);
                        }
                    }
                    Ok(())
                }
            })
            .unwrap();

        let seed_url = base("/archive/");
        let cap_run = foxhound
            .start_crawl(&crawl, [Seed::new(&seed_url, &page, true)])
            .settings(
                Settings::new()
                    .set(crawl::scope::STRATEGY, crawl::scope::Strategy::SameHostname)
                    .set(crawl::scope::EXCLUDE, vec!["**/skip/**".to_owned()])
                    .set(
                        crawl::MAX_URL_LENGTH,
                        Some(u64::try_from(seed_url.len() + 10).unwrap()),
                    )
                    .set(crawl::MAX_PATH_SEGMENTS, Some(3))
                    .set(crawl::MAX_SEGMENT_REPEATS, Some(1)),
            )
            .await
            .unwrap();
        let cap_report = crawl.serve(&cap_run).await.unwrap();
        assert_eq!(cap_report.report.completeness, Completeness::Unverified);
        let reasons: Vec<NotAddedReason> = answers
            .lock()
            .unwrap()
            .iter()
            .map(|ans| match ans {
                Enqueued::NotAdded { reason, .. } => reason.clone(),
                Enqueued::Added { url, .. } => panic!("expected refusal for {url}"),
            })
            .collect();
        assert_eq!(
            reasons,
            vec![
                NotAddedReason::MaxUrlLength,
                NotAddedReason::MaxPathSegments,
                NotAddedReason::MaxSegmentRepeats,
                NotAddedReason::OutOfScope,
            ]
        );
        let report_codes: Vec<&str> = cap_report
            .report
            .completeness_reasons
            .iter()
            .map(|r| r.code.as_str())
            .collect();
        assert!(report_codes.contains(&"crawl.max_url_length"));
        assert!(report_codes.contains(&"crawl.max_path_segments"));
        assert!(report_codes.contains(&"crawl.max_segment_repeats"));

        answers.lock().unwrap().clear();
        let uncapped_run = foxhound
            .start_crawl(&crawl, [Seed::new(&seed_url, &page, true)])
            .settings(
                Settings::new()
                    .set(crawl::scope::EXCLUDE, vec!["**/skip/**".to_owned()])
                    .set(crawl::MAX_URL_LENGTH, None)
                    .set(crawl::MAX_PATH_SEGMENTS, None)
                    .set(crawl::MAX_SEGMENT_REPEATS, None),
            )
            .await
            .unwrap();
        let uncapped_report = crawl.serve(&uncapped_run).await.unwrap();
        assert_eq!(uncapped_report.report.completeness, Completeness::Complete);
        let seen = answers.lock().unwrap().clone();
        assert!(matches!(seen.first(), Some(Enqueued::Added { .. })));
        assert!(matches!(seen.get(1), Some(Enqueued::Added { .. })));
        assert!(matches!(seen.get(2), Some(Enqueued::Added { .. })));
        assert!(matches!(
            seen.get(3),
            Some(Enqueued::NotAdded {
                reason: NotAddedReason::OutOfScope,
                ..
            })
        ));
        foxhound.close().await.unwrap();
    });
}

/// Covers: story:6-314, setting:crawl.max_depth, setting:crawl.max_requests, setting:crawl.max_url_length, setting:crawl.max_path_segments, operation:config.explain
#[test]
fn explain_settings_reports_value_layer_and_lock_for_foxhound_keys_and_matches_report_settings() {
    let env = TestEnv::new();
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.crawl]\nmax_depth = 2\n\n[cap.crawl]\nmax_requests = 10\n",
    )
    .unwrap();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let seed_url = site.url("traps.test", "/archive/");

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(fast_options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&seed_url, &page, ())])
            .settings(Settings::new().set(crawl::MAX_URL_LENGTH, Some(4_096)))
            .await
            .unwrap();

        let explained = run.run().explain_settings().await.unwrap();
        let by_key = |key: &str| -> &ExplainedSetting {
            explained
                .settings
                .iter()
                .find(|entry| entry.key == key)
                .unwrap_or_else(|| panic!("expected `{key}` in explain_settings()"))
        };

        let depth = by_key("crawl.max_depth");
        assert_eq!(depth.value, json!(2));
        assert_eq!(depth.layer, SettingsLayer::OperatorPolicy);
        assert_eq!(depth.locked_by, Some(SettingsLayer::OperatorPolicy));

        let requests = by_key("crawl.max_requests");
        assert_eq!(requests.value, json!(10));
        assert_eq!(requests.layer, SettingsLayer::OperatorPolicy);
        assert_eq!(requests.cap, Some(json!(10)));

        let url_len = by_key("crawl.max_url_length");
        assert_eq!(url_len.value, json!(4_096));
        assert_eq!(url_len.layer, SettingsLayer::Run);
        assert_eq!(url_len.locked_by, None);

        let segments = by_key("crawl.max_path_segments");
        assert_eq!(segments.value, json!(20));
        assert_eq!(segments.layer, SettingsLayer::Default);

        let report = crawl.serve(&run).await.unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        assert_eq!(
            report.report.settings.as_deref(),
            Some(explained.settings.as_slice()),
            "Run report's `settings` list must equal `run.explain_settings().settings`"
        );

        foxhound.close().await.unwrap();
    });
}
