//! A code-free crawl started again under its Run ID (spec #582; ADRs
//! 0102 and 0103), on Foxhound's composition with its fetch tiers serving a
//! site from memory, on the paused current-thread runtime with blocking
//! work inline: the start's `extraction` replaces the one the Run began
//! with, as its Datasets do, so the pages the resume fetches are read and
//! checked by the extraction their schema was generated from. Each start
//! under the Run's ID answers as the start table says (ADR 0102), across
//! the engine's restarts in one home.
#![expect(
    clippy::disallowed_methods,
    reason = "the engines run on the test Clock and Entropy (ADR 0059)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::{HELLO, Surface};
use goodfetch_protocol::home::{RUN_FILE, RUNS, per_major};
use goodfetch_test_client::Hello;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The Run ID of the crawl the cases start again.
const RUN_ID: &str = "restarted";

/// The Product version the engine runs as.
const PRODUCT_VERSION: &str = "1.4.0";

/// The site, served from memory: `/a`, `/b` and `/c`, each page naming
/// itself in its `<title>`, its `<h1>` and its `<h2>`.
struct Site;

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let name = hop.url.path().trim_start_matches('/').to_owned();
        let page = ["a", "b", "c"].contains(&name.as_str()).then(|| {
            format!(
                "<!doctype html><html><head><title>Title {name}</title></head><body><h1>Heading {name}</h1><h2>Price {name}</h2></body></html>"
            )
        });
        Box::pin(async move {
            let (status, body) = match page {
                Some(page) => (200, page),
                None => (404, String::new()),
            };
            Ok(Sent {
                status,
                headers: vec![
                    (
                        "content-type".to_owned(),
                        "text/html; charset=utf-8".to_owned(),
                    ),
                    ("content-length".to_owned(), body.len().to_string()),
                ],
                body: Box::new(Whole(Some(Bytes::from(body)))),
                reused: false,
                decoded: false,
                certificate_exception: None,
            })
        })
    }
}

/// A page's body, sent in one chunk.
struct Whole(Option<Bytes>);

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let chunk = self.0.take().filter(|chunk| !chunk.is_empty());
        Box::pin(std::future::ready(Ok(chunk)))
    }
}

/// Runs a Foxhound engine whose fetch tiers serve [`Site`] on the home
/// `home`, its paused clock starting at `start`, sends `hello` and then
/// each of `calls`, each `(method, params)`, once the previous one was
/// answered, and answers each call's reply.
fn session(home: &Path, start: SystemTime, seed: u64, calls: &[(&str, Value)]) -> Vec<Value> {
    let tiers = FetchTiers::serving(Arc::new(Site));
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse(PRODUCT_VERSION).unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = goodfetch_foxhound::composition_with(tiers);
    let engine = Engine::new(identity, &composition).unwrap();
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(start);
        // The dispatch records go into the test's exercise record (ADR
        // 0072), timed by the engine's Clock.
        let wall = clock;
        let _subscriber =
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                wall.wall()
            }));
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            let mut replies = Vec::new();
            let hello = (
                HELLO,
                Hello::needing(Product::Foxhound)
                    .surface(Surface::Rust)
                    .with(|params| {
                        params.product_version = ProductVersion::parse(PRODUCT_VERSION).unwrap();
                        params.settings = connection_settings();
                    })
                    .params(),
            );
            for (index, (method, params)) in std::iter::once(&hello).chain(calls).enumerate() {
                let mut line = serde_json::to_vec(&json!({
                    "jsonrpc": "2.0", "id": index, "method": method, "params": params,
                }))
                .unwrap();
                line.push(b'\n');
                writer.write_all(&line).await.unwrap();
                loop {
                    let line = lines.next_line().await.unwrap().unwrap();
                    let message: Value = serde_json::from_str(&line).unwrap();
                    if message["id"] == json!(index) {
                        replies.push(message);
                        break;
                    }
                }
            }
            writer.shutdown().await.unwrap();
            while let Ok(Some(_)) = lines.next_line().await {}
            replies.split_off(1)
        };
        let dependencies = Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: goodfetch_engine::secrets::Keychain::absent(),
            home: Home::new(home.to_owned()),
            clock,
            entropy: Entropy::seeded(seed),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
            politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
        };
        let (served, replies) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.unwrap();
        replies
    })
}

