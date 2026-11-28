//! `run.progress()` says why a slow crawl is slow (spec #611's #1496; ADRs
//! 0040, 0086, 0102 and 0118).

use std::time::Duration;

use goodfetch_foxhound::settings::{Settings, crawl as crawl_settings, fetch, politeness};
use goodfetch_foxhound::{ConnectOptions, Crawl, CrawlContext, Foxhound, Seed, SettingsLayer};
use goodfetch_protocol::foxhound::progress::{
    ANALYSIS_PENDING_BYTES, BACKOFF, CrawlRunProgress, DOMAIN_CONCURRENCY, DOMAIN_DELAY, WaitGroup,
};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde_json::json;

use super::crawl::site_settings;

const SEED: u64 = 1496;
const EDGE_HOST: &str = "fetch-edge.test";
const SITEMAP_HOST: &str = "sitemap-seed.test";
const ACCESS_HOST: &str = "access.test";

fn default_politeness_options(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
    let settings = site_settings(site).set(fetch::TIER, fetch::Tier::Http);
    ConnectOptions::default()
        .environment(env.env_os())
        .settings(settings)
}

fn fast_http_options(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
    let settings = site_settings(site)
        .set(fetch::TIER, fetch::Tier::Http)
        .set(politeness::domain::CONCURRENCY, 16)
        .set(politeness::domain::DELAY, Duration::ZERO);
    ConnectOptions::default()
        .environment(env.env_os())
        .settings(settings)
}

/// Covers: story:3-162, story:11-129, story:11-1080, operation:run.progress
#[test]
fn progress_reports_default_politeness_waits_503_backoff_robots_txt_and_concurrency() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();

    real_time_runtime().block_on(async {
        verify_default_politeness_and_concurrency(&env, &site).await;
        verify_503_retry_after_backoff(&env, &site).await;
        verify_disallowing_robots_txt(&env, &site).await;
    });
}

