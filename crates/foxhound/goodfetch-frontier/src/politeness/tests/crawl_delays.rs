//! A Run's Crawl-delays in the Politeness file it shares (ADR 0086): what
//! a copy of a robots.txt asks reaches the file once a Run, and again once
//! the Run reads another copy (#1899).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::politeness::{Politeness, PolitenessOptions};
use goodfetch_engine::robots_txt::{self, Fetched};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;

use super::{KEY, SHOP};
use crate::politeness::{CrawlDelays, TABLES};

/// Every transaction the file committed, whatever it counted on.
fn transactions(work: &CountedWork) -> u64 {
    work.read(Row::PolitenessTransactions) + work.read(Row::PolitenessOtherTransactions)
}

/// A Politeness file at `path` whose transactions `work` counts, outside
/// any Delegation.
async fn opened(path: std::path::PathBuf, work: &CountedWork, owner: &TaskOwner) -> Politeness {
    Politeness::open(
        path,
        &PolitenessOptions {
            work: work.clone(),
            ..PolitenessOptions::default()
        },
        vec![TABLES],
        |_, _| false,
        "engine".to_owned(),
        owner,
    )
    .await
    .unwrap()
}

/// A copy of a robots.txt that asks `seconds` of Crawl-delay, fetched anew.
fn copy_asking(seconds: u64) -> Arc<Fetched> {
    Arc::new(Fetched::File(robots_txt::parse(
        format!("User-agent: *\nCrawl-delay: {seconds}\n").as_bytes(),
    )))
}

/// The Crawl-delay the file holds for [`KEY`] at [`SHOP`], in milliseconds.
fn recorded(path: &std::path::Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT delay FROM crawl_delays WHERE key = ?1 AND location = ?2",
            [KEY, SHOP],
            |row| row.get(0),
        )
        .unwrap()
}

/// Covers: story:6-243, setting:site_preferences.max_crawl_delay
#[tokio::test(start_paused = true)]
async fn a_runs_crawl_delay_reaches_the_file_once_a_copy_of_its_robots_txt() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let work = CountedWork::default();
    let path = env.folder().join("politeness.sqlite");
    let politeness = opened(path.clone(), &work, &owner).await;
    let crawl_delays = CrawlDelays::new(politeness);
    let opened = transactions(&work);
    let copy = copy_asking(5);

    for _ in 0..3 {
        crawl_delays
            .read(KEY, SHOP, &copy, Some(Duration::from_secs(5)))
            .await
            .unwrap();
    }
    let one_copy = transactions(&work) - opened;
    crawl_delays
        .read(KEY, SHOP, &copy_asking(10), Some(Duration::from_secs(10)))
        .await
        .unwrap();
    let another_copy = transactions(&work) - opened - one_copy;

    assert_eq!((one_copy, another_copy, recorded(&path)), (1, 1, 10_000));
}

/// Two Runs share the file: one reading again the copy it recorded leaves
/// the other's newer ask in place, and once it reads a newer copy, even
/// one asking what it recorded before, the file holds that copy's ask.
///
/// Covers: story:6-243
#[tokio::test(start_paused = true)]
async fn a_newer_copy_a_run_reads_is_recorded_even_when_it_asks_what_the_run_recorded_before() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let work = CountedWork::default();
    let path = env.folder().join("politeness.sqlite");
    let politeness = opened(path.clone(), &work, &owner).await;
    let (first, second) = (
        CrawlDelays::new(politeness.clone()),
        CrawlDelays::new(politeness),
    );
    let (five, ten) = (Some(Duration::from_secs(5)), Some(Duration::from_secs(10)));
    let first_copy = copy_asking(5);

    first.read(KEY, SHOP, &first_copy, five).await.unwrap();
    second.read(KEY, SHOP, &copy_asking(10), ten).await.unwrap();
    first.read(KEY, SHOP, &first_copy, five).await.unwrap();
    let after_the_older_copy = recorded(&path);
    first.read(KEY, SHOP, &copy_asking(5), five).await.unwrap();

    assert_eq!((after_the_older_copy, recorded(&path)), (10_000, 5_000));
}

/// A robots.txt's Crawl-delay read is a site's ask, no Permit's work, even
/// when it asks none and changes nothing: its transaction counts on
/// `E6.other_transactions` (#1899).
///
/// Covers: story:6-243
#[tokio::test(start_paused = true)]
async fn a_crawl_delay_read_that_changes_nothing_counts_as_a_sites_ask() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let work = CountedWork::default();
    let politeness = opened(env.folder().join("politeness.sqlite"), &work, &owner).await;
    let crawl_delays = CrawlDelays::new(politeness);
    let (permits, others) = (
        work.read(Row::PolitenessTransactions),
        work.read(Row::PolitenessOtherTransactions),
    );

    crawl_delays
        .read(KEY, SHOP, &Arc::new(Fetched::NotFound(404)), None)
        .await
        .unwrap();

    assert_eq!(
        (
            work.read(Row::PolitenessTransactions) - permits,
            work.read(Row::PolitenessOtherTransactions) - others,
        ),
        (0, 1)
    );
}
