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

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::crawl::{handler, retry};
use goodfetch_foxhound::{
    Crawl, CrawlContext, CrawlRun, Dataset, DatasetDeclaration, Error, Foxhound, Schema, Seed,
    error_code,
};
use goodfetch_protocol::foxhound::dataset::RecordInvalidDetails;
use goodfetch_test_support::{ProvingGround, TestEnv, real_time_runtime};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

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

/// The Records `run` committed into `dataset`.
async fn committed<I>(run: &CrawlRun, dataset: &Dataset<I>) -> Vec<Value> {
    let mut values = Vec::new();
    let mut read = run.records(dataset);
    while let Some(record) = read.next().await {
        values.push(record.unwrap().value);
    }
    values
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
    let failures: Arc<Mutex<Vec<(&str, Refused)>>> = Arc::default();
    let escaped_attempts = Arc::new(AtomicU32::new(0));
    let (pushed, noted, failed) = (products.clone(), notes.clone(), Arc::clone(&failures));
    crawl
        .handle(&caught, move |ctx: CrawlContext<()>| async move {
            let pushes = [
                (
                    "P-1",
                    ctx.push(&pushed, &product("P-1", "Blue kettle", "12.50")),
                ),
                (
                    "P-2",
                    ctx.push(&pushed, &product("P-2", "Red kettle", "free")),
                ),
                (
                    "P-3",
                    ctx.push(&pushed, &product("P-3", "Green kettle", "9.99")),
                ),
            ];
            for (id, push) in pushes {
                if let Err(error) = push.await {
                    failed.lock().unwrap().push((id, refused(&error)));
                }
            }
            let notes = [
                ("N-1", Some("N-1"), 1.5),
                ("N-2", Some("N-2"), f64::NAN),
                ("N-3", None, 3.0),
            ];
            for (id, key, text) in notes {
                let note = Note {
                    id: key.map(str::to_owned),
                    text,
                };
                if let Err(error) = ctx.push(&noted, &note).await {
                    failed.lock().unwrap().push((id, refused(&error)));
                }
            }
            Ok(())
        })
        .unwrap();
    let (lid, attempts) = (products.clone(), Arc::clone(&escaped_attempts));
    crawl
        .handle(&escaped, move |ctx: CrawlContext<()>| async move {
            attempts.fetch_add(1, Ordering::Relaxed);
            ctx.push(&lid, &product("P-4", "Kettle lid", "1.5")).await?;
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
            committed(&run, &products).await,
            committed(&run, &notes).await,
        );
        foxhound.close().await.unwrap();
        (report, committed)
    });

    assert_eq!(
        *failures.lock().unwrap(),
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
            escaped_attempts.load(Ordering::Relaxed),
            requests.failed,
            count(&requests.by_error_code, error_code::RECORD_INVALID.as_str()),
        ),
        (1, 1, 1),
        "{requests:?}"
    );
}
