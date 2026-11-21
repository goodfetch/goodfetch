//! A Run's Crawl-delays in the Politeness file it shares (ADR 0086): what
//! a robots.txt asks reaches the file once a Run while it asks the same,
//! and again once it asks another (#1899).

use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::politeness::{Politeness, PolitenessOptions};
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

/// Covers: story:6-243, setting:site_preferences.max_crawl_delay
#[tokio::test(start_paused = true)]
async fn a_runs_crawl_delay_reaches_the_file_once_while_its_robots_txt_asks_the_same() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let work = CountedWork::default();
    let path = env.folder().join("politeness.sqlite");
    let politeness = opened(path.clone(), &work, &owner).await;
    let crawl_delays = CrawlDelays::new(politeness);
    let opened = transactions(&work);

    for _ in 0..3 {
        crawl_delays
            .read(KEY, SHOP, Some(Duration::from_secs(5)))
            .await
            .unwrap();
    }
    let asked_alike = transactions(&work) - opened;
    crawl_delays
        .read(KEY, SHOP, Some(Duration::from_secs(10)))
        .await
        .unwrap();
    let asked_another = transactions(&work) - opened - asked_alike;

    let recorded: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT delay FROM crawl_delays WHERE key = ?1 AND location = ?2",
            [KEY, SHOP],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((asked_alike, asked_another, recorded), (1, 1, 10_000));
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

    crawl_delays.read(KEY, SHOP, None).await.unwrap();

    assert_eq!(
        (
            work.read(Row::PolitenessTransactions) - permits,
            work.read(Row::PolitenessOtherTransactions) - others,
        ),
        (0, 1)
    );
}
