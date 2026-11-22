//! The catalog program in Rust (issue 3229), on the linked engine: a crawl
//! of the Proving ground's `catalog` family whose one handler reads its
//! attempt's page with `page.read`, pushes each product the page lists as
//! one Record and follows the page's Next link, then reads the Dataset back
//! with `run.records` for the Proving ground's oracle to judge, as the
//! TypeScript SDK's catalog program does: the `crawl-catalog-program`
//! client conformance case in Rust's mode, `linked`, and the same program
//! without async, which follows the link with `enqueue_links`.

use std::collections::BTreeMap;

use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_foxhound::{
    BoxError, Crawl, CrawlContext, Dataset, FieldLocator, FieldMatch, FieldTransform, Foxhound,
    Label, ListField, PageField, Seed, ValueField, blocking,
};
use goodfetch_protocol::result_document::{OracleAnswer, OracleOutcome};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus, case_logs, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::crawl::options;

/// The Proving ground's seed the catalog crawls read, the TypeScript
/// case's.
const SEED: u64 = 1467;

/// The task the oracle judges the Records of.
const TASK: &str = "catalog/1";

/// One product, as the listing page gives it, the price without its
/// currency sign.
#[derive(Debug, Serialize)]
struct Product {
    id: String,
    name: String,
    price: String,
}

/// A listing page's fields, as the catalog program reads them.
#[derive(Debug, Deserialize)]
struct Listing {
    ids: Vec<Option<String>>,
    names: Vec<Option<String>>,
    prices: Vec<Option<String>>,
    next: Option<String>,
}

impl Listing {
    /// The products the page lists, or an error naming a field the page
    /// did not give in full.
    fn products(self) -> Result<Vec<Product>, BoxError> {
        let ids = texts("ids", self.ids)?;
        let names = texts("names", self.names)?;
        let prices = texts("prices", self.prices)?;
        if names.len() != ids.len() || prices.len() != ids.len() {
            return Err(format!(
                "the page lists {} products but {} names and {} prices",
                ids.len(),
                names.len(),
                prices.len()
            )
            .into());
        }
        Ok(ids
            .into_iter()
            .zip(names)
            .zip(prices)
            .map(|((id, name), price)| Product { id, name, price })
            .collect())
    }
}

/// The texts a list field read, or an error naming the field when the page
/// left one of them out.
fn texts(field: &str, list: Vec<Option<String>>) -> Result<Vec<String>, BoxError> {
    list.into_iter()
        .collect::<Option<Vec<String>>>()
        .ok_or_else(|| format!("the page gave no {field}").into())
}

/// A value field of the element the CSS selector `selector` finds, read
/// as its text.
fn css(selector: &str) -> ValueField {
    ValueField {
        locator: FieldLocator::Css {
            selector: selector.to_owned(),
        },
        fallbacks: Vec::new(),
        attribute: None,
        matched: None,
        transform: FieldTransform::Text,
    }
}

/// The fields of a listing page: each product's ID, name and price, and
/// where its Next link leads.
fn listing_fields() -> BTreeMap<String, PageField> {
    BTreeMap::from([
        (
            "ids".to_owned(),
            PageField::List(ListField {
                list: ValueField {
                    attribute: Some("data-id".to_owned()),
                    ..css("li.product")
                },
            }),
        ),
        (
            "names".to_owned(),
            PageField::List(ListField {
                list: css("li.product a"),
            }),
        ),
        (
            "prices".to_owned(),
            PageField::List(ListField {
                list: ValueField {
                    matched: Some(FieldMatch {
                        pattern: r"([0-9]+\.[0-9]{2})".to_owned(),
                        group: 1,
                    }),
                    ..css("li.product span.price")
                },
            }),
        ),
        (
            "next".to_owned(),
            PageField::Value(ValueField {
                attribute: Some("href".to_owned()),
                ..css("a[rel=next]")
            }),
        ),
    ])
}

