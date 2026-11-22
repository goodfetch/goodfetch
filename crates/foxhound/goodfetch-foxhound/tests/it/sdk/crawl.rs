//! A crawl in Rust on the linked engine (ADRs 0093, 0109 and 0112): the
//! program declares a [`Crawl`] with no connection, starts its Run with
//! `start_crawl` and serves it with `crawl.serve(&run)`, whose handlers run
//! inside that future, until the Run ends and its report returns.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::{Settings, egress, fetch, network};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, Error, ErrorCode, Foxhound, LabelDeclaration, RunId,
    Schema, Seed, error_code,
};
use goodfetch_protocol::foxhound::crawl::LabelUnknownDetails;
use goodfetch_protocol::foxhound::frontier::PayloadInvalidDetails;
use goodfetch_protocol::foxhound::report::CountOf;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc};

/// The Proving ground's seed the crawls read.
const SITE: u64 = 1602;

/// A page's Payload: how far from the seed it is.
#[derive(Clone, Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
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
fn options(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
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

/// Covers: story:11-1013, story:11-1014, story:11-1015
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
        let mut crawl = Crawl::new();
        let page = crawl.label::<Visit>("page").unwrap();
        let pages = crawl.dataset::<Visited>("pages").unwrap();
        let next = (page.clone(), empty.clone());
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

/// Covers: story:11-1025
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
/// Covers: error:retry_requested
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
/// nothing, `'static`: a spawned task serves the Run.
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
        crawl
            .handle(&page, |_ctx: CrawlContext<Visit>| async { Ok(()) })
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
        let report = served.await.unwrap().unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.requests.unwrap().succeeded, 1);
    });
}

/// A Label without a handler fails the start, before anything is sent.
///
/// Covers: story:11-1015, error:label_unhandled
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
/// Covers: story:11-1013, story:11-1015, error:label_duplicate, error:label_unknown
#[test]
fn a_declaration_mistake_fails_at_once() {
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
        let shelf = crawl.label::<Shelf>("shelf").unwrap();
        let book = crawl
            .label(LabelDeclaration::<Value>::new("book").payload(isbn()))
            .unwrap();
        let price = crawl.label::<Price>("price").unwrap();
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
        let shelf = crawl.label::<Shelf>("shelf").unwrap();
        let book = crawl
            .label(LabelDeclaration::<Value>::new("book").payload(isbn()))
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
