//! A handler's enqueues in Rust (ADRs 0093, 0099 and 0120), on the linked
//! engine: `ctx.enqueue` with a method and a body, a priority, the
//! forefront and a key of the program's own, each a builder method named
//! after its option, and `ctx.enqueue_links` with one Payload for every
//! link or a `LinkPayload::with` function of each, ADR 0099's one name for
//! both forms; a relative `enqueue` resolves against its document's base
//! URL. The TypeScript and Python twins are `link-payload.test.ts`,
//! `enqueue-base.test.ts` and `test_link_payload_attached.py` of Foxhound's
//! packages.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the handlers note what they saw on the test's own side, which loom does not model (ADR 0060)"
)]

use std::sync::{Arc, Mutex};

use goodfetch_foxhound::{
    Crawl, CrawlContext, EnqueuedLinks, Foxhound, HttpMethod, LinkPayload, MatchedLink, Seed,
    blocking,
};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::crawl::options;
use super::seeds::{CATALOG, CATALOG_HOST, Ordered, ordered_field};
use crate::fetch_tiers::{ORDER_PATH, ORDERED, order_methods};

/// The Proving ground's seed of the `links` family.
const LINKS: u64 = 863;

/// The `links` family's host.
const LINKS_HOST: &str = "links.test";

/// A linked page's Payload, which the Payload function answers from the
/// link that leads to it.
#[derive(Debug, Serialize, Deserialize)]
struct Linked {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    isbn: Option<String>,
}

/// What a handler noted, shared with the test's thread.
type Noted<T> = Arc<Mutex<Vec<T>>>;

/// `noted` with `value`.
fn note<T>(noted: &Noted<T>, value: T) {
    noted
        .lock()
        .expect("invariant: no handler panics holding the lock")
        .push(value);
}

/// What `noted` holds.
fn taken<T: Clone>(noted: &Noted<T>) -> Vec<T> {
    noted
        .lock()
        .expect("invariant: no handler panics holding the lock")
        .clone()
}

/// `enqueue_links` with a `LinkPayload::with` function reads each element
/// of the `links` family's answers page, as `crawl_request.links` answers
/// it, and gives each link the Payload the function answers: a link with
/// a `data-isbn` takes it, the link the function answers `None` for is
/// `skipped_by_payload`, and the others take their text, as the Frontier
/// judges each.
///
/// Covers: story:11-1021, operation:crawl_request.enqueue_links
#[test]
fn enqueue_links_gives_each_link_the_payload_its_function_answers() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, LINKS).unwrap();
    let url = |path: &str| site.url(LINKS_HOST, path);
    let read: Noted<MatchedLink> = Arc::default();
    let answered: Noted<EnqueuedLinks> = Arc::default();
    let given: Noted<(String, Value)> = Arc::default();
    let mut crawl = Crawl::new();
    let index = crawl.label::<()>("index").unwrap();
    let page = crawl.label::<Linked>("page").unwrap();
    let (reads, answers, to_page) = (Arc::clone(&read), Arc::clone(&answered), page.clone());
    crawl
        .handle(&index, move |ctx: CrawlContext<()>| async move {
            let reading = Arc::clone(&reads);
            let links = ctx.page()?.locator("ul.links a");
            let enqueued = ctx
                .enqueue_links(
                    &links,
                    &to_page,
                    LinkPayload::with(move |link: &MatchedLink| {
                        note(&reading, link.clone());
                        if let Some(isbn) = link.attributes.get("data-isbn") {
                            return Some(Linked {
                                from: None,
                                isbn: Some(isbn.clone()),
                            });
                        }
                        (link.href.as_deref() != Some("2")).then(|| Linked {
                            from: Some(link.text.clone()),
                            isbn: None,
                        })
                    }),
                )
                .await?;
            note(&answers, enqueued);
            Ok(())
        })
        .unwrap();
    let on_page = Arc::clone(&given);
    crawl
        .handle(&page, move |ctx: CrawlContext<Linked>| async move {
            let payload = json!({ "from": ctx.payload().from, "isbn": ctx.payload().isbn });
            note(&on_page, (ctx.url().to_owned(), payload));
            Ok(())
        })
        .unwrap();

    let report = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(url("/answers/"), &index, ())])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();
        report
    });

    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let texts: Vec<String> = taken(&read).into_iter().map(|link| link.text).collect();
    let answered: Vec<Value> = taken(&answered)
        .iter()
        .map(|links| serde_json::to_value(&links.links).unwrap())
        .collect();
    let mut given = taken(&given);
    given.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        (texts, answered, given),
        (
            [
                "One",
                "One again",
                "No link",
                "Mail",
                "Elsewhere",
                "Broken",
                "Two",
                "Three"
            ]
            .map(str::to_owned)
            .to_vec(),
            vec![json!([
                { "kind": "added", "id": "2", "url": url("/answers/1") },
                { "kind": "not_added", "reason": "duplicate", "url": "/answers/1#top" },
                { "kind": "no_link" },
                { "kind": "not_added", "reason": "unsupported_scheme", "url": "mailto:shop@links.test" },
                { "kind": "not_added", "reason": "out_of_scope", "url": "http://elsewhere.test/" },
                { "kind": "not_added", "reason": "invalid_url", "url": "http://[::1/" },
                { "kind": "skipped_by_payload", "url": "2" },
                { "kind": "added", "id": "3", "url": url("/answers/3") },
            ])],
            vec![
                (url("/answers/1"), json!({ "from": "One", "isbn": null })),
                (url("/answers/3"), json!({ "from": null, "isbn": "978-3" })),
            ],
        )
    );
}

