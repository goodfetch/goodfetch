//! A crawl in Rust on the linked engine (ADRs 0093, 0109 and 0112): the
//! program declares a [`Crawl`] with no connection, starts its Run with
//! `start_crawl` and serves it with `crawl.serve(&run)`, whose handlers run
//! inside that future, until the Run ends and its report returns; or, for
//! a crawl with no handler, answers its requests itself through
//! `run.crawl_requests()`. A program that does not use async does the same
//! with `blocking::Crawl`, and a web service serves its crawl from an axum
//! handler.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::{self, Body};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::{Router, routing};
use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::{Settings, egress, fetch, network};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRunReport, Error, ErrorCode, Foxhound, Label,
    LabelDeclaration, RunId, Schema, Seed, blocking, error_code,
};
use goodfetch_protocol::foxhound::crawl::LabelUnknownDetails;
use goodfetch_protocol::foxhound::frontier::{PayloadInvalidDetails, PayloadSchemaChangedDetails};
use goodfetch_protocol::foxhound::report::CountOf;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc};
use tower_service::Service as _;

/// The Proving ground's seed the crawls read.
const SITE: u64 = 1602;

/// A page's Payload: how far from the seed it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Visit {
    depth: u32,
}

/// The Record each page pushes.
#[derive(Debug, Serialize)]
struct Visited {
    url: String,
    depth: u32,
}

/// What a connection that reaches the Proving ground on a forced `http`
/// tier opens with.
pub(super) fn options(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
    let resolve: BTreeMap<String, Vec<String>> = site
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), vec![address.clone()]))
        .collect();
    let hosts: Vec<String> = resolve.keys().cloned().collect();
    super::options(env).settings(
        Settings::new()
            .set(network::RESOLVE, resolve)
            .set(egress::PRIVATE_RANGES, hosts)
            .set(fetch::TIER, fetch::Tier::Http),
    )
}

/// The tracer crawl: each page pushes a Record, and the seed enqueues
/// `empty`, so a Run seeded at a page ends with two requests succeeded.
fn tracer_crawl(empty: String) -> (Crawl<'static>, Label<Visit>) {
    let mut crawl = Crawl::new();
    let page = crawl.label::<Visit>("page").unwrap();
    let pages = crawl.dataset::<Visited>("pages").unwrap();
    let next = (page.clone(), empty);
    crawl
        .handle(&page, move |ctx: CrawlContext<Visit>| async move {
            let depth = ctx.payload().depth;
            ctx.push(
                &pages,
                &Visited {
                    url: ctx.url().to_owned(),
                    depth,
                },
            )
            .await?;
            if depth == 0 {
                ctx.enqueue(&next.1, &next.0, Visit { depth: 1 }).await?;
            }
            Ok(())
        })
        .unwrap();
    (crawl, page)
}

/// Covers: story:11-1013, story:11-1014, story:11-1015, story:11-1075, story:11-1101
#[test]
fn a_crawl_serves_its_run_on_the_linked_engine_until_the_run_ends() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let tracer = RunId::parse("tracer").unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, page) = tracer_crawl(empty);

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .run_id(&tracer)
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.report.run_id, tracer);
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (2, 0),
            "{requests:?}"
        );
    });
}

