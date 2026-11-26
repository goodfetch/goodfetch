//! A handler's `push` in Rust on the linked engine (ADR 0103; issue
//! 1486): crawling the Proving ground's `catalog` family, each Record is
//! checked in the program before it is sent. One that is not JSON, or
//! that its Dataset's schema does not read back as given, is never
//! staged, and `record_invalid` names it as the engine names one it
//! refuses. Caught, the attempt keeps the others; escaped, the request
//! fails without a retry. TypeScript's `datasets.test.ts` and Python's
//! `test_datasets_attached.py` of Foxhound's packages are its twins.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use futures_util::TryStreamExt as _;
use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::crawl::{handler, retry};
use goodfetch_foxhound::{
    Crawl, CrawlContext, CrawlRun, Dataset, DatasetDeclaration, Error, Foxhound, Label, Schema,
    Seed, error_code,
};
use goodfetch_protocol::foxhound::dataset::RecordInvalidDetails;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::crawl::{count, site_settings};

/// The Proving ground's seed for these tests, TypeScript's.
const SEED: u64 = 1486;

/// The task the crawl starts at, the catalog's first listing.
const TASK: &str = "catalog/1";

/// A price with two decimals, which its `Deserialize` checks: one built
/// in the program without that check does not read back.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(transparent)]
struct Price(String);

impl<'de> Deserialize<'de> for Price {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let price = String::deserialize(deserializer)?;
        let (whole, cents) = price.split_once('.').unwrap_or((&price, ""));
        let digits =
            |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
        if digits(whole) && cents.len() == 2 && digits(cents) {
            Ok(Self(price))
        } else {
            Err(serde::de::Error::custom("the price has no two decimals"))
        }
    }
}

/// One product.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Product {
    id: String,
    name: String,
    price: Price,
}

/// The product `id`, named `name`, at `price`.
fn product(id: &str, name: &str, price: &str) -> Product {
    Product {
        id: id.to_owned(),
        name: name.to_owned(),
        price: Price(price.to_owned()),
    }
}

/// A note of a Dataset without a schema, keyed by its ID, which it may
/// lack.
#[derive(Debug, Serialize)]
struct Note {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    text: f64,
}

/// What a `record_invalid` names: each refused Record's position and the
/// paths of its issues.
type Refused = Vec<(u64, Vec<String>)>;

/// What `error`, a push's, refuses.
fn refused(error: &Error) -> Refused {
    let details: RecordInvalidDetails = error.details().expect("a record_invalid's details");
    details
        .records
        .into_iter()
        .map(|record| {
            let paths = record.issues.into_iter().map(|issue| issue.path).collect();
            (record.position, paths)
        })
        .collect()
}

/// The Records `run` committed into its Dataset `name`, each as its JSON,
/// read through a reader's own Dataset of that name.
async fn committed(run: &CrawlRun, name: &str) -> Vec<Value> {
    let json = Crawl::new().dataset::<Value>(name).unwrap();
    let mut values = Vec::new();
    let mut read = run.records(&json);
    while let Some(record) = read.next().await {
        values.push(record.unwrap().value);
    }
    values
}

/// What each push of three products and three notes refuses, caught, by
/// the Record's own name.
async fn push_caught(
    ctx: &CrawlContext<()>,
    products: &Dataset<Product>,
    notes: &Dataset<Note>,
) -> Vec<(&'static str, Refused)> {
    let mut failures = Vec::new();
    for (id, name, price) in [
        ("P-1", "Blue kettle", "12.50"),
        ("P-2", "Red kettle", "free"),
        ("P-3", "Green kettle", "9.99"),
    ] {
        if let Err(error) = ctx.push(products, &product(id, name, price)).await {
            failures.push((id, refused(&error)));
        }
    }
    for (name, id, text) in [
        ("N-1", Some("N-1".to_owned()), 1.5),
        ("N-2", Some("N-2".to_owned()), f64::NAN),
        ("N-3", None, 3.0),
    ] {
        if let Err(error) = ctx.push(notes, &Note { id, text }).await {
            failures.push((name, refused(&error)));
        }
    }
    failures
}

