//! A handler's calls on a document tier read the attempt's kept tree (spec
//! #1577's #1918; ADR 0138), on Foxhound's composition with its fetch tiers
//! serving one page from memory, on the paused current-thread runtime with
//! blocking work inline, or on a running clock with blocking work on
//! threads, as in production: the page is parsed once for its attempt, whatever
//! the handler calls on it and however many calls it has out at once, and
//! a program's selectors read the page as it was served after
//! `page.formats` read its main content.
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the test Clock and Entropy (ADR 0059)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::Arc;

use bytes::Bytes;
use goodfetch_counted_work::Row;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::Hello;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The page: a navigation of three links and a footer the main-content
/// step leaves out of the article it finds.
const PAGE: &str = r#"<!doctype html><html><head><title>Delta kite</title></head><body>
<nav><a href="/a">Kites</a><a href="/b">Lines</a><a href="/c">Reels</a></nav>
<main><article><h1>Delta kite</h1>
<p>The delta kite flies in light winds, its long spine and wide sail holding it steady at a high angle above the field.</p>
<p>Its keel keeps it on course while the bridle point sets how far it climbs before the line pulls it back.</p>
</article></main>
<footer><a href="/d">Contact</a></footer>
</body></html>"#;

/// The page's host, mapped to an address no connection reaches.
const HOST: &str = "kites.test";

/// The site, served from memory: [`PAGE`] at every path.
struct Site;

impl Transport for Site {
    fn send<'a>(&'a self, _hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            Ok(Sent {
                status: 200,
                headers: vec![
                    (
                        "content-type".to_owned(),
                        "text/html; charset=utf-8".to_owned(),
                    ),
                    ("content-length".to_owned(), PAGE.len().to_string()),
                ],
                body: Box::new(Whole(Some(Bytes::from_static(PAGE.as_bytes())))),
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
        Box::pin(std::future::ready(Ok(self.0.take())))
    }
}

/// A call of the handler on the page `page`, under the ID `id`.
fn call(id: &str, method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The calls the handler sends at once on the page `page`: a wait on its
/// heading, a count of its navigation's links, and its formats read from
/// its main content.
fn at_once(page: &Value) -> Vec<Value> {
    let locator = |selector: &str| json!({ "page_id": page, "selector": selector });
    vec![
        call(
            "heading",
            "locator.wait_for",
            &json!({ "locator": locator("h1"), "state": "visible" }),
        ),
        call(
            "links",
            "locator.wait_for_count",
            &json!({ "locator": locator("nav a"), "count": { "min": 1 } }),
        ),
        call(
            "formats",
            "page.formats",
            &json!({ "page_id": page, "formats": { "markdown": true, "html": { "kind": "main" } } }),
        ),
    ]
}

/// Writes `message` and its newline.
async fn send(writer: &mut ConnectionWriter, message: &Value) {
    writer
        .write_all(message.to_string().as_bytes())
        .await
        .unwrap();
    writer.write_all(b"\n").await.unwrap();
}

/// What the handler read: the replies to its calls sent at once, by ID,
/// and, once they all came, the reply to its count of the navigation's
/// links and to its wait for the footer's link, in that order.
struct Handled {
    at_once: Vec<(String, Value)>,
    after: Vec<Value>,
}

/// Opens the connection and starts the crawl of [`HOST`]'s page.
async fn start(writer: &mut ConnectionWriter) {
    send(
        writer,
        &Hello::needing(Product::Foxhound)
            .id("hello")
            .surface(Surface::Rust)
            .with(|params| params.product_version = ProductVersion::parse("1.4.0").unwrap())
            .message(),
    )
    .await;
    send(
        writer,
        &call(
            "start",
            "crawl.start",
            &json!({
                "run_id": "kites",
                "seeds": [{ "url": format!("http://{HOST}/delta"), "label": "page" }],
                "labels": { "page": {} },
                "settings": {
                    "fetch": { "tier": "http" },
                    "politeness": { "domain": { "delay": 0 } },
                    "network": { "resolve": { HOST: ["127.0.0.1"] } },
                    "egress": { "private_ranges": [HOST] },
                    "crawl": { "handler": { "concurrency": 1 } },
                },
            }),
        ),
    )
    .await;
}

/// The engine's dependencies on `env`, its clock paused and its blocking
/// work run as `blocking` says.
fn dependencies(env: &TestEnv, blocking: BlockingPolicy) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// Crawls the page once on the `http` tier, its handler sending
/// [`at_once`]'s calls together, then, once they were all answered, its
/// count of the navigation's links and a wait for the footer's link, on an
/// engine whose counted work is `metrics`' and whose blocking work runs as
/// `blocking` says.
fn crawled(env: &TestEnv, metrics: Metrics, blocking: BlockingPolicy) -> Handled {
    let tiers = FetchTiers::serving(Arc::new(Site));
    let identity = Identity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = goodfetch_foxhound::composition_with(tiers);
    let engine = Engine::new(identity, &composition)
        .unwrap()
        .with_metrics(metrics);
    // Blocking work on threads runs on a runtime whose time moves with the
    // real clock, as the paused one would move past the engine's timeouts
    // while the analysis threads work.
    let runtime = match blocking {
        BlockingPolicy::Inline => goodfetch_test_support::runtime(),
        BlockingPolicy::Threads => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    };
    runtime.block_on(async {
        // The dispatch records go into the test's exercise record (ADR
        // 0072), which the Feature audit reads for the operations the test
        // covers, timed by the engine's Clock.
        let clock = Clock::paused(env.start_time());
        let _subscriber =
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }));
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            start(&mut writer).await;
            let mut handled = Handled {
                at_once: Vec::new(),
                after: Vec::new(),
            };
            let mut handle = Value::Null;
            let mut page = Value::Null;
            while let Some(line) = lines.next_line().await.unwrap() {
                let message: Value = serde_json::from_str(&line).unwrap();
                let id = message["id"].as_str().unwrap_or_default().to_owned();
                if message["method"] == "crawl_request.handle" {
                    handle = message["id"].clone();
                    page = message["params"]["page_id"].clone();
                    for asked in at_once(&page) {
                        send(&mut writer, &asked).await;
                    }
                } else if ["heading", "links", "formats"].contains(&id.as_str()) {
                    handled.at_once.push((id, message));
                    if handled.at_once.len() == 3 {
                        let locator = |selector: &str| json!({ "page_id": page, "selector": selector });
                        send(
                            &mut writer,
                            &call(
                                "links again",
                                "locator.wait_for_count",
                                &json!({ "locator": locator("nav a"), "count": { "min": 3, "max": 3 } }),
                            ),
                        )
                        .await;
                        send(
                            &mut writer,
                            &call(
                                "footer",
                                "locator.wait_for",
                                &json!({ "locator": locator("footer a"), "state": "attached" }),
                            ),
                        )
                        .await;
                    }
                } else if id == "links again" || id == "footer" {
                    handled.after.push(message);
                    if handled.after.len() == 2 {
                        send(
                            &mut writer,
                            &json!({ "jsonrpc": "2.0", "id": handle, "result": { "records": [] } }),
                        )
                        .await;
                    }
                } else if id == "start" {
                    assert!(message.get("result").is_some(), "{message}");
                    send(
                        &mut writer,
                        &call("finish", "run.finish", &json!({ "run_id": "kites" })),
                    )
                    .await;
                } else if id == "finish" {
                    assert!(message.get("result").is_some(), "{message}");
                    break;
                }
            }
            writer.shutdown().await.unwrap();
            while let Ok(Some(_)) = lines.next_line().await {}
            handled
        };
        let dependencies = dependencies(env, blocking);
        let (served, handled) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.unwrap();
        handled
    })
}