/// A program that does not use async declares the tracer crawl on a
/// `blocking::Crawl`: its handlers are plain functions, whose calls block
/// until answered, and its `serve` blocks the test's thread, which runs no
/// runtime, until the Run ends and its report returns.
///
/// Covers: story:11-1013, story:11-1104
#[test]
fn a_blocking_crawl_serves_its_run_from_a_thread_with_no_runtime() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let tracer = RunId::parse("tracer").unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    let foxhound = blocking::Foxhound::connect(options(&env, &site)).unwrap();
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<Visit>("page").unwrap();
    let pages = crawl.dataset::<Visited>("pages").unwrap();
    let next = page.clone();
    crawl
        .handle(&page, move |ctx: blocking::CrawlContext<Visit>| {
            let depth = ctx.payload().depth;
            let visited = Visited {
                url: ctx.url().to_owned(),
                depth,
            };
            ctx.push(&pages, &visited)?;
            if depth == 0 {
                ctx.enqueue(empty.clone(), &next, Visit { depth: 1 })?;
            }
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
        .run_id(&tracer)
        .send()
        .unwrap();
    let report = crawl.serve(&run).unwrap();
    foxhound.close().unwrap();

    assert_eq!(report.report.run_id, tracer);
    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let requests = report.requests.unwrap();
    assert_eq!(
        (requests.succeeded, requests.failed),
        (2, 0),
        "{requests:?}"
    );
}

/// What the web service's handler holds: one connection and one crawl,
/// declared once, which serve every call.
#[derive(Clone)]
struct WebService {
    foxhound: Foxhound,
    crawl: Arc<Crawl<'static>>,
    page: Label<Visit>,
    book: String,
}

/// The web handler: each call crawls from the book's page and answers how
/// its Run ended.
async fn crawl_the_book(State(service): State<WebService>) -> Result<String, String> {
    let seed = Seed::new(&service.book, &service.page, Visit { depth: 0 });
    let run = service
        .foxhound
        .start_crawl(&service.crawl, [seed])
        .await
        .map_err(|error| error.to_string())?;
    let report = service
        .crawl
        .serve(&run)
        .await
        .map_err(|error| error.to_string())?;
    let requests = report.requests.unwrap_or_default();
    Ok(format!(
        "{:?}: {} succeeded, {} failed",
        report.report.lifecycle, requests.succeeded, requests.failed
    ))
}

/// The tracer crawl, declared once, serves each call of an axum handler,
/// whose future axum requires to be `Send`: two calls at once, each a task
/// of a multi-threaded runtime, as a web server runs them, and the Router,
/// driven as a tower Service, answers each once its Run has ended.
///
/// Covers: story:11-1013, story:11-1101
#[test]
fn an_axum_handler_serves_the_tracer_crawl_on_every_call() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    let runtime = goodfetch_runtime::multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, page) = tracer_crawl(empty);
        let service = WebService {
            foxhound: foxhound.clone(),
            crawl: Arc::new(crawl),
            page,
            book,
        };
        let router = Router::new()
            .route("/crawl", routing::post(crawl_the_book))
            .with_state(service);

        let mut calls = Vec::new();
        for _call in 0..2 {
            let mut router = router.clone();
            let request = Request::post("/crawl").body(Body::empty()).unwrap();
            #[expect(
                clippy::disallowed_methods,
                reason = "the test spawns each call as a web server's own task would"
            )]
            let call = tokio::spawn(async move {
                let response = router.call(request).await.unwrap();
                let status = response.status();
                let body = body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                (status, String::from_utf8(body.to_vec()).unwrap())
            });
            calls.push(call);
        }
        let mut answers = Vec::new();
        for call in calls {
            answers.push(call.await.unwrap());
        }
        foxhound.close().await.unwrap();

        let answered = (StatusCode::OK, "Finished: 2 succeeded, 0 failed".to_owned());
        assert_eq!(answers, [answered.clone(), answered]);
    });
}

/// Covers: story:11-1025, story:11-1102
#[test]
fn a_panic_in_a_handler_fails_its_request_with_handler_failed_and_serve_goes_on() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        crawl
            .handle(&page, |ctx: CrawlContext<Visit>| async move {
                assert!(ctx.payload().depth != 0, "the handler panics on the seed");
                Ok(())
            })
            .unwrap();

        let seeds = [
            Seed::new(&book, &page, Visit { depth: 0 }),
            Seed::new(&empty, &page, Visit { depth: 1 }),
        ];
        let run = foxhound.start_crawl(&crawl, seeds).await.unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (1, 1),
            "{requests:?}"
        );
        assert_eq!(
            count(&requests.by_error_code, "handler_failed"),
            1,
            "{requests:?}"
        );
    });
}