async fn verify_default_politeness_and_concurrency(env: &TestEnv, site: &ProvingGround) {
    let foxhound = Foxhound::connect(default_politeness_options(env, site))
        .await
        .unwrap();
    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    crawl
        .handle(&page, |_ctx: CrawlContext<()>| async {
            tokio::time::sleep(Duration::from_millis(120)).await;
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(
            &crawl,
            [
                Seed::new(site.url(ACCESS_HOST, "/corpus/article/1"), &page, ()),
                Seed::new(site.url(ACCESS_HOST, "/corpus/article/2"), &page, ()),
                Seed::new(site.url(ACCESS_HOST, "/corpus/article/3"), &page, ()),
            ],
        )
        .await
        .unwrap();

    let (served, observed) = tokio::join!(crawl.serve(&run), async {
        let mut stream = run.progress();
        let mut saw_politeness_wait = false;
        let mut saw_next_admitted = false;
        let mut saw_concurrency = false;
        while let Some(step) = stream.next().await {
            let step = step.unwrap();
            if let Some(groups) = &step.waiting {
                for group in groups {
                    if matches!(group.reason.as_str(), DOMAIN_CONCURRENCY | DOMAIN_DELAY) {
                        assert!(
                            group.domains.iter().any(|d| d.domain == ACCESS_HOST),
                            "wait group must name held domain: {group:?}"
                        );
                        let setting = group
                            .setting
                            .as_ref()
                            .expect("politeness wait group names its setting");
                        assert!(
                            matches!(
                                setting.key.as_str(),
                                "politeness.domain.concurrency" | "politeness.domain.delay"
                            ),
                            "unexpected wait setting: {setting:?}"
                        );
                        assert_eq!(setting.layer, SettingsLayer::Default);
                        saw_politeness_wait = true;
                        if group.domains.iter().any(|d| d.next_admitted_at.is_some()) {
                            saw_next_admitted = true;
                        }
                    }
                }
            }
            if let Some(conc) = &step.concurrency
                && conc.handler_concurrency == 16
                && conc.handler_concurrency_layer == Some(SettingsLayer::Default)
                && conc.governor.is_some()
                && conc.allowance_set_by.is_some()
            {
                saw_concurrency = true;
            }
            if step.progress.last {
                break;
            }
        }
        (saw_politeness_wait, saw_next_admitted, saw_concurrency)
    });
    assert_eq!(served.unwrap().report.lifecycle, Lifecycle::Finished);
    assert!(observed.0, "expected domain_concurrency/domain_delay wait");
    assert!(observed.1, "expected next_admitted_at during domain_delay");
    assert!(
        observed.2,
        "expected concurrency with handler_concurrency=16, default layer, governor, and allowance_set_by"
    );
    foxhound.close().await.unwrap();
}

async fn verify_503_retry_after_backoff(env: &TestEnv, site: &ProvingGround) {
    let settings = site_settings(site)
        .set(fetch::TIER, fetch::Tier::Http)
        .set(politeness::domain::CONCURRENCY, 1)
        .set(politeness::domain::DELAY, Duration::ZERO);
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
        .start_crawl(
            &crawl,
            [
                Seed::new(site.url(EDGE_HOST, "/retry-after/503/seconds"), &page, ()),
                Seed::new(site.url(EDGE_HOST, "/after-503"), &page, ()),
            ],
        )
        .await
        .unwrap();

    let (served, saw_backoff) = tokio::join!(crawl.serve(&run), async {
        let mut stream = run.progress();
        let mut found = false;
        while let Some(step) = stream.next().await {
            let step = step.unwrap();
            if let Some(groups) = &step.waiting
                && let Some(group) = groups.iter().find(|g| g.reason == BACKOFF)
            {
                assert!(group.domains.iter().any(|d| d.domain == EDGE_HOST));
                assert!(group.domains.iter().any(|d| d.next_admitted_at.is_some()));
                assert!(group.setting.is_some());
                found = true;
            }
            if step.progress.last {
                break;
            }
        }
        found
    });
    assert_eq!(served.unwrap().report.lifecycle, Lifecycle::Finished);
    assert!(
        saw_backoff,
        "expected backoff wait group after 503 Retry-After"
    );
    foxhound.close().await.unwrap();
}

async fn verify_disallowing_robots_txt(env: &TestEnv, site: &ProvingGround) {
    let foxhound = Foxhound::connect(default_politeness_options(env, site))
        .await
        .unwrap();
    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    crawl
        .handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();

    let run = foxhound
        .start_crawl(
            &crawl,
            [Seed::new(site.url(EDGE_HOST, "/private/secret"), &page, ())],
        )
        .await
        .unwrap();

    let (served, last) = tokio::join!(crawl.serve(&run), async {
        let mut stream = run.progress();
        let mut last = None;
        while let Some(step) = stream.next().await {
            let step = step.unwrap();
            let done = step.progress.last;
            last = Some(step);
            if done {
                break;
            }
        }
        last.expect("final progress value")
    });
    assert_eq!(served.unwrap().report.lifecycle, Lifecycle::Finished);
    let requests = last.requests.as_ref().expect("requests summary");
    assert_eq!(requests.skipped, 1);
    assert!(
        requests
            .by_skip_reason
            .iter()
            .any(|entry| entry.name == "robots_txt" && entry.count == 1),
        "expected robots_txt skip reason in progress: {requests:?}"
    );
    foxhound.close().await.unwrap();
}

/// Covers: story:3-162, story:6-207, story:11-129, story:11-1080, operation:run.progress
#[test]
fn progress_reports_sitemaps_keep_alive_idle_coalescing_and_analysis_pending_bytes() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();

    real_time_runtime().block_on(async {
        verify_sitemaps_progress(&env, &site).await;
        verify_idle_and_coalescing(&env, &site).await;
    });
    verify_analysis_pending_bytes_decoding();
}

async fn verify_sitemaps_progress(env: &TestEnv, site: &ProvingGround) {
    let foxhound = Foxhound::connect(fast_http_options(env, site))
        .await
        .unwrap();
    let mut crawl = Crawl::new();
    let book = crawl.label::<()>("book").unwrap();
    let shelf = crawl.label::<()>("shelf").unwrap();
    let author = crawl.label::<()>("author").unwrap();
    for label in [&book, &shelf, &author] {
        crawl
            .handle(label, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();
    }

    let run = foxhound
        .start_crawl(
            &crawl,
            [
                Seed::new(site.url(SITEMAP_HOST, "/books/1"), &book, ()),
                Seed::sitemap(
                    site.url(SITEMAP_HOST, "/sitemap-index.xml"),
                    [
                        ("**/books/*", &book),
                        ("**/books/**", &shelf),
                        ("**/authors/*", &author),
                    ],
                ),
            ],
        )
        .await
        .unwrap();

    let (served, last) = tokio::join!(crawl.serve(&run), async {
        let mut stream = run.progress();
        let mut last = None;
        while let Some(step) = stream.next().await {
            let step = step.unwrap();
            let done = step.progress.last;
            last = Some(step);
            if done {
                break;
            }
        }
        last.expect("final sitemap progress")
    });
    assert_eq!(served.unwrap().report.lifecycle, Lifecycle::Finished);
    let sitemaps = last.sitemaps.as_ref().expect("sitemaps progress");
    assert_eq!(sitemaps.len(), 1);
    let report = &sitemaps[0];
    assert!(
        report.added > 0,
        "expected added sitemap entries: {report:?}"
    );
    assert!(
        report.not_added > 0,
        "expected not-added sitemap entries: {report:?}"
    );
    let reasons: Vec<&str> = report
        .not_added_by_reason
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert!(
        reasons.contains(&"duplicate"),
        "missing duplicate: {reasons:?}"
    );
    assert!(reasons.contains(&"no_rule"), "missing no_rule: {reasons:?}");
    assert!(
        reasons.contains(&"out_of_scope"),
        "missing out_of_scope: {reasons:?}"
    );
    foxhound.close().await.unwrap();
}

async fn verify_idle_and_coalescing(env: &TestEnv, site: &ProvingGround) {
    let foxhound = Foxhound::connect(fast_http_options(env, site))
        .await
        .unwrap();

    // 1. Keep-alive with all seeds refused: first value arrives immediately with `idle: true`.
    let mut keep_crawl = Crawl::new();
    let keep_page = keep_crawl.label::<()>("page").unwrap();
    keep_crawl
        .handle(&keep_page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();
    let keep_run = foxhound
        .start_crawl(&keep_crawl, [Seed::new("/relative-only", &keep_page, ())])
        .settings(Settings::new().set(crawl_settings::KEEP_ALIVE, true))
        .await
        .unwrap();
    let (served_keep, ()) = tokio::join!(keep_crawl.serve(&keep_run), async {
        let mut progress = keep_run.progress();
        let first = tokio::time::timeout(Duration::from_secs(2), progress.next())
            .await
            .expect("first progress value must arrive at once")
            .unwrap()
            .unwrap();
        assert_eq!(first.progress.lifecycle, Lifecycle::Running);
        assert_eq!(first.idle, Some(true));
        drop(progress);
        keep_run.finish().await.unwrap();
    });
    assert_eq!(served_keep.unwrap().report.lifecycle, Lifecycle::Finished);

    // 2. Coalescing under load: 30 fast requests produce far fewer delivered progress snapshots
    // when the reader pauses while the crawl completes.
    let mut burst_crawl = Crawl::new();
    let burst_page = burst_crawl.label::<()>("page").unwrap();
    burst_crawl
        .handle(&burst_page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();
    let seeds_30: Vec<Seed> = (1..=30_u32)
        .map(|i| {
            Seed::new(
                site.url(ACCESS_HOST, &format!("/corpus/article/1?burst={i}")),
                &burst_page,
                (),
            )
        })
        .collect();
    let burst_run = foxhound.start_crawl(&burst_crawl, seeds_30).await.unwrap();
    let mut progress = burst_run.progress();
    let first = tokio::time::timeout(Duration::from_secs(2), progress.next())
        .await
        .expect("first burst progress value must arrive at once")
        .unwrap()
        .unwrap();
    assert!(!first.progress.last || first.requests.as_ref().is_some_and(|r| r.succeeded == 30));

    let report = burst_crawl.serve(&burst_run).await.unwrap();
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);

    let mut delivered = 1_usize;
    let mut final_succeeded = first.requests.as_ref().map_or(0, |r| r.succeeded);
    if !first.progress.last {
        while let Some(step) = progress.next().await {
            let step = step.unwrap();
            delivered += 1;
            final_succeeded = step.requests.as_ref().map_or(0, |r| r.succeeded);
            if step.progress.last {
                break;
            }
        }
    }
    assert_eq!(final_succeeded, 30);
    assert!(
        delivered < 30,
        "expected progress values to coalesce under load, got {delivered}"
    );
    foxhound.close().await.unwrap();
}

fn verify_analysis_pending_bytes_decoding() {
    let payload = json!({
        "run_id": "pending-bytes",
        "position": 2,
        "at": "2026-11-27T09:00:00Z",
        "lifecycle": "running",
        "writer": "live",
        "epoch": 1,
        "pauses": [],
        "last": false,
        "waiting": [{
            "reason": ANALYSIS_PENDING_BYTES,
            "count": 4,
            "domains": [],
            "setting": {
                "key": "crawl.analysis.max_pending_bytes",
                "value": 67_108_864,
                "layer": "default"
            }
        }],
        "concurrency": {
            "handlers": 2,
            "handler_concurrency": 16,
            "handler_concurrency_layer": "default"
        }
    });
    let decoded: CrawlRunProgress = serde_json::from_value(payload).unwrap();
    let waiting: &[WaitGroup] = decoded.waiting.as_deref().unwrap();
    assert_eq!(waiting[0].reason, ANALYSIS_PENDING_BYTES);
    assert_eq!(
        waiting[0].setting.as_ref().unwrap().key,
        "crawl.analysis.max_pending_bytes"
    );
    assert_eq!(
        waiting[0].setting.as_ref().unwrap().layer,
        SettingsLayer::Default
    );
    assert_eq!(
        decoded
            .concurrency
            .as_ref()
            .unwrap()
            .handler_concurrency_layer,
        Some(SettingsLayer::Default)
    );
}