/// The Connection's own settings, which each start of the Run inherits: a
/// 15 s Run, which only the Connection sets (ADR 0083).
fn connection_settings() -> serde_json::Map<String, Value> {
    json!({ "timeout": { "run": 15_000 } })
        .as_object()
        .cloned()
        .unwrap()
}

/// The Run's settings, at its start and at its resume: its pages fetched on
/// the `http` tier and one request every 10 s, so the Run, under its
/// Connection's 15 s, stops with `/c` pending.
fn settings() -> Value {
    json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "delay": 0 } },
        "admission": { "max_requests_per_minute": 6 },
        "network": { "resolve": { "site.test": ["127.0.0.1"] } },
        "egress": { "private_ranges": ["site.test"] },
    })
}

/// The code-free `crawl.start` of the Run of `/a`, `/b` and `/c`, reading
/// `extraction`.
fn start(extraction: &Value) -> (&'static str, Value) {
    let seeds = ["a", "b", "c"]
        .map(|name| json!({ "url": format!("http://site.test/{name}"), "label": "page" }));
    (
        "crawl.start",
        json!({
            "run_id": RUN_ID,
            "seeds": seeds,
            "datasets": [],
            "extraction": extraction,
            "settings": settings(),
        }),
    )
}

/// `run.finish` of the Run.
fn finish() -> (&'static str, Value) {
    ("run.finish", json!({ "run_id": RUN_ID }))
}

/// The run file of the Run.
fn run_file(home: &Path) -> Connection {
    let version = ProductVersion::parse(PRODUCT_VERSION).unwrap();
    Connection::open(
        per_major(home, &version)
            .join(RUNS)
            .join(RUN_ID)
            .join(RUN_FILE),
    )
    .unwrap()
}

/// The data of each Record of `pages`, in number order.
fn pages(home: &Path) -> Vec<Value> {
    let connection = run_file(home);
    let mut select = connection
        .prepare("SELECT data FROM records WHERE dataset = 'pages' ORDER BY number")
        .unwrap();
    select
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect()
}

/// Each work item of the Run's file: its URL, its status and its Error
/// code, in order.
fn work_items(home: &Path) -> Vec<(String, String, Option<String>)> {
    let connection = run_file(home);
    let mut select = connection
        .prepare("SELECT url, status, error_code FROM work_items ORDER BY position")
        .unwrap();
    select
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A code-free crawl stopped with `/c` pending and started again under
/// its Run ID with another extraction, a field added, a format added or
/// removed, or a field read anew, resumes under it: `/c` is fetched,
/// read by the new extraction and checked against the schema generated
/// from it, so it succeeds holding what the new extraction reads.
///
/// Covers: story:6-304, story:3-166, operation:crawl.start
#[rstest]
#[case::a_field_added(
    json!({ "formats": {}, "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } } }),
    json!({ "formats": {}, "fields": {
        "title": { "locator": { "kind": "css", "selector": "h1" } },
        "price": { "locator": { "kind": "css", "selector": "h2" } },
    } }),
    json!({ "title": "Heading c", "price": "Price c" }),
)]
#[case::a_format_added(
    json!({ "formats": { "links": true } }),
    json!({ "formats": { "links": true, "html": true } }),
    json!({ "links": [], "html": true }),
)]
#[case::a_format_removed(
    json!({ "formats": { "links": true, "html": true } }),
    json!({ "formats": { "links": true } }),
    json!({ "links": [], "html": false }),
)]
#[case::a_field_read_anew(
    json!({ "formats": {}, "fields": { "title": { "locator": { "kind": "css", "selector": "h1" } } } }),
    json!({ "formats": {}, "fields": { "title": { "locator": { "kind": "css", "selector": "title" } } } }),
    json!({ "title": "Title c" }),
)]
fn a_code_free_crawl_started_again_with_another_extraction_resumes_under_it(
    #[case] began_with: Value,
    #[case] again: Value,
    #[case] expected: Value,
) {
    let env = TestEnv::new();
    let stopped = session(
        env.home(),
        env.start_time(),
        env.seed(),
        &[start(&began_with), finish()],
    );
    let resumed = session(
        env.home(),
        env.start_time() + Duration::from_secs(3_600),
        env.seed(),
        &[start(&again), finish()],
    );

    let last = pages(env.home()).pop().unwrap_or_default();
    // What `/c`'s Record holds of `expected`'s fields: `html` as whether
    // it holds the format at all.
    let read: serde_json::Map<String, Value> = expected
        .as_object()
        .unwrap()
        .keys()
        .map(|key| match key.as_str() {
            "html" => (key.clone(), Value::Bool(last.get("html").is_some())),
            _ => (key.clone(), last[key].clone()),
        })
        .collect();
    assert_eq!(
        (
            &stopped[1]["result"]["lifecycle"],
            &resumed[0]["result"]["started_as"],
            &resumed[1]["result"]["lifecycle"],
            work_items(env.home()),
            Value::Object(read),
        ),
        (
            &json!("stopped"),
            &json!("resumed"),
            &json!("finished"),
            vec![
                (
                    "http://site.test/a".to_owned(),
                    "succeeded".to_owned(),
                    None
                ),
                (
                    "http://site.test/b".to_owned(),
                    "succeeded".to_owned(),
                    None
                ),
                (
                    "http://site.test/c".to_owned(),
                    "succeeded".to_owned(),
                    None
                ),
            ],
            expected,
        )
    );
}

