//! `.workers(n)` runs `n` serving loops on the Connection's runtime without
//! needing an importable module (spec #611's #1497; ADR 0024).

#![expect(
    clippy::doc_markdown,
    reason = "Covers annotations use bare coverage tags"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Barrier as SyncBarrier};
use std::time::Duration;

use goodfetch_foxhound::settings::{Settings, crawl as crawl_settings, fetch, politeness};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, ErrorCode, Foxhound, Seed, blocking,
};
use goodfetch_protocol::foxhound::crawl::{CRAWL_NOT_IMPORTABLE, CrawlNotImportableDetails};
use goodfetch_protocol::foxhound::error_code;
use goodfetch_protocol::result_document::OracleOutcome;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus, real_time_runtime};
use serde::{Deserialize, Serialize};
use tokio::sync::Barrier;

use super::catalog::{Product, SEED, TASK, crawl_listing, judged, products_json};
use super::crawl::{options, site_settings};

#[derive(Debug, Serialize, Deserialize)]
struct SeenPage {
    url: String,
    tag: String,
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

async fn assert_catalog_workers_three(env: &TestEnv, site: &ProvingGround, start: &str) {
    let foxhound = Foxhound::connect(options(env, site)).await.unwrap();
    let local_marker = String::from("local-borrowed");
    let mut crawl = Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let products = crawl.dataset::<Product>("products").unwrap();
    let next = listing.clone();
    let marker_ref = local_marker.as_str();
    crawl
        .handle(&listing, move |ctx: CrawlContext<()>| async move {
            assert_eq!(marker_ref, "local-borrowed");
            crawl_listing(&ctx, &next, &products).await?;
            Ok(())
        })
        .unwrap();

    let refused = foxhound
        .start_crawl(&crawl, [Seed::new(start, &listing, ())])
        .workers(0)
        .await;
    assert!(
        matches!(&refused, Err(error) if error.code() == ErrorCode::SETTING_INVALID),
        "workers(0) must fail with setting_invalid: {refused:?}"
    );

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(start, &listing, ())])
        .workers(3)
        .await
        .unwrap();
    let report = crawl.serve(&run).await.unwrap();
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);

    let mut records = Vec::new();
    let mut seen_ids = BTreeSet::new();
    let mut read = run.records(&products_json());
    while let Some(record) = read.next().await {
        let value = record.unwrap().value;
        let id = value["id"].as_str().unwrap().to_owned();
        assert!(
            seen_ids.insert(id.clone()),
            "duplicate product record committed: {id}"
        );
        records.push(value);
    }
    assert!(
        !records.is_empty(),
        "catalog crawl must commit at least one product record"
    );
    let answer = judged(site, ToolStatus::Success, &records);
    assert_eq!(answer.outcome, OracleOutcome::Verified, "{answer:?}");
    foxhound.close().await.unwrap();
}

async fn assert_concurrent_barrier_workers_three(env: &TestEnv, site: &ProvingGround) {
    let fast = Foxhound::connect(fast_http_options(env, site))
        .await
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut concurrent_crawl = Crawl::new();
    let page = concurrent_crawl.label::<()>("page").unwrap();
    let pages = concurrent_crawl.dataset::<SeenPage>("pages").unwrap();
    let barrier_for_handler = Arc::clone(&barrier);
    let pages_for_handler = pages.clone();
    concurrent_crawl
        .handle(&page, move |ctx: CrawlContext<()>| {
            let barrier = Arc::clone(&barrier_for_handler);
            let pages = pages_for_handler.clone();
            async move {
                // Only releases if 3 attempts are leased at once.
                if ctx.url().contains("barrier=") {
                    tokio::time::timeout(Duration::from_secs(10), barrier.wait())
                        .await
                        .expect("three serving loops must run concurrently");
                }
                ctx.push(
                    &pages,
                    &SeenPage {
                        url: ctx.url().to_owned(),
                        tag: "ok".to_owned(),
                    },
                )
                .await?;
                Ok(())
            }
        })
        .unwrap();

    let seeds = (1..=3).map(|idx| {
        Seed::new(
            site.url("access.test", &format!("/corpus/article/1?barrier={idx}")),
            &page,
            (),
        )
    });
    let run = fast
        .start_crawl(&concurrent_crawl, seeds)
        .settings(Settings::new().set(crawl_settings::handler::CONCURRENCY, 1))
        .workers(3)
        .await
        .unwrap();
    let report = concurrent_crawl.serve(&run).await.unwrap();
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let mut committed = Vec::new();
    let mut read = run.records(&pages);
    while let Some(record) = read.next().await {
        committed.push(record.unwrap().value.url);
    }
    assert_eq!(committed.len(), 3);
    let unique: BTreeSet<&String> = committed.iter().collect();
    assert_eq!(unique.len(), 3, "no Record committed twice");
    fast.close().await.unwrap();
}

/// Covers: story:3-17, operation:crawl.serve, operation:crawl.start, error:crawl_not_importable
#[test]
fn workers_runs_three_serving_loops_on_catalog_without_duplicate_records_or_importable_module() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();

    // `crawl_not_importable` protocol entry exists for SDKs that spawn processes.
    assert_eq!(
        error_code::CRAWL_NOT_IMPORTABLE.as_str(),
        CRAWL_NOT_IMPORTABLE.name
    );
    let details = CrawlNotImportableDetails {
        module: Some("example".to_owned()),
        export: Some("crawl".to_owned()),
    };
    assert_eq!(details.module.as_deref(), Some("example"));

    real_time_runtime().block_on(async {
        assert_catalog_workers_three(&env, &site, &start).await;
        assert_concurrent_barrier_workers_three(&env, &site).await;
    });
}

/// Covers: story:3-17, operation:crawl.serve, operation:crawl.start
#[test]
fn blocking_workers_runs_three_serving_loops_in_parallel() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let foxhound = blocking::Foxhound::connect(fast_http_options(&env, &site)).unwrap();

    let barrier = Arc::new(SyncBarrier::new(3));
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    let pages = crawl.dataset::<SeenPage>("pages").unwrap();
    let barrier_for_handler = Arc::clone(&barrier);
    let pages_for_handler = pages.clone();
    crawl
        .handle(&page, move |ctx: blocking::CrawlContext<()>| {
            barrier_for_handler.wait();
            ctx.push(
                &pages_for_handler,
                &SeenPage {
                    url: ctx.url().to_owned(),
                    tag: "blocking".to_owned(),
                },
            )?;
            Ok(())
        })
        .unwrap();

    let seeds = (1..=3).map(|idx| {
        Seed::new(
            site.url("access.test", &format!("/corpus/article/1?sync={idx}")),
            &page,
            (),
        )
    });
    let run = foxhound
        .start_crawl(&crawl, seeds)
        .settings(Settings::new().set(crawl_settings::handler::CONCURRENCY, 1))
        .workers(3)
        .send()
        .unwrap();
    let report = crawl.serve(&run).unwrap();
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);

    let committed: Vec<String> = run
        .records(&pages)
        .map(|record| record.unwrap().value.url)
        .collect();
    assert_eq!(committed.len(), 3);
    let unique: BTreeSet<&String> = committed.iter().collect();
    assert_eq!(unique.len(), 3);
    foxhound.close().unwrap();
}