/// How many requests `counts` lists under `name`.
fn count(counts: &[CountOf], name: &str) -> u64 {
    counts
        .iter()
        .find(|counted| counted.name == name)
        .map_or(0, |counted| counted.count)
}

/// `?` on the program's own error fails the request with
/// `handler_failed`, while the SDK's own `Error` keeps its code: an
/// enqueue to another crawl's Label fails with `label_unknown`.
///
/// Covers: story:11-1016, story:11-1101
#[test]
fn a_handlers_error_is_handler_failed_and_an_sdk_error_keeps_its_code() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let mut other = Crawl::new();
        let elsewhere = other.label::<Visit>("elsewhere").unwrap();
        crawl
            .handle(&page, move |ctx: CrawlContext<Visit>| async move {
                if ctx.payload().depth == 0 {
                    let _shelf: u32 = "no shelf".parse()?;
                } else {
                    ctx.enqueue(ctx.url().to_owned(), &elsewhere, Visit { depth: 2 })
                        .await?;
                }
                Ok(())
            })
            .unwrap();

        let seeds = [
            Seed::new(&book, &page, Visit { depth: 0 }),
            Seed::new(&empty, &page, Visit { depth: 1 }),
        ];
        let run = foxhound.start_crawl(&crawl, seeds).await.unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            [
                count(&requests.by_error_code, "handler_failed"),
                count(&requests.by_error_code, "label_unknown"),
            ],
            [1, 1],
            "{requests:?}"
        );
    });
}

/// `ctx.skip` ends its request `skipped` with the reason `handler`, and
/// `ctx.retry` ends its attempt with `retry_requested`, so the request is
/// delivered again, even when the handler swallowed the retry: every later
/// call on its context fails with `attempt_ended`, which no outcome
/// carries.
///
/// Covers: story:11-1103, error:retry_requested
#[test]
fn a_skip_and_a_swallowed_retry_end_their_attempts_as_asked() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    let (seen, mut sightings) = mpsc::unbounded_channel();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let pages = crawl.dataset::<Visited>("pages").unwrap();
        crawl
            .handle(&page, move |ctx: CrawlContext<Visit>| async move {
                let depth = ctx.payload().depth;
                if depth == 0 {
                    return Err(ctx.skip("no book here"));
                }
                if ctx.attempt() == 1 {
                    let _swallowed = ctx.retry("once more");
                    let visited = Visited {
                        url: ctx.url().to_owned(),
                        depth,
                    };
                    let pushed = ctx.push(&pages, &visited).await.unwrap_err();
                    seen.send(pushed.code().to_string()).unwrap();
                }
                seen.send(format!("attempt {}", ctx.attempt())).unwrap();
                Ok(())
            })
            .unwrap();

        let seeds = [
            Seed::new(&book, &page, Visit { depth: 0 }),
            Seed::new(&empty, &page, Visit { depth: 1 }),
        ];
        let run = foxhound.start_crawl(&crawl, seeds).await.unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.skipped, requests.failed),
            (1, 1, 0),
            "{requests:?}"
        );
        assert_eq!(
            count(&requests.by_skip_reason, "handler"),
            1,
            "{requests:?}"
        );
    });
    let mut sighted = Vec::new();
    while let Ok(sighting) = sightings.try_recv() {
        sighted.push(sighting);
    }
    assert_eq!(sighted, ["attempt_ended", "attempt 1", "attempt 2"]);
}

/// A handler that retries every attempt runs `crawl.retry.max` more
/// times, 2 by default, then its request ends `failed` with
/// `retry_requested`.
///
/// Covers: error:retry_requested
#[test]
fn a_request_retried_past_its_retries_fails_with_retry_requested() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    let (seen, mut attempts) = mpsc::unbounded_channel();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        crawl
            .handle(&page, move |ctx: CrawlContext<Visit>| async move {
                seen.send(ctx.attempt()).unwrap();
                Err(ctx.retry("the shelf is still empty"))
            })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            (
                requests.failed,
                count(&requests.by_error_code, "retry_requested")
            ),
            (1, 1),
            "{requests:?}"
        );
    });
    let mut seen_attempts = Vec::new();
    while let Ok(attempt) = attempts.try_recv() {
        seen_attempts.push(attempt);
    }
    assert_eq!(seen_attempts, [1, 2, 3]);
}