/// All that `channel` has received.
fn drained<T>(channel: &mut UnboundedReceiver<T>) -> Vec<T> {
    let mut received = Vec::new();
    while let Ok(value) = channel.try_recv() {
        received.push(value);
    }
    received
}

/// Caught, a push keeps the Records that hold; escaped, it is not retried.
/// A product whose price has no two decimals does not read back through
/// its schema, and a note holds NaN: the program refuses both. The engine
/// refuses the note without its key.
///
/// Covers: story:11-1058, story:11-1059, error:record_invalid
#[test]
fn a_record_is_refused_before_it_is_sent_and_fails_its_request_when_it_escapes() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let mut crawl = Crawl::new();
    let caught = crawl.label::<()>("caught").unwrap();
    let escaped = crawl.label::<()>("escaped").unwrap();
    let products = crawl
        .dataset(
            DatasetDeclaration::<Product>::new("products")
                .schema(&Schema::derived())
                .key(&["id"]),
        )
        .unwrap();
    let notes = crawl
        .dataset(DatasetDeclaration::<Note>::new("notes").key(&["id"]))
        .unwrap();
    let (failed, mut failures) = mpsc::unbounded_channel();
    let (started, mut escaped_attempts) = mpsc::unbounded_channel();
    let (shelf, memos) = (products.clone(), notes.clone());
    crawl
        .handle(&caught, move |ctx: CrawlContext<()>| async move {
            for failure in push_caught(&ctx, &shelf, &memos).await {
                failed.send(failure).unwrap();
            }
            Ok(())
        })
        .unwrap();
    let lids = products.clone();
    crawl
        .handle(&escaped, move |ctx: CrawlContext<()>| async move {
            started.send(ctx.attempt()).unwrap();
            ctx.push(&lids, &product("P-4", "Kettle lid", "1.5"))
                .await?;
            Ok(())
        })
        .unwrap();
    let settings = site_settings(&site)
        .set(handler::CONCURRENCY, 1_u64)
        .set(retry::HANDLER_FAILED, true);

    let (report, committed) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(super::options(&env).settings(settings))
            .await
            .unwrap();
        let run = foxhound
            .start_crawl(
                &crawl,
                [
                    Seed::new(&start, &caught, ()),
                    Seed::new(&start, &escaped, ()).key("escaped"),
                ],
            )
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        let committed = (
            committed(&run, "products").await,
            committed(&run, "notes").await,
        );
        foxhound.close().await.unwrap();
        (report, committed)
    });

    assert_eq!(
        drained(&mut failures),
        [
            ("P-2", vec![(0, vec![String::new()])]),
            ("N-2", vec![(0, vec!["/text".to_owned()])]),
            ("N-3", vec![(0, vec!["/id".to_owned()])]),
        ]
    );
    assert_eq!(
        committed,
        (
            vec![
                json!({"id": "P-1", "name": "Blue kettle", "price": "12.50"}),
                json!({"id": "P-3", "name": "Green kettle", "price": "9.99"}),
            ],
            vec![json!({"id": "N-1", "text": 1.5})],
        )
    );
    let requests = report.requests.unwrap();
    assert_eq!(
        (
            drained(&mut escaped_attempts),
            requests.failed,
            count(&requests.by_error_code, error_code::RECORD_INVALID.as_str()),
        ),
        (vec![1], 1, 1),
        "{requests:?}"
    );
}

/// A product as a reader reads it, whose price must be a string, with
/// every field it does not declare kept.
#[derive(Debug, PartialEq, Deserialize)]
struct StrictProduct {
    id: String,
    name: String,
    price: String,
    #[serde(flatten)]
    other: serde_json::Map<String, Value>,
}