/// The catalog program: one Label for the listing pages, whose handler
/// pushes each product into `products` and enqueues the page the Next
/// link leads to.
fn catalog() -> (Crawl<'static>, Label<()>, Dataset<Product>) {
    let mut crawl = Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let products = crawl.dataset::<Product>("products").unwrap();
    let (next, pushed) = (listing.clone(), products.clone());
    crawl
        .handle(&listing, move |ctx: CrawlContext<()>| async move {
            let read = ctx
                .page()?
                .read(listing_fields())
                .typed::<Listing>()
                .await?;
            let following = read.values.next.clone();
            for product in read.values.products()? {
                ctx.push(&pushed, &product).await?;
            }
            if let Some(following) = following {
                ctx.enqueue(following, &next, ()).await?;
            }
            Ok(())
        })
        .unwrap();
    (crawl, listing, products)
}

/// The oracle's answer on `records` as the Records of the catalog task.
fn judged(site: &ProvingGround, records: &[Value]) -> OracleAnswer {
    let answer = site
        .judge_records(TASK, ToolStatus::Success, records)
        .unwrap();
    serde_json::from_value(answer).unwrap()
}

/// The `crawl-catalog-program` client conformance case, linked: the
/// catalog program crawls the `catalog` family with one Label, one
/// handler, one Dataset and one seed, its handler reading its attempt's
/// page, and the oracle accepts the Records `run.records` reads back.
/// Covers: story:11-1013, story:11-1015
#[test]
fn the_catalog_program_crawls_the_catalog_and_the_oracle_accepts_its_records() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "crawl-catalog-program",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let (report, records) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env, &site)).await.unwrap();
        let (crawl, listing, products) = catalog();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&start, &listing, ())])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        let mut records = Vec::new();
        let mut read = run.records(&products);
        while let Some(record) = read.next().await {
            records.push(record.unwrap().value);
        }
        foxhound.close().await.unwrap();
        (report, records)
    });
    let answer = judged(&site, &records);
    case.oracle_answer(answer.clone());

    case.check("finish() returns the finished Run's report", || {
        assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    });
    case.check("the oracle accepts the Records", || {
        assert_eq!(answer.outcome, OracleOutcome::Verified, "{answer:?}");
    });
}

/// The catalog program without async: its handler reads the blocking page
/// and follows the Next link with `enqueue_links`, which enqueues the one
/// link the page has, and the blocking `run.records` reads the Records
/// back, which the oracle accepts.
/// Covers: story:11-1013, story:11-1015, story:11-1104
#[test]
fn the_blocking_catalog_program_follows_its_links_and_reads_its_records_back() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let foxhound = blocking::Foxhound::connect(options(&env, &site)).unwrap();
    let mut crawl = blocking::Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let products = crawl.dataset::<Product>("products").unwrap();
    let (next, pushed) = (listing.clone(), products.clone());
    crawl
        .handle(&listing, move |ctx: blocking::CrawlContext<()>| {
            let page = ctx.page()?;
            let read = page.read(listing_fields()).typed::<Listing>().send()?;
            let last = read.values.next.is_none();
            for product in read.values.products()? {
                ctx.push(&pushed, &product)?;
            }
            let enqueued = ctx.enqueue_links(&page.locator("a[rel=next]"), &next, ())?;
            let expected = usize::from(!last);
            if enqueued.links.len() != expected {
                return Err(format!(
                    "enqueue_links answered {} links where the page has {expected}",
                    enqueued.links.len()
                )
                .into());
            }
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&start, &listing, ())])
        .send()
        .unwrap();
    let report = crawl.serve(&run).unwrap();
    let records: Vec<Value> = run
        .records(&products)
        .map(|record| record.unwrap().value)
        .collect();
    foxhound.close().unwrap();

    assert_eq!(report.report.lifecycle, Lifecycle::Finished);
    let requests = report.requests.unwrap();
    assert_eq!(requests.failed, 0, "{requests:?}");
    let answer = judged(&site, &records);
    assert_eq!(answer.outcome, OracleOutcome::Verified, "{answer:?}");
}