/// Dropping the `serve` future gives the Crawl requests it was handling
/// back to the engine, uncounted: the next serve of the Run is handed the
/// same request again as its first attempt, and the Run finishes with it.
///
/// Covers: story:11-1075
#[test]
fn dropping_serve_gives_its_requests_back_uncounted() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    let (started, mut starts) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let opened = Arc::clone(&gate);
        crawl
            .handle(&page, move |ctx: CrawlContext<Visit>| async move {
                started.send(ctx.attempt()).unwrap();
                let _permit = opened.acquire().await?;
                Ok(())
            })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .await
            .unwrap();
        tokio::select! {
            ended = crawl.serve(&run) => panic!("the serve ended first: {ended:?}"),
            first = starts.recv() => assert_eq!(first, Some(1)),
        }
        gate.add_permits(1);
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(starts.recv().await, Some(1), "delivered again, uncounted");
        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (1, 0),
            "{requests:?}"
        );
    });
}

/// The serve's future is `Send` and, for a crawl whose handlers borrow
/// nothing, `'static`: a spawned task serves the Run. Its handler runs
/// inside that task, the one task alive on the program's runtime: the SDK
/// spawns none of its own there.
///
/// Covers: story:11-1025
#[test]
fn a_spawned_task_serves_a_crawl() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let (seen, mut handled) = mpsc::unbounded_channel();
        crawl
            .handle(&page, move |_ctx: CrawlContext<Visit>| {
                let seen = seen.clone();
                async move {
                    let alive = tokio::runtime::Handle::current()
                        .metrics()
                        .num_alive_tasks();
                    seen.send((tokio::task::try_id(), alive)).unwrap();
                    Ok(())
                }
            })
            .unwrap();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .await
            .unwrap();

        #[expect(
            clippy::disallowed_methods,
            reason = "the test spawns the serve as a program's own task would"
        )]
        let served = tokio::spawn(async move { crawl.serve(&run).await });
        let serving = served.id();
        let report = served.await.unwrap().unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.requests.unwrap().succeeded, 1);
        assert_eq!(handled.recv().await, Some((Some(serving), 1)));
    });
}

/// A Run whose serve stopped with its connection resumes on a new
/// connection under its Run ID: the new serve is handed the request the
/// first one was running, uncounted, as its first attempt, and the Run
/// finishes with it.
///
/// Covers: story:11-1075
#[test]
fn a_run_resumed_on_a_new_connection_continues_the_requests_given_back() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    let resumed = RunId::parse("resumed").unwrap();
    let (started, mut starts) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    real_time_runtime().block_on(async {
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let opened = Arc::clone(&gate);
        crawl
            .handle(&page, move |ctx: CrawlContext<Visit>| async move {
                started.send((ctx.run_id().clone(), ctx.attempt())).unwrap();
                let _permit = opened.acquire().await?;
                Ok(())
            })
            .unwrap();
        let seeds = || [Seed::new(&book, &page, Visit { depth: 0 })];

        let first = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = first
            .start_crawl(&crawl, seeds())
            .run_id(&resumed)
            .await
            .unwrap();
        tokio::select! {
            ended = crawl.serve(&run) => panic!("the serve ended first: {ended:?}"),
            first = starts.recv() => assert_eq!(first, Some((resumed.clone(), 1))),
        }
        first.close().await.unwrap();

        gate.add_permits(1);
        let second = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = second
            .start_crawl(&crawl, seeds())
            .run_id(&resumed)
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        second.close().await.unwrap();

        assert_eq!(
            starts.recv().await,
            Some((resumed.clone(), 1)),
            "delivered again on the new connection, uncounted"
        );
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (1, 0),
            "{requests:?}"
        );
    });
}

