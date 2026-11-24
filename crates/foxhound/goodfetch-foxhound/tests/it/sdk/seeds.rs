//! A crawl's seeds in Rust (ADRs 0093 and 0120), on the linked engine:
//! `Seed::new` with or without a Payload, sent with a method and a body
//! or identified by a key of the program's own, and `Seed::sitemap`,
//! whose rules give each entry the Label of the first that matches it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the handlers note what they saw on the test's own side, which loom does not model (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use goodfetch_foxhound::{
    Crawl, CrawlContext, Error, FieldLocator, FieldTransform, Foxhound, HttpMethod, Label,
    PageField, Seed, ValueField, error_code,
};
use goodfetch_protocol::foxhound::crawl::LabelUnknownDetails;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::crawl::options;
use crate::fetch_tiers::{ORDER_PATH, ORDERED, order_methods};

/// The Proving ground's seed of the `catalog` family the request seeds
/// read, the catalog program's.
const CATALOG: u64 = 1467;

/// The `catalog` family's host.
const CATALOG_HOST: &str = "catalog.test";

/// The Proving ground's seed of the `sitemap-seed` family.
const SITEMAPS: u64 = 903;

/// The `sitemap-seed` family's host.
const SITEMAP_HOST: &str = "sitemap-seed.test";

/// A product seed's Payload: the listing page it is listed on.
#[derive(Debug, Serialize, Deserialize)]
struct Listed {
    page: u32,
}

/// What the handlers saw, each request's Label, URL and what its handler
/// read of it, in the order they ran.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// `seen` with what `ctx`'s handler read of its request.
fn see<P>(seen: &Seen, ctx: &CrawlContext<P>, read: Value) {
    seen.lock()
        .expect("invariant: no handler panics holding the lock")
        .push((ctx.label().to_owned(), ctx.url().to_owned(), read));
}

/// What the handlers saw, sorted.
fn sorted(seen: &Seen) -> Vec<(String, String, Value)> {
    let mut seen = seen
        .lock()
        .expect("invariant: no handler panics holding the lock")
        .clone();
    seen.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    seen
}

/// What the order page names: the product the order's body ordered.
#[derive(Debug, Deserialize)]
struct Ordered {
    id: Option<String>,
}

/// The text of the order page's `span.id`, read as [`Ordered`].
fn ordered_field() -> BTreeMap<String, PageField> {
    BTreeMap::from([(
        "id".to_owned(),
        PageField::Value(ValueField {
            locator: FieldLocator::Css {
                selector: "span.id".to_owned(),
            },
            fallbacks: Vec::new(),
            attribute: None,
            matched: None,
            transform: FieldTransform::Text,
        }),
    )])
}

/// A seed without a Payload, one with a Payload and a `POST` with a form
/// body all start one crawl of the `catalog` family: each handler is
/// given its seed, with its Payload, and the `POST` reaches the site as
/// that `POST` with that body, whose order page names the ordered
/// product.
///
/// Covers: story:11-1019, operation:crawl.start
#[test]
fn seeds_with_and_without_a_payload_and_a_post_with_a_body_start_one_crawl() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, CATALOG).unwrap();
    let url = |path: &str| site.url(CATALOG_HOST, path);
    let seen: Seen = Arc::default();
    let mut crawl = Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let product = crawl.label::<Listed>("product").unwrap();
    let order = crawl.label::<()>("order").unwrap();
    let (on_listing, on_product, on_order) =
        (Arc::clone(&seen), Arc::clone(&seen), Arc::clone(&seen));
    crawl
        .handle(&listing, move |ctx: CrawlContext<()>| async move {
            see(&on_listing, &ctx, Value::Null);
            Ok(())
        })
        .unwrap();
    crawl
        .handle(&product, move |ctx: CrawlContext<Listed>| async move {
            see(&on_product, &ctx, json!(ctx.payload().page));
            Ok(())
        })
        .unwrap();
    crawl
        .handle(&order, move |ctx: CrawlContext<()>| async move {
            let read = ctx.page()?.read(ordered_field()).typed::<Ordered>().await?;
            see(&on_order, &ctx, json!(read.values.id));
            Ok(())
        })
        .unwrap();

    let report = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(
                &crawl,
                [
                    Seed::new(url("/catalog/1"), &listing, ()),
                    Seed::new(url("/products/P-0002"), &product, Listed { page: 1 }),
                    Seed::new(url(ORDER_PATH), &order, ())
                        .method(HttpMethod::parse("post").unwrap())
                        .body(format!("product={ORDERED}")),
                ],
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();
        report
    });

    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let requests = report.requests.unwrap();
    assert_eq!(
        (
            (requests.succeeded, requests.failed),
            sorted(&seen),
            order_methods(&site)
        ),
        (
            (3, 0),
            vec![
                ("listing".to_owned(), url("/catalog/1"), Value::Null),
                ("order".to_owned(), url(ORDER_PATH), json!(ORDERED)),
                ("product".to_owned(), url("/products/P-0002"), json!(1)),
            ],
            vec!["POST".to_owned()],
        )
    );
}