/// What a start answered: its `started_as`, or its error's code.
fn answered(reply: &Value) -> &str {
    reply["result"]["started_as"]
        .as_str()
        .or_else(|| reply["error"]["data"]["code"].as_str())
        .unwrap_or_else(|| panic!("a start answered neither: {reply}"))
}

/// Each start under the Run's ID answers as the start table says, in the
/// table's order, across the engine's restarts in one home: `new` for a
/// Run with no state; while it runs, `run_conflict` for other seeds, then
/// `setting_not_applicable` for another value of a key fixed for the Run's
/// life (`crawl.scope.strategy`), then `run_in_use`; once a stop left it
/// interrupted, `resumed`; once it finished, `ended`; and once its run file
/// cannot be read, `run_state_unavailable`.
///
/// Covers: story:3-177, error:run_conflict, error:run_in_use,
/// error:setting_not_applicable, error:run_state_unavailable,
/// operation:crawl.start
#[test]
fn each_start_under_a_run_id_answers_as_the_start_table_says_across_restarts() {
    let env = TestEnv::new();
    let extraction = json!({ "formats": { "links": true } });
    let (method, began) = start(&extraction);
    let mut other_seeds = began.clone();
    other_seeds["seeds"] = json!([{ "url": "http://site.test/a", "label": "page" }]);
    let mut other_scope = began.clone();
    other_scope["settings"]["crawl"] = json!({ "scope": { "strategy": "same_domain" } });
    let hour = Duration::from_secs(3_600);

    let running = session(
        env.home(),
        env.start_time(),
        env.seed(),
        &[
            (method, began.clone()),
            (method, other_seeds),
            (method, other_scope),
            (method, began.clone()),
            finish(),
        ],
    );
    let resumed = session(
        env.home(),
        env.start_time() + hour,
        env.seed(),
        &[(method, began.clone()), finish()],
    );
    let ended = session(
        env.home(),
        env.start_time() + hour * 2,
        env.seed(),
        &[(method, began.clone())],
    );
    run_file(env.home())
        .execute_batch("DROP TABLE run")
        .unwrap();
    let unreadable = session(
        env.home(),
        env.start_time() + hour * 3,
        env.seed(),
        &[(method, began)],
    );

    assert_eq!(
        (
            running[..4].iter().map(answered).collect::<Vec<_>>(),
            &running[4]["result"]["lifecycle"],
            answered(&resumed[0]),
            &resumed[1]["result"]["lifecycle"],
            answered(&ended[0]),
            answered(&unreadable[0]),
        ),
        (
            vec![
                "new",
                "run_conflict",
                "setting_not_applicable",
                "run_in_use"
            ],
            &json!("stopped"),
            "resumed",
            &json!("finished"),
            "ended",
            "run_state_unavailable",
        )
    );
}