/// A crawl that attaches no handler is answered by the program itself:
/// `run.crawl_requests()` hands each Crawl request as a `CrawlCall`, whose
/// context pushes as a handler's does and whose `finish` answers it as a
/// handler's return does. A call dropped unanswered goes back to pending,
/// uncounted, and comes again as its first attempt; the stream ends once
/// the Run ends, as `serve` returns then.
///
/// Covers: story:11-1105
#[test]
fn a_program_answers_its_crawl_requests_itself() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let (book, empty) = (
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    );
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let pages = crawl.dataset::<Visited>("pages").unwrap();
        let seeds = [
            Seed::new(&book, &page, Visit { depth: 0 }),
            Seed::new(&empty, &page, Visit { depth: 1 }),
        ];
        let run = foxhound.start_crawl(&crawl, seeds).await.unwrap();
        let mut calls = run.crawl_requests();
        let mut dropped = None;
        let mut seen = Vec::new();
        while let Some(call) = calls.next().await {
            let call = call.unwrap();
            let context = call.context();
            seen.push((context.url().to_owned(), context.attempt()));
            if dropped.is_none() {
                dropped = Some(context.url().to_owned());
                drop(call);
                continue;
            }
            let visit: Visit = serde_json::from_value(context.payload().clone()).unwrap();
            let visited = Visited {
                url: context.url().to_owned(),
                depth: visit.depth,
            };
            context.push(&pages, &visited).await.unwrap();
            call.finish(Ok(())).unwrap();
        }
        drop(calls);
        let ended = run.run().report().no_timeout().await.unwrap();
        foxhound.close().await.unwrap();

        let dropped = dropped.unwrap();
        let attempts: Vec<u32> = seen
            .iter()
            .filter(|(url, _)| *url == dropped)
            .map(|(_, attempt)| *attempt)
            .collect();
        assert_eq!(attempts, [1, 1], "delivered again, uncounted: {seen:?}");
        let report: CrawlRunReport = serde_json::from_value(ended.raw().clone()).unwrap();
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (2, 0),
            "{requests:?}"
        );
    });
}

/// A crawl that attaches no handler starts, for the program to answer its
/// requests itself, but its `serve` fails at once with `label_unhandled`
/// rather than wait on requests no handler answers.
#[test]
fn serving_a_crawl_with_no_handler_fails_with_label_unhandled() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let book = site.url("shop.test", "/book");
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&book, &page, Visit { depth: 0 })])
            .await
            .unwrap();
        let unhandled = crawl.serve(&run).await.unwrap_err();
        foxhound.close().await.unwrap();

        assert_eq!(unhandled.code(), error_code::LABEL_UNHANDLED, "{unhandled}");
    });
}

/// A Label without a handler, beside one with a handler, fails the start,
/// before anything is sent.
///
/// Covers: story:11-1015, story:11-1016
#[test]
fn a_crawl_with_a_label_left_unhandled_is_not_started() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let _detail = crawl.label::<Visit>("detail").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<Visit>| async { Ok(()) })
            .unwrap();

        let seed = Seed::new("https://shop.test/", &page, Visit { depth: 0 });
        let started = foxhound.start_crawl(&crawl, [seed]).await;
        foxhound.close().await.unwrap();

        let error = started.unwrap_err();
        assert_eq!(error.code(), error_code::LABEL_UNHANDLED, "{error}");
    });
}