/// Two seeds of two URLs given one `key` are one Crawl request: the
/// program's key replaces the Frontier's identity of each, so the second
/// is a duplicate of the first and only the first is handled.
///
/// Covers: story:11-1023, operation:crawl.start
#[test]
fn two_seeds_of_one_key_are_one_crawl_request() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, CATALOG).unwrap();
    let url = |path: &str| site.url(CATALOG_HOST, path);
    let seen: Seen = Arc::default();
    let mut crawl = Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let on_listing = Arc::clone(&seen);
    crawl
        .handle(&listing, move |ctx: CrawlContext<()>| async move {
            see(&on_listing, &ctx, Value::Null);
            Ok(())
        })
        .unwrap();

    let report = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(
                &crawl,
                [
                    Seed::new(url("/catalog/1"), &listing, ()).key("the-catalog"),
                    Seed::new(url("/catalog/2"), &listing, ()).key("the-catalog"),
                ],
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();
        report
    });

    let requests = report.requests.unwrap();
    assert_eq!(
        ((requests.succeeded, requests.failed), sorted(&seen)),
        (
            (1, 0),
            vec![("listing".to_owned(), url("/catalog/1"), Value::Null)]
        )
    );
}

/// A sitemap index's two sitemaps seed their entries, each handled by the
/// Label of the first rule whose pattern matches its URL: a book matches
/// both `book`'s rule and the later `shelf`'s, and is a `book`.
///
/// Covers: story:11-1019, operation:crawl.start
#[test]
fn a_sitemap_seed_routes_each_entry_to_the_label_of_its_first_matching_rule() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SITEMAPS).unwrap();
    let url = |path: &str| site.url(SITEMAP_HOST, path);
    let seen: Seen = Arc::default();
    let mut crawl = Crawl::new();
    let mut labels = Vec::new();
    for name in ["book", "shelf", "author"] {
        let label = crawl.label::<()>(name).unwrap();
        let on_label = Arc::clone(&seen);
        crawl
            .handle(&label, move |ctx: CrawlContext<()>| async move {
                see(&on_label, &ctx, Value::Null);
                Ok(())
            })
            .unwrap();
        labels.push(label);
    }
    let [book, shelf, author] = labels.as_slice() else {
        unreachable!("three Labels are declared");
    };

    let report = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(
                &crawl,
                [Seed::sitemap(
                    url("/sitemap-index.xml"),
                    [
                        ("**/books/*", book),
                        ("**/books/**", shelf),
                        ("**/authors/*", author),
                    ],
                )],
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();
        report
    });

    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let handled = |label: &str, path: &str| (label.to_owned(), url(path), Value::Null);
    assert_eq!(
        sorted(&seen),
        vec![
            handled("author", "/authors/1"),
            handled("author", "/authors/2"),
            handled("book", "/books/1"),
            handled("book", "/books/2"),
            handled("book", "/books/3"),
        ]
    );
}

/// A crawl with one handled Label, `page`, and the `page` Label.
fn one_label() -> (Crawl<'static>, Label<()>) {
    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    crawl
        .handle(&page, |_ctx: CrawlContext<()>| async { Ok(()) })
        .unwrap();
    (crawl, page)
}

/// A sitemap seed takes no request option, and its rules only the
/// crawl's own Labels: a method given to one fails the start with
/// `invalid_params`, and a rule of another crawl's Label with
/// `label_unknown`, before anything is sent.
///
/// Covers: story:11-1019, error:label_unknown
#[test]
fn a_sitemap_seed_given_a_request_option_or_another_crawls_label_is_refused() {
    let env = TestEnv::new();
    let (crawl, page) = one_label();
    let elsewhere = Crawl::new().label::<()>("elsewhere").unwrap();
    let sitemap = "https://shop.test/sitemap.xml";

    let post = HttpMethod::parse("POST").unwrap();

    let (with_a_method, of_another_crawl): (Error, Error) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env)).await.unwrap();
        let with_a_method = foxhound
            .start_crawl(
                &crawl,
                [Seed::sitemap(sitemap, [("**", &page)]).method(post)],
            )
            .await
            .unwrap_err();
        let of_another_crawl = foxhound
            .start_crawl(&crawl, [Seed::sitemap(sitemap, [("**", &elsewhere)])])
            .await
            .unwrap_err();
        foxhound.close().await.unwrap();
        (with_a_method, of_another_crawl)
    });

    let details: LabelUnknownDetails = of_another_crawl.details().unwrap();
    assert_eq!(
        (
            with_a_method.code().as_str(),
            of_another_crawl.code(),
            details.label.as_str(),
            details.declared
        ),
        (
            "invalid_params",
            error_code::LABEL_UNKNOWN,
            "elsewhere",
            vec!["page".to_owned()]
        ),
        "{with_a_method}"
    );
}
