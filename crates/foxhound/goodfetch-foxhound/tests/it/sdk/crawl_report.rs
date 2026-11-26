//! A crawl's Run report and progress, typed as Foxhound's in Rust (ADRs
//! 0102 and 0120): `finish`, `stop` and `report` answer a
//! [`CrawlRunReport`], whose request totals, Records per Dataset and
//! Completeness are the crawl's, a cap naming its setting and its count;
//! `progress` hands a [`CrawlRunProgress`]; and `start_crawl` takes the
//! Run's own settings, Limits and tags.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::future::IntoFuture as _;
use std::time::SystemTime;

use goodfetch_foxhound::settings::{Settings, crawl};
use goodfetch_foxhound::{
    Answer, Crawl, CrawlContext, CrawlRunProgress, CrawlRunReport, Foxhound, Label, Limits, RunId,
    Seed, Tags, UsageKind,
};
use goodfetch_protocol::run::{Completeness, Lifecycle, NothingDone, SeedsKept};
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::crawl::options;

/// The Proving ground's seed the crawls read.
const SITE: u64 = 1602;

/// A page's Payload: how far from the seed it is.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Visit {
    depth: u32,
}

/// The Record each page pushes.
#[derive(Debug, Serialize)]
struct Visited {
    url: String,
}

/// The crawl each test starts: each page pushes its URL to `pages`, and
/// the seed enqueues `next`.
fn linking_crawl(next: String) -> (Crawl<'static>, Label<Visit>) {
    let mut crawl = Crawl::new();
    let page = crawl.label::<Visit>("page").unwrap();
    let pages = crawl.dataset::<Visited>("pages").unwrap();
    let linked = page.clone();
    crawl
        .handle(&page, move |ctx: CrawlContext<Visit>| {
            let (pages, linked, next) = (pages.clone(), linked.clone(), next.clone());
            async move {
                let visited = Visited {
                    url: ctx.url().to_owned(),
                };
                ctx.push(&pages, &visited).await?;
                if ctx.payload().depth == 0 {
                    ctx.enqueue(&next, &linked, Visit { depth: 1 }).await?;
                }
                Ok(())
            }
        })
        .unwrap();
    (crawl, page)
}

/// The report `finish` answers for the crawl of `book`, which links
/// `empty`, started with `settings`, once its serve returned.
fn finished(settings: impl FnOnce(&str) -> Settings) -> Answer<CrawlRunReport> {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    let settings = settings(&book);
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, page) = linking_crawl(empty);
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .settings(settings)
            .await
            .unwrap();
        crawl.serve(&run).await.unwrap();
        let finished: Answer<CrawlRunReport> = run.finish().await.unwrap();
        let reported: Answer<CrawlRunReport> = run.report().await.unwrap();
        foxhound.close().await.unwrap();
        assert_eq!(*finished, *reported, "finish and report answer one report");
        finished
    })
}

/// The `(code, count)` of each Completeness reason of `report`.
fn reasons(report: &CrawlRunReport) -> Vec<(&str, Option<u64>)> {
    report
        .report
        .completeness_reasons
        .iter()
        .map(|reason| (reason.code.as_str(), reason.count))
        .collect()
}

/// A crawl that completes: `finish` answers its request totals, its
/// Records per Dataset and `complete`, each point in time a `SystemTime`.
///
/// Covers: story:11-1080, story:11-1082, operation:run.finish, operation:run.report
#[test]
fn finish_answers_a_crawls_report_typed() {
    let report = finished(|_| Settings::new());

    let requests = report.requests.clone().unwrap();
    let records = report.records.clone().unwrap();
    let (started_at, ended_at): (SystemTime, SystemTime) =
        (report.report.started_at, report.report.ended_at);
    assert_eq!(
        (
            report.report.lifecycle,
            (requests.succeeded, requests.failed, requests.pending),
            records
                .iter()
                .map(|records| (records.dataset.as_str(), records.committed))
                .collect::<Vec<_>>(),
            report.report.completeness,
            started_at <= ended_at,
        ),
        (
            Lifecycle::Finished,
            (2, 0, 0),
            vec![("pages", 2)],
            Completeness::Complete,
            true,
        ),
        "{report:?}"
    );
}