/// A crawl's Run carries the project file its connection read (ADR 0083):
/// `crawl.start` sends it, so the Run explains the file's value at
/// `project_configuration`.
///
/// Covers: story:11-75
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes its project file before any async worker runs"
)]
fn a_crawls_run_carries_the_project_file_its_connection_read() {
    let env = TestEnv::new();
    let project = env.folder().join("goodfetch.toml");
    std::fs::write(
        &project,
        "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 45000\n",
    )
    .unwrap();
    let explained = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env).project(&project))
            .await
            .unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<Visit>| async { Ok(()) })
            .unwrap();
        let seed = Seed::new("https://shop.test/", &page, Visit { depth: 0 });
        let run = foxhound.start_crawl(&crawl, [seed]).await.unwrap();
        let explained = run
            .run()
            .explain_settings()
            .keys(["timeout.navigation"])
            .await
            .unwrap();
        foxhound.close().await.unwrap();
        explained
    });

    let setting = &explained.settings[0];
    assert_eq!(
        json!([setting.value, setting.layer]),
        json!([45_000, "project_configuration"])
    );
}

/// A seed of another crawl's Label fails the start with `label_unknown`.
#[test]
fn a_seed_of_another_crawls_label_is_refused() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        crawl
            .handle(&page, |_ctx: CrawlContext<Visit>| async { Ok(()) })
            .unwrap();
        let mut other = Crawl::new();
        let elsewhere = other.label::<Visit>("elsewhere").unwrap();

        let seed = Seed::new("https://shop.test/", &elsewhere, Visit { depth: 0 });
        let started = foxhound.start_crawl(&crawl, [seed]).await;
        foxhound.close().await.unwrap();

        let error: Error = started.unwrap_err();
        assert_eq!(error.code(), error_code::LABEL_UNKNOWN, "{error}");
        let details: LabelUnknownDetails = error.details().unwrap();
        assert_eq!(details.label, "elsewhere");
        assert_eq!(details.declared, ["page"]);
    });
}

/// Declaring needs no connection: a second Label of one name, or a second
/// handler, fails at once with `label_duplicate`, and a handler for
/// another crawl's Label with `label_unknown`.
///
/// Covers: story:11-1013, story:11-1015, story:11-1016, error:label_duplicate, error:label_unknown
#[test]
fn a_declaration_mistake_fails_at_once() {
    let _env = TestEnv::new();
    let mut crawl = Crawl::new();
    let page = crawl.label::<Visit>("page").unwrap();
    let again = crawl.label::<Visit>("page").unwrap_err();
    crawl
        .handle(&page, |_ctx: CrawlContext<Visit>| async { Ok(()) })
        .unwrap();
    let twice = crawl
        .handle(&page, |_ctx: CrawlContext<Visit>| async { Ok(()) })
        .unwrap_err();
    let mut other = Crawl::new();
    let elsewhere = other.label::<Visit>("elsewhere").unwrap();
    let foreign = crawl
        .handle(&elsewhere, |_ctx: CrawlContext<Visit>| async { Ok(()) })
        .unwrap_err();

    let codes: Vec<ErrorCode> = [&again, &twice, &foreign].map(Error::code).into();
    assert_eq!(
        codes,
        [
            error_code::LABEL_DUPLICATE,
            error_code::LABEL_DUPLICATE,
            error_code::LABEL_UNKNOWN
        ]
    );
}

/// A shelf's Payload, whose number `schemars` bounds from 1.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
struct Shelf {
    #[schemars(range(min = 1))]
    number: u32,
}

/// A price's Payload, whose amount may be no JSON number.
#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
struct Price {
    amount: f64,
}

/// The `shelf` Label, its Payloads typed by the schema `schemars` derives
/// for [`Shelf`].
fn shelves() -> LabelDeclaration<'static, Shelf> {
    LabelDeclaration::new("shelf").payload(&Schema::derived())
}

/// A book's Payload schema written by hand: its ISBN, a text.
fn isbn() -> Schema<Value> {
    Schema::json(json!({
        "type": "object",
        "properties": { "isbn": { "type": "string" } },
        "required": ["isbn"],
    }))
    .unwrap()
}