/// A crawl whose listing handler pushes three kettles keyed by `id`, the
/// second with a number for its price, and a fourth without its key.
fn kettle_shelf() -> (Crawl<'static>, Label<()>, Dataset<Value>) {
    let mut crawl = Crawl::new();
    let listing = crawl.label::<()>("listing").unwrap();
    let products = crawl
        .dataset(DatasetDeclaration::<Value>::new("products").key(&["id"]))
        .unwrap();
    let shelf = products.clone();
    crawl
        .handle(&listing, move |ctx: CrawlContext<()>| async move {
            for record in [
                json!({"id": "P-1", "name": "Blue kettle", "price": "12.50"}),
                json!({"id": "P-2", "name": "Red kettle", "price": 9.99}),
                json!({"id": "P-3", "name": "Green kettle", "price": "3.00", "colour": "green"}),
                json!({"name": "Nameless kettle", "price": "1.00"}),
            ] {
                match ctx.push(&shelf, &record).await {
                    Err(error) if error.code() != error_code::RECORD_INVALID => {
                        return Err(error.into());
                    }
                    _ => {}
                }
            }
            Ok(())
        })
        .unwrap();
    (crawl, listing, products)
}

/// A typed read stops at the Record its type refuses, and resumes past
/// it. The engine refuses the Record without its key into the rejection
/// table, which reads with its reason, issue, request and URL.
///
/// Covers: story:11-1060, story:11-1061, story:11-1070, error:record_invalid
#[test]
fn reads_records_as_a_readers_type_past_after_and_the_rejection_table() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let (crawl, listing, products) = kettle_shelf();
    // A reader's own declaration of the Dataset, whose type wants a string
    // price.
    let typed = Crawl::new().dataset::<StrictProduct>("products").unwrap();

    let (numbers, stopped, resumed, rejected, past_rejected) =
        real_time_runtime().block_on(async {
            let foxhound = Foxhound::connect(super::options(&env).settings(site_settings(&site)))
                .await
                .unwrap();
            let run = foxhound
                .start_crawl(&crawl, [Seed::new(&start, &listing, ())])
                .await
                .unwrap();
            crawl.serve(&run).await.unwrap();
            let read: Vec<_> = run.records(&products).try_collect().await.unwrap();
            let numbers: Vec<_> = read
                .into_iter()
                .map(|record| (record.value["id"].clone(), record.number))
                .collect();
            let mut refusing = run.records(&typed).after(numbers[0].1);
            let stopped = (
                refusing
                    .next()
                    .await
                    .map(|read| read.map(|record| record.number)),
                refusing.next().await.is_none(),
            );
            let resumed: Vec<_> = run
                .records(&typed)
                .after(numbers[1].1)
                .try_collect()
                .await
                .unwrap();
            let resumed: Vec<_> = resumed.into_iter().map(|record| record.value).collect();
            let rejected: Vec<_> = run.rejected_records(&products).try_collect().await.unwrap();
            let mut read = run.rejected_records(&products).after(1);
            let past_rejected = read
                .next()
                .await
                .map(|entry| entry.map(|entry| entry.number));
            foxhound.close().await.unwrap();
            (numbers, stopped, resumed, rejected, past_rejected)
        });

    assert_eq!(
        numbers,
        [(json!("P-1"), 1), (json!("P-2"), 2), (json!("P-3"), 3)]
    );
    let (Some(Err(error)), true) = stopped else {
        panic!("the typed read did not stop at P-2: {stopped:?}");
    };
    assert_eq!(error.code(), error_code::RECORD_INVALID, "{error:?}");
    let details: RecordInvalidDetails = error.details().expect("a record_invalid's details");
    assert_eq!(
        (
            details.dataset.as_str(),
            details.number,
            details
                .records
                .iter()
                .map(|record| record.position)
                .collect::<Vec<_>>(),
        ),
        ("products", Some(2), vec![0])
    );
    assert_eq!(
        resumed,
        [StrictProduct {
            id: "P-3".to_owned(),
            name: "Green kettle".to_owned(),
            price: "3.00".to_owned(),
            other: serde_json::Map::from_iter([("colour".to_owned(), json!("green"))]),
        }]
    );
    let rejected: Vec<_> = rejected
        .into_iter()
        .map(|entry| {
            let paths: Vec<_> = entry.issues.into_iter().map(|issue| issue.path).collect();
            (entry.number, entry.data, paths, entry.url)
        })
        .collect();
    assert_eq!(
        rejected,
        [(
            1,
            json!({"name": "Nameless kettle", "price": "1.00"}),
            vec!["/id".to_owned()],
            start.clone()
        )]
    );
    assert!(past_rejected.is_none(), "{past_rejected:?}");
}
