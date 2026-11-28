//! Politeness defaults and pressing-harder reporting through Foxhound's SDK
//! (ADR 0086; issue 1499): with nothing set, a crawl obeys `robots.txt`, keeps
//! one request in flight per domain and waits at least one second between
//! starts; pressing harder is recorded in `weakened`, logged once as a warning
//! and shown by `explain_settings()`, never set by an environment variable;
//! `politeness.degraded_max` is refused in `settings` with
//! `setting_not_applicable` yet listed by `explain_settings()`; and hedges or
//! `fetch.tier_race` are refused at the default concurrency of 1 naming
//! `politeness.domain.concurrency`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests track concurrent handlers outside loom"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::TryStreamExt as _;
use goodfetch_foxhound::settings::{Settings, fetch, politeness, site_preferences};
use goodfetch_foxhound::{
    Crawl, CrawlContext, CrawlRequestOutcome, ErrorCode, Foxhound, OutcomeKind, RunId, Seed,
};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_protocol::settings::{ExplainedSetting, SettingNotApplicableDetails, SettingsLayer};
use goodfetch_test_support::{ProvingGround, TestEnv, cli, real_time_runtime};
use serde_json::{Value, json};

use super::crawl::{options, site_settings};

/// The Proving ground's seed for these tests.
const SEED: u64 = 1499;

/// Foxhound's executable, used to inspect the Audit log after a linked run.
const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// Covers: story:4-117, story:6-239, story:3-80, story:3-81, setting:politeness.domain.concurrency, setting:politeness.domain.delay, setting:site_preferences.respect_robots_txt
#[test]
fn default_politeness_obeys_robots_txt_keeps_one_in_flight_and_spaces_starts_by_one_second() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let book1 = site.url("sitemap-seed.test", "/books/1");
    let book2 = site.url("sitemap-seed.test", "/books/2");
    let book3 = site.url("sitemap-seed.test", "/books/3");
    let disallowed = site.url("sitemap-seed.test", "/private/books/1");

    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let in_flight_h = Arc::clone(&in_flight);
    let max_in_flight_h = Arc::clone(&max_in_flight);

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        crawl
            .handle(&page, move |_ctx: CrawlContext<()>| {
                let in_flight = Arc::clone(&in_flight_h);
                let max_in_flight = Arc::clone(&max_in_flight_h);
                async move {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .unwrap();

        // No politeness or site_preferences settings at all: pure ADR 0086 defaults.
        #[expect(
            clippy::disallowed_methods,
            reason = "measures wall-clock spacing of real HTTP requests against the Proving ground"
        )]
        let started = Instant::now();
        let run = foxhound
            .start_crawl(
                &crawl,
                [
                    Seed::new(&book1, &page, ()),
                    Seed::new(&book2, &page, ()),
                    Seed::new(&book3, &page, ()),
                    Seed::new(&disallowed, &page, ()),
                ],
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        let elapsed = started.elapsed();

        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        assert_eq!(
            max_in_flight.load(Ordering::SeqCst),
            1,
            "default politeness.domain.concurrency must allow at most 1 request in flight per domain"
        );
        assert!(
            elapsed >= Duration::from_millis(1_800),
            "3 requests on one domain with default 1s politeness.domain.delay must span >= 1.8s, took {elapsed:?}"
        );

        let outcomes: Vec<CrawlRequestOutcome> = run.outcomes().try_collect().await.unwrap();
        let skipped_private = outcomes
            .iter()
            .find(|o| o.request.url == disallowed)
            .expect("disallowed URL must have an outcome");
        assert_eq!(skipped_private.ending.kind(), OutcomeKind::Skipped);

        let received_paths: Vec<String> = site
            .received_requests()
            .unwrap()
            .of_the_case()
            .map(|req| req.target.clone())
            .collect();
        assert!(
            !received_paths.iter().any(|path| path.starts_with("/private/")),
            "robots.txt disallowed path must never be fetched: {received_paths:?}"
        );

        let weakened_politeness: Vec<&str> = report
            .report
            .weakened
            .iter()
            .filter(|w| {
                w.key.starts_with("politeness.") || w.key.starts_with("site_preferences.")
            })
            .map(|w| w.key.as_str())
            .collect();
        assert_eq!(weakened_politeness, Vec::<&str>::new());

        foxhound.close().await.unwrap();
    });
}