/// A crawl `crawl.max_requests` cut short is `incomplete`, its reason
/// naming the setting and counting the URLs it refused.
///
/// Covers: story:11-1080, setting:crawl.max_requests
#[test]
fn a_crawl_max_requests_cut_short_is_incomplete_naming_the_setting() {
    let report = finished(|_| Settings::new().set(crawl::MAX_REQUESTS, Some(1)));

    assert_eq!(
        (
            report.requests.clone().unwrap().succeeded,
            report.report.completeness,
            reasons(&report),
        ),
        (
            1,
            Completeness::Incomplete,
            vec![("crawl.max_requests", Some(1))]
        ),
        "{report:?}"
    );
}

/// A crawl whose cap on a URL's length refused a link is `unverified`,
/// its reason naming the setting and counting the URLs it refused.
///
/// Covers: story:11-1080, setting:crawl.max_url_length
#[test]
fn a_crawl_whose_cap_refused_a_url_is_unverified_naming_the_setting() {
    let report = finished(|book| {
        let length = u64::try_from(book.len()).unwrap();
        Settings::new().set(crawl::MAX_URL_LENGTH, Some(length))
    });

    assert_eq!(
        (
            report.requests.clone().unwrap().succeeded,
            report.report.completeness,
            reasons(&report),
        ),
        (
            1,
            Completeness::Unverified,
            vec![("crawl.max_url_length", Some(1))]
        ),
        "{report:?}"
    );
}

/// A keep-alive crawl's progress, typed as a crawl's, says once it waits
/// idle; `stop` then stops it with a Checkpoint, its report `stopped`, and
/// its serve returns. The start carries its own tags and Limits.
///
/// Covers: story:11-1081, operation:run.stop, operation:run.progress, setting:crawl.keep_alive
#[test]
fn stop_stops_an_idle_keep_alive_crawl_with_a_checkpoint() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, page) = linking_crawl(empty);
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .run_id(&RunId::parse("kept").unwrap())
            .tags(Tags::parse(["crawls"]).unwrap())
            .limits(Limits::new().count(UsageKind::REQUESTS, 10))
            .settings(Settings::new().set(crawl::KEEP_ALIVE, true))
            .await
            .unwrap();
        let stopping = async {
            let mut progress = run.progress();
            loop {
                let read: Answer<CrawlRunProgress> = progress.next().await.unwrap().unwrap();
                if read.idle == Some(true) {
                    break read;
                }
            }
        };
        let (served, idle) = tokio::join!(crawl.serve(&run), async {
            let idle = stopping.await;
            let stopped: Answer<CrawlRunReport> = run.stop().await.unwrap();
            (idle, stopped)
        });
        let (idle, stopped) = idle;
        let served = served.unwrap();
        let tagged = foxhound
            .call("run.list")
            .params(json!({ "tags": ["crawls"] }))
            .await
            .unwrap();
        foxhound.close().await.unwrap();

        let limits: Vec<_> = stopped
            .report
            .usage
            .limits
            .iter()
            .map(|limit| (limit.scope.kind.as_str(), limit.usage_kind.as_str()))
            .collect();
        assert_eq!(
            (
                idle.requests.clone().map(|requests| requests.pending),
                stopped.report.lifecycle,
                stopped.report.checkpoint.is_some(),
                served.report.lifecycle,
                tagged["runs"][0]["run_id"].clone(),
                limits,
            ),
            (
                Some(0),
                Lifecycle::Stopped,
                true,
                Lifecycle::Stopped,
                json!("kept"),
                vec![("run", "requests")],
            ),
            "{stopped:?}"
        );
    });
}

/// A crawl none of whose seeds reached a handler finishes saying so,
/// counting its seeds by why.
///
/// Covers: story:11-1081, operation:run.finish
#[test]
fn a_crawl_that_handled_no_seed_says_nothing_done() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, page) = linking_crawl(site.url("shop.test", "/empty"));
        let run = foxhound
            .start_crawl(
                &crawl,
                [Seed::new("ftp://shop.test/a", &page, Visit { depth: 0 })],
            )
            .await
            .unwrap();
        let (served, finished) = tokio::join!(crawl.serve(&run), run.finish().into_future());
        foxhound.close().await.unwrap();

        assert_eq!(
            (
                served.unwrap().report.nothing_done,
                finished.unwrap().report.nothing_done.clone()
            ),
            (Some(no_seed_handled()), Some(no_seed_handled()),)
        );
    });
}

/// The `nothingDone` of a crawl whose one seed had a scheme it does not
/// fetch.
fn no_seed_handled() -> NothingDone {
    NothingDone::NoSeedHandled {
        seeds: vec![SeedsKept {
            reason: "unsupported_scheme".to_owned(),
            count: 1,
        }],
    }
}