/// A handler that sends a wait, a count and `page.formats` at once on its
/// page, then counts its navigation's links and waits for its footer's
/// link, has every call answered from the page's one parse: `formats` read
/// the main content of a copy of its tree, which left the navigation and
/// the footer out of the article, and the calls after it still read them
/// as the page was served. The Page signature read another copy, and the
/// main-content step its own copy for its landmarks fallback (#1913).
///
/// Covers: story:5-115, operation:locator.wait_for,
/// operation:locator.wait_for_count, operation:page.formats
#[test]
fn a_handlers_calls_read_its_pages_one_tree_untouched() {
    let env = TestEnv::new();
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();

    let Handled { mut at_once, after } = crawled(&env, metrics, BlockingPolicy::Inline);

    at_once.sort_by(|(left, _), (right, _)| left.cmp(right));
    let read: Vec<(String, Value)> = at_once
        .into_iter()
        .map(|(id, reply)| {
            let result = &reply["result"];
            let read = match id.as_str() {
                "formats" => json!({
                    "main_content": result["main_content"],
                    "main_holds_the_navigation": result["html"].as_str().unwrap_or_default().contains("Reels"),
                    "markdown_holds_the_article": result["markdown"].as_str().unwrap_or_default().contains("light winds"),
                }),
                "links" => result["count"].clone(),
                _ => result.clone(),
            };
            (id, if reply.get("error").is_some() { reply["error"].clone() } else { read })
        })
        .collect();
    assert_eq!(
        (
            read,
            after
                .iter()
                .map(|reply| reply
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| reply["error"].clone()))
                .collect::<Vec<_>>(),
            work.read(Row::PageParses),
            work.read(Row::PageTreeCopies),
        ),
        (
            vec![
                (
                    "formats".to_owned(),
                    json!({
                        "main_content": "found",
                        "main_holds_the_navigation": false,
                        "markdown_holds_the_article": true,
                    })
                ),
                ("heading".to_owned(), json!({})),
                ("links".to_owned(), json!(3)),
            ],
            vec![json!({ "count": 3 }), json!({})],
            1,
            3,
        )
    );
}

/// With blocking work on threads, as in production, a page's document is
/// read in its one hand-off to the analysis pool, which parses its tree
/// and keeps it there, off the async runtime, and the handler's calls
/// still read it: every call is answered with its result (issue 2673, a
/// regression from issue 2189).
///
/// Covers: story:5-115, operation:locator.wait_for,
/// operation:locator.wait_for_count, operation:page.formats
#[test]
fn a_tree_kept_on_an_analysis_thread_answers_the_handlers_calls() {
    let env = TestEnv::new();

    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();

    let Handled { mut at_once, after } = crawled(&env, metrics, BlockingPolicy::Threads);

    at_once.sort_by(|(left, _), (right, _)| left.cmp(right));
    let answered = |reply: &Value| reply.get("result").is_some();
    assert_eq!(
        (
            at_once
                .iter()
                .map(|(id, reply)| (id.as_str(), answered(reply)))
                .collect::<Vec<_>>(),
            after.iter().map(answered).collect::<Vec<_>>(),
            work.read(Row::PageAnalysisHandoffs),
            work.read(Row::PageParses),
        ),
        (
            vec![("formats", true), ("heading", true), ("links", true)],
            vec![true, true],
            1,
            1,
        ),
        "{at_once:?} {after:?}"
    );
}