/// What a `payload_invalid` names: its Label, each issue's path, and the
/// setting a Payload too long passes.
fn payload_refusal(error: &Error) -> (String, Vec<String>, Option<String>) {
    assert_eq!(error.code(), error_code::PAYLOAD_INVALID, "{error}");
    let details: PayloadInvalidDetails = error.details().unwrap();
    let paths = details.issues.into_iter().map(|issue| issue.path).collect();
    (details.label, paths, details.limit)
}

/// A Label's Payload schema reaches the engine as JSON Schema, the one
/// `schemars` derives for its type or one written by hand (ADR 0082): a
/// seed whose Payload it refuses fails the start with `payload_invalid`,
/// naming each issue's path, and one that is not JSON fails before
/// anything is sent.
///
/// Covers: story:11-1018, error:payload_invalid, operation:crawl.start
#[test]
fn a_seed_whose_payload_its_labels_schema_refuses_fails_the_start_with_payload_invalid() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let mut crawl = Crawl::new();
        let shelf = crawl.label(shelves()).unwrap();
        let book = crawl
            .label(LabelDeclaration::<Value>::new("book").payload(&isbn()))
            .unwrap();
        let price = crawl
            .label(LabelDeclaration::new("price").payload(&Schema::<Price>::derived()))
            .unwrap();
        crawl
            .handle(&shelf, |_ctx: CrawlContext<Shelf>| async { Ok(()) })
            .unwrap();
        crawl
            .handle(&book, |_ctx: CrawlContext<Value>| async { Ok(()) })
            .unwrap();
        crawl
            .handle(&price, |_ctx: CrawlContext<Price>| async { Ok(()) })
            .unwrap();

        let mut refused = Vec::new();
        for seed in [
            Seed::new("https://shop.test/shelf", &shelf, Shelf { number: 0 }),
            Seed::new("https://shop.test/book", &book, json!({ "isbn": 7 })),
            Seed::new(
                "https://shop.test/price",
                &price,
                Price { amount: f64::NAN },
            ),
        ] {
            let started = foxhound.start_crawl(&crawl, [seed]).await;
            refused.push(payload_refusal(&started.unwrap_err()));
        }
        foxhound.close().await.unwrap();

        let expected = [
            ("shelf", "/number"),
            ("book", "/isbn"),
            ("price", "/amount"),
        ]
        .map(|(label, path)| (label.to_owned(), vec![path.to_owned()], None));
        assert_eq!(refused, expected);
    });
}

/// An enqueue whose Payload its Label's schema refuses, or whose JSON is
/// longer than `crawl.payload.max_size`, 64 KiB by default, fails with
/// `payload_invalid`, which the handler reads, naming each issue's path
/// or the setting, and the handler's own attempt goes on.
///
/// Covers: story:11-1018, error:payload_invalid, setting:crawl.payload.max_size, operation:crawl_request.enqueue
#[test]
fn an_enqueue_whose_payload_its_label_refuses_fails_with_payload_invalid_naming_why() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let start = site.url("shop.test", "/book");
    let (seen, mut refusals) = mpsc::unbounded_channel();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let mut crawl = Crawl::new();
        let page = crawl.label::<()>("page").unwrap();
        let shelf = crawl.label(shelves()).unwrap();
        let book = crawl
            .label(LabelDeclaration::<Value>::new("book").payload(&isbn()))
            .unwrap();
        let (to_shelf, to_book) = (shelf.clone(), book.clone());
        crawl
            .handle(&page, move |ctx: CrawlContext<()>| async move {
                let url = ctx.url().to_owned();
                let long = json!({ "isbn": "9".repeat(65 * 1024) });
                let enqueued = [
                    ctx.enqueue(format!("{url}?shelf"), &to_shelf, Shelf { number: 0 })
                        .await,
                    ctx.enqueue(format!("{url}?isbn"), &to_book, json!({ "isbn": 7 }))
                        .await,
                    ctx.enqueue(format!("{url}?long"), &to_book, long).await,
                ];
                for refused in enqueued {
                    seen.send(payload_refusal(&refused.unwrap_err())).unwrap();
                }
                Ok(())
            })
            .unwrap();
        crawl
            .handle(&shelf, |_ctx: CrawlContext<Shelf>| async { Ok(()) })
            .unwrap();
        crawl
            .handle(&book, |_ctx: CrawlContext<Value>| async { Ok(()) })
            .unwrap();

        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&start, &page, ())])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();

        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (1, 0),
            "{requests:?}"
        );
    });
    let mut refused = Vec::new();
    while let Ok(refusal) = refusals.try_recv() {
        refused.push(refusal);
    }
    let expected = [
        ("shelf", Some("/number"), None),
        ("book", Some("/isbn"), None),
        ("book", None, Some("crawl.payload.max_size")),
    ]
    .map(|(label, path, limit)| {
        (
            label.to_owned(),
            path.map(str::to_owned).into_iter().collect(),
            limit.map(str::to_owned),
        )
    });
    assert_eq!(refused, expected);
}