/// Covers: story:4-117, setting:politeness.domain.concurrency, setting:politeness.domain.delay, setting:politeness.backoff.enabled, setting:politeness.backoff.initial, setting:politeness.backoff.max, setting:site_preferences.respect_robots_txt, setting:site_preferences.robots_txt_user_agent, error:setting_unknown
#[test]
fn pressed_harder_values_appear_in_weakened_and_explain_settings_and_env_is_refused() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let url = site.url("sitemap-seed.test", "/books/1");
    let run_id = RunId::parse("rust-politeness-pressed").unwrap();

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();

        let pressed = Settings::new()
            .set(politeness::domain::CONCURRENCY, 2)
            .set(politeness::domain::DELAY, Duration::from_millis(250))
            .set(politeness::backoff::ENABLED, false)
            .set(politeness::backoff::INITIAL, Duration::from_secs(1))
            .set(politeness::backoff::MAX, Duration::from_secs(30))
            .set(site_preferences::RESPECT_ROBOTS_TXT, false)
            .set(site_preferences::ROBOTS_TXT_USER_AGENT, "operator-bot");

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&url, &page, ())])
            .run_id(&run_id)
            .settings(pressed)
            .await
            .unwrap();

        let explained = run.run().explain_settings().await.unwrap();
        let report = crawl.serve(&run).await.unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);

        foxhound.close().await.unwrap();

        let exported = cli(&env, FOXHOUND).args(["audit", "export"]).run().unwrap();
        let audit_entries: Vec<Value> = exported
            .stdout
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|entry| entry["kind"] == "politeness_weakened")
            .collect();

        let expected_keys = [
            "politeness.domain.concurrency",
            "politeness.domain.delay",
            "politeness.backoff.enabled",
            "politeness.backoff.initial",
            "politeness.backoff.max",
            "site_preferences.respect_robots_txt",
            "site_preferences.robots_txt_user_agent",
        ];
        for key in expected_keys {
            assert!(
                report
                    .report
                    .weakened
                    .iter()
                    .any(|w| w.key == key && w.layer == SettingsLayer::Run),
                "expected `{key}` in report.weakened at layer Run: {:?}",
                report.report.weakened
            );
            let entry: &ExplainedSetting = explained
                .settings
                .iter()
                .find(|s| s.key == key)
                .unwrap_or_else(|| panic!("expected `{key}` in explain_settings()"));
            assert_eq!(entry.layer, SettingsLayer::Run, "{entry:?}");
            assert!(entry.weakens, "expected `{key}` weakens=true: {entry:?}");
            let audit_count = audit_entries
                .iter()
                .filter(|e| e["run_id"] == run_id.as_str() && e["details"]["setting"] == key)
                .count();
            assert_eq!(
                audit_count, 1,
                "expected exactly 1 politeness_weakened Audit entry for `{key}` on `{run_id}`"
            );
        }

        // Pressing harder through an environment variable is refused (`setting_unknown`).
        let mut bad_env = env.env_os();
        bad_env.insert("GOODFETCH_POLITENESS_DOMAIN_CONCURRENCY".into(), "4".into());
        let refused = Foxhound::connect(
            super::options(&env)
                .environment(bad_env)
                .settings(site_settings(&site)),
        )
        .await
        .expect_err("environment variable cannot set politeness.domain.concurrency");
        assert_eq!(refused.code(), ErrorCode::SETTING_UNKNOWN);
    });
}

/// Covers: story:3-63, story:6-263, setting:politeness.degraded_max, setting:fetch.tier_race, setting:fetch.hedge.enabled, error:setting_not_applicable
#[test]
fn degraded_max_is_refused_in_settings_yet_explained_and_hedges_and_tier_race_refused_at_default_concurrency()
 {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let url = site.url("sitemap-seed.test", "/books/1");

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();

        // `politeness.degraded_max` is listed by `explain_settings()` with its 10m default.
        let explained = foxhound.explain_settings().await.unwrap();
        let degraded = explained
            .settings
            .iter()
            .find(|s| s.key == "politeness.degraded_max")
            .expect("explain_settings() must list politeness.degraded_max");
        assert_eq!(degraded.value, json!(600_000));
        assert_eq!(degraded.layer, SettingsLayer::Default);
        assert!(degraded.user_settable);

        // Passing `politeness.degraded_max` in Run settings is refused with `setting_not_applicable`.
        let refused_degraded = foxhound
            .call("crawl.start")
            .params(json!({
                "labels": { "page": {} },
                "seeds": [{ "url": url, "label": "page" }],
                "settings": { "politeness": { "degraded_max": 60_000 } },
            }))
            .await
            .expect_err("politeness.degraded_max must be refused in program settings");
        assert_eq!(refused_degraded.code(), ErrorCode::SETTING_NOT_APPLICABLE);

        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })
            .unwrap();

        // With no politeness settings (default concurrency = 1), `fetch.tier_race` is refused naming the key.
        let refused_race = foxhound
            .start_crawl(&crawl, [Seed::new(&url, &page, ())])
            .settings(Settings::new().set(fetch::TIER_RACE, true))
            .await
            .expect_err("fetch.tier_race at default concurrency 1 must be refused");
        assert_eq!(refused_race.code(), ErrorCode::SETTING_NOT_APPLICABLE);
        let race_details = refused_race
            .details::<SettingNotApplicableDetails>()
            .expect("setting_not_applicable details");
        assert_eq!(race_details.key, "fetch.tier_race");
        assert!(
            race_details.fix.contains("politeness.domain.concurrency"),
            "fix must name politeness.domain.concurrency: {race_details:?}"
        );

        // Same for `fetch.hedge.enabled` at default concurrency = 1.
        let refused_hedge = foxhound
            .start_crawl(&crawl, [Seed::new(&url, &page, ())])
            .settings(Settings::new().set(fetch::hedge::ENABLED, true))
            .await
            .expect_err("fetch.hedge.enabled at default concurrency 1 must be refused");
        assert_eq!(refused_hedge.code(), ErrorCode::SETTING_NOT_APPLICABLE);
        let hedge_details = refused_hedge
            .details::<SettingNotApplicableDetails>()
            .expect("setting_not_applicable details");
        assert_eq!(hedge_details.key, "fetch.hedge.enabled");
        assert!(
            hedge_details.fix.contains("politeness.domain.concurrency"),
            "fix must name politeness.domain.concurrency: {hedge_details:?}"
        );

        foxhound.close().await.unwrap();
    });
}