/// A relative URL given to `enqueue` resolves against the base URL of the
/// document its handler was given: on `links/1`'s shelf, against its
/// `<base href="/library/shelves/">`; on `links/2`'s, against the URL
/// `/old/shelf` moved to, `/new/shelf/`. Neither request reaches a wrong
/// base's `404`.
///
/// Covers: story:11-1022, operation:crawl_request.enqueue
#[test]
fn a_relative_enqueue_resolves_against_its_documents_base_url() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, LINKS).unwrap();
    let enqueued: Noted<Value> = Arc::default();
    let handled: Noted<String> = Arc::default();
    let mut crawl = Crawl::new();
    let shelf = crawl.label::<()>("shelf").unwrap();
    let book = crawl.label::<()>("book").unwrap();
    let (to_book, answers) = (book.clone(), Arc::clone(&enqueued));
    crawl
        .handle(&shelf, move |ctx: CrawlContext<()>| async move {
            let added = ctx.enqueue("book/1", &to_book, ()).await?;
            note(&answers, serde_json::to_value(added)?);
            Ok(())
        })
        .unwrap();
    let on_book = Arc::clone(&handled);
    crawl
        .handle(&book, move |ctx: CrawlContext<()>| async move {
            note(&on_book, ctx.url().to_owned());
            Ok(())
        })
        .unwrap();
    let seeds = [
        Seed::new(site.url("1.links.test", "/guide/"), &shelf, ()),
        Seed::new(site.url("2.links.test", "/old/shelf"), &shelf, ()),
    ];

    let report = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound.start_crawl(&crawl, seeds).await.unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();
        report
    });

    let books = [
        site.url("1.links.test", "/library/shelves/book/1"),
        site.url("2.links.test", "/new/shelf/book/1"),
    ];
    let mut answered: Vec<Value> = taken(&enqueued)
        .into_iter()
        .map(|answer| answer["url"].clone())
        .collect();
    answered.sort_by_key(Value::to_string);
    let mut handled = taken(&handled);
    handled.sort();
    assert_eq!(
        (
            report.report.lifecycle,
            answered,
            handled,
            site.observations().unwrap()["findings"].clone()
        ),
        (
            Lifecycle::Finished,
            books.iter().map(|url| json!(url)).collect(),
            books.to_vec(),
            json!([])
        )
    );
}

/// A blocking handler's `enqueue` takes each request option as a builder
/// method named after it, which `send()` sends: a `POST` with a form body,
/// at the forefront and of a priority, reaches the site as that `POST`,
/// whose order page names the ordered product, and two URLs enqueued under
/// one `key` are one Crawl request, the second answered `duplicate`.
///
/// Covers: story:11-1020, story:11-1023, operation:crawl_request.enqueue
#[test]
fn a_blocking_enqueue_sends_its_method_body_and_key() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, CATALOG).unwrap();
    let url = |path: &str| site.url(CATALOG_HOST, path);
    let enqueued: Noted<Value> = Arc::default();
    let handled: Noted<(String, String, Value)> = Arc::default();
    let foxhound = blocking::Foxhound::connect(options(&env, &site)).unwrap();
    let mut crawl = blocking::Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let shelf = crawl.label::<()>("shelf").unwrap();
    let order = crawl.label::<()>("order").unwrap();
    let (to_order, to_shelf, answers) = (order.clone(), shelf.clone(), Arc::clone(&enqueued));
    let (order_url, shelves) = (url(ORDER_PATH), [url("/catalog/2"), url("/catalog/3")]);
    crawl
        .handle(&listing, move |ctx: blocking::CrawlContext<()>| {
            let ordered = ctx
                .enqueue(order_url.clone(), &to_order, ())
                .method(HttpMethod::parse("POST")?)
                .body(format!("product={ORDERED}"))
                .priority(1)
                .forefront()
                .send()?;
            note(&answers, serde_json::to_value(ordered)?);
            for shelf in &shelves {
                let keyed = ctx
                    .enqueue(shelf.clone(), &to_shelf, ())
                    .key("the-shelf")
                    .send()?;
                note(&answers, serde_json::to_value(keyed)?);
            }
            Ok(())
        })
        .unwrap();
    let on_shelf = Arc::clone(&handled);
    crawl
        .handle(&shelf, move |ctx: blocking::CrawlContext<()>| {
            note(
                &on_shelf,
                (ctx.label().to_owned(), ctx.url().to_owned(), Value::Null),
            );
            Ok(())
        })
        .unwrap();
    let on_order = Arc::clone(&handled);
    crawl
        .handle(&order, move |ctx: blocking::CrawlContext<()>| {
            let read = ctx
                .page()?
                .read(ordered_field())
                .typed::<Ordered>()
                .send()?;
            note(
                &on_order,
                (
                    ctx.label().to_owned(),
                    ctx.url().to_owned(),
                    json!(read.values.id),
                ),
            );
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(url("/catalog/1"), &listing, ())])
        .send()
        .unwrap();
    let report = crawl.serve(&run).unwrap();
    foxhound.close().unwrap();

    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let mut handled = taken(&handled);
    handled.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    assert_eq!(
        (taken(&enqueued), handled, order_methods(&site)),
        (
            vec![
                json!({ "kind": "added", "id": "2", "url": url(ORDER_PATH) }),
                json!({ "kind": "added", "id": "3", "url": url("/catalog/2") }),
                json!({ "kind": "not_added", "reason": "duplicate", "url": url("/catalog/3") }),
            ],
            vec![
                ("order".to_owned(), url(ORDER_PATH), json!(ORDERED)),
                ("shelf".to_owned(), url("/catalog/2"), Value::Null),
            ],
            vec!["POST".to_owned()],
        )
    );
}