/// A crawl of books whose Payloads `payload` types, and whose handler
/// takes each one.
fn books(payload: Value) -> (Crawl<'static>, Label<Value>) {
    let mut crawl = Crawl::new();
    let book = crawl
        .label(LabelDeclaration::<Value>::new("book").payload(&Schema::json(payload).unwrap()))
        .unwrap();
    crawl
        .handle(&book, |_ctx: CrawlContext<Value>| async { Ok(()) })
        .unwrap();
    (crawl, book)
}

/// A Run whose connection closed before its two books were served resumes
/// under its Run ID only once its pending Payloads are checked against the
/// `book` Label's new schema (ADR 0082): a schema they all fail, one that
/// requires a title, refuses the resume with `payload_schema_changed`,
/// naming the Label, how many of its pending Payloads fail it and how, and
/// one they all fit resumes the Run, which then serves both.
///
/// Covers: story:11-1018, error:payload_schema_changed, operation:crawl.start
#[test]
fn a_resume_after_its_labels_payload_schema_changed_checks_the_pending_payloads() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITE).unwrap();
    let urls = [
        site.url("shop.test", "/book"),
        site.url("shop.test", "/empty"),
    ];
    let changed = RunId::parse("changed").unwrap();
    let isbn = json!({
        "type": "object",
        "properties": { "isbn": { "type": "string" } },
        "required": ["isbn"],
    });
    let mut titled = isbn.clone();
    titled["properties"]["title"] = json!({ "type": "string" });
    let mut required = titled.clone();
    required["required"] = json!(["isbn", "title"]);
    real_time_runtime().block_on(async {
        let start = async |foxhound: &Foxhound, crawl: &Crawl<'_>, book: &Label<Value>| {
            let seeds = urls
                .iter()
                .map(|url| Seed::new(url, book, json!({ "isbn": "0" })));
            foxhound.start_crawl(crawl, seeds).run_id(&changed).await
        };
        let first = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, book) = books(isbn);
        start(&first, &crawl, &book).await.unwrap();
        first.close().await.unwrap();

        let second = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, book) = books(required);
        let refused = start(&second, &crawl, &book).await.unwrap_err();
        let (crawl, book) = books(titled);
        let run = start(&second, &crawl, &book).await.unwrap();
        let report = crawl.serve(&run).await.unwrap();
        second.close().await.unwrap();

        assert_eq!(
            refused.code(),
            error_code::PAYLOAD_SCHEMA_CHANGED,
            "{refused}"
        );
        let details: PayloadSchemaChangedDetails = refused.details().unwrap();
        let paths: Vec<String> = details.issues.into_iter().map(|issue| issue.path).collect();
        assert_eq!(
            (
                details.label.as_str(),
                details.failing,
                details.pending,
                paths
            ),
            ("book", 2, 2, vec![String::new()])
        );
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
        let requests = report.requests.unwrap();
        assert_eq!(
            (requests.succeeded, requests.failed),
            (2, 0),
            "{requests:?}"
        );
    });
}
