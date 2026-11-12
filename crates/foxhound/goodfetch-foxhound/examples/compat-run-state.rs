//! Run state's writer for the Compatibility corpus (ADR 0066): runs fixed
//! scenarios against Foxhound's engine, whose run files hold every table
//! both Products' compositions add, over the in-memory network, on the test
//! `Clock` and a fixed `Entropy` seed, in a home of its own, its crawls
//! fetching a site served from memory on a forced `http` tier
//! (`support/site.rs`): under an
//! install Limit, whose Slices its run state journals, a tagged crawl of
//! six seeds, one with a Payload of a Label declared with its schema, beside
//! one declared without, one the Frontier answers `duplicate`,
//! one sent as a `POST` with a body under a declared key and one
//! `crawl.max_segment_repeats` refuses, under a scope of every host
//! narrowed by an `include` and an `exclude` pattern and under a depth and
//! a request bound, and eleven more seeds on `list.test`, three pairs of
//! items with and without `ref`, three offsets of one listing and a guide
//! with and without its trailing slash, whose pages the writer's own
//! contribution hands the Frontier, which learns that `ref` has no effect,
//! ends a request of item 3 still pending then
//! and merges a seed it admits then, counts item 1's `rel=canonical`, reads
//! the cursor each page of the listing links until the third teaches it, and
//! learns the guide's trailing slash from its redirect until its two pages
//! differ and stop the rule, and a sitemap seed whose sitemap it reads as
//! listing an entry no rule matches and one the scope refuses, beside one
//! whose sitemap answered 404, whose handler enqueues a `forefront` request on another domain from the
//! first and pushes there a Record the engine rejects into its rejection table, answers Records for its two Datasets, `pages` with a schema and
//! a key and `links` with neither, for the first, the `POST` and that request and
//! fails on the second, whose first byte never came, so its
//! domain still backs off when `run.finish` ends it, and whose last site
//! blocks its address, so its Session pool holds a Session retired by the
//! escalation, its position in the `egress.local_addresses` rotation and
//! the address block it blocked, and its domain's proxy tier; a code-free
//! crawl, whose extraction its run file keeps, of one seed
//! `crawl.max_segment_repeats` refuses, so it finishes at once; and a Run its
//! program drives, under an Operator policy and with a setting that
//! weakens a guarantee, whose Key-value store keeps a cursor, a note in its
//! row and an archive over 1 MiB as a kept file, and whose run
//! state holds its data key wrapped, which `run.abort` stops; and a Run
//! bound to a
//! Browser host and holding to a Clean-up rules digest, started through a
//! contribution of the writer's own since no Client sets those yet, with a
//! warning and `nothingDone` its owners handed over, and three Sessions its
//! program opened, its default one open, another closed, with a proxy of
//! its own, and a third closed, on a Borrowed browser, which that
//! contribution records since no browser runs here, and a Download's file
//! it keeps, which its Checkpoints carry, the robots.txt copies of two
//! origins its program's navigation read, one the site answered, one it
//! never did, whose program sets a progress
//! message, which `run.checkpoint` checkpoints and
//! `run.pause` pauses with a Checkpoint.
//! It writes each Run's file, as SQL text, into the folder given as the
//! only argument: `crawl.sql`, `code-free.sql`, `aborted.sql`,
//! `paused.sql` and `unclean.sql`, a Run at `off` whose writer then opens
//! its file again and ends as a crash ends it, without clearing its
//! "unclean" mark; and the
//! pause's real Checkpoint file, `checkpoint.sqlite`, as SQL text too,
//! `checkpoint.sql`, which alone holds the chunks of a kept file, with the
//! Checkpoint the pause reported, `checkpoint.json`, its location the file's name
//! (ADR 0066). `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime, writes files and runs on the test Clock (ADRs 0059 and 0066)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::robots_txt::Answer;
use goodfetch_engine::run_files::FileKind;
use goodfetch_engine::run_key::{self, StoredKey, Wrapped};
use goodfetch_engine::run_state::{Durability, RunState, RunStateOptions};
use goodfetch_engine::runs::{EndedBy, Start, Started};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Contribution, Dependencies, Engine, Handler, Identity,
};
use goodfetch_fetch_tier::FetchTiers;
use goodfetch_frontier::PageSeen;
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::checkpoint::RunBinding;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::home::{OPERATOR_POLICY_FILE, RUN_FILE, RUNS, per_major};
use goodfetch_protocol::run::{NothingDone, RunId, RunKind, RunWarning, Tags};
use goodfetch_test_client::Hello;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

#[path = "support/site.rs"]
mod site;

/// How long the site takes to answer, on the paused clock, which passes
/// once the engine is idle: the crawl's pages arrive after `compat.learned`
/// taught the Frontier its own, so the lessons come in the scenario's order.
const SITE_LATENCY: Duration = Duration::from_secs(1);

/// What the Client asks, in order; each call waits for its reply.
fn scenario() -> Vec<Value> {
    // Every host a request of the crawls names, so that no name is looked
    // up: a lookup takes real time, which the paused clock does not keep.
    let (fetch, network) = site::settings(&[
        "shop.test",
        "api.test",
        "guarded.test",
        "list.test",
        "maps.test",
        "outlet.test",
    ]);
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    vec![
        call(
            0,
            "hello",
            Hello::needing(Product::Foxhound)
                .surface(Surface::Rust)
                .params(),
        ),
        call(
            10,
            "usage.set",
            json!({ "scope": "install", "usage_kind": "requests", "amount": 1_000 }),
        ),
        call(
            1,
            "crawl.start",
            json!({
                "run_id": "crawl",
                "tags": ["schedule:nightly"],
                "labels": {
                    "home": { "payload": { "type": ["object", "null"] } },
                    "product": {},
                },
                "datasets": [
                    {
                        "name": "pages",
                        "schema": {
                            "type": "object",
                            "required": ["url"],
                            "properties": { "url": { "type": "string" }, "title": { "type": "string" } },
                        },
                        "key": ["url"],
                    },
                    { "name": "links" },
                ],
                "seeds": [
                    { "url": "https://shop.test/", "label": "home", "payload": { "depth": 0 } },
                    { "url": "https://shop.test/broken", "label": "product" },
                    { "url": "https://SHOP.test:443/", "label": "home" },
                    { "url": "https://api.test/search", "label": "home", "method": "POST", "body": "eyJwYWdlIjoxfQ==", "key": "search-1" },
                    { "url": "https://guarded.test/", "label": "guarded" },
                    { "url": "https://shop.test/more/more/more/", "label": "home" },
                    { "url": "https://list.test/items?n=1", "label": "listed" },
                    { "url": "https://list.test/items?n=1&ref=a", "label": "listed" },
                    { "url": "https://list.test/items?n=2", "label": "listed" },
                    { "url": "https://list.test/items?n=2&ref=b", "label": "listed" },
                    { "url": "https://list.test/items?n=3", "label": "listed" },
                    { "url": "https://list.test/items?n=3&ref=c", "label": "listed" },
                    { "url": "https://list.test/rows?offset=0", "label": "listed" },
                    { "url": "https://list.test/rows?offset=20", "label": "listed" },
                    { "url": "https://list.test/rows?offset=40", "label": "listed" },
                    { "url": "https://shop.test/pages/1", "label": "home" },
                    { "url": "https://shop.test/pages/2", "label": "home" },
                    { "url": "https://list.test/guide", "label": "listed" },
                    { "url": "https://list.test/guide/", "label": "listed" },
                ],
                "settings": {
                    "crawl": {
                        "scope": { "strategy": "all", "include": ["**/sale"], "exclude": ["**/admin/**"] },
                        "max_depth": 3,
                        "max_requests": 100,
                        "path_budgets": [{ "pattern": "/pages/*", "max_requests": 1 }],
                    },
                    "egress": { "local_addresses": ["127.0.0.1"] },
                    "fetch": fetch,
                    "network": network,
                },
            }),
        ),
        call(15, "compat.learned", json!({ "run_id": "crawl" })),
        call(2, "run.finish", json!({ "run_id": "crawl" })),
        call(
            3,
            "run.start",
            json!({
                "run_id": "aborted",
                "tags": ["manual"],
                "settings": { "tls": { "accept_invalid_certs": { "intranet.test": null } } },
            }),
        ),
        call(
            11,
            "key_value_store.set",
            json!({ "run_id": "aborted", "key": "cursor", "value": { "page": 2, "seen": [1, null] } }),
        ),
        call(
            13,
            "key_value_store.set_bytes",
            json!({ "run_id": "aborted", "key": "note", "content": "aGk=", "content_type": "text/plain" }),
        ),
        // 1,048,578 zero bytes, a multiple of 3 just over 1 MiB, are this
        // Base64 with no padding: kept as a file the row references.
        call(
            14,
            "key_value_store.set_bytes",
            json!({ "run_id": "aborted", "key": "archive", "content": "A".repeat(1_398_104), "content_type": "application/octet-stream" }),
        ),
        call(12, "compat.keyed", json!({ "run_id": "aborted" })),
        call(4, "run.abort", json!({ "run_id": "aborted" })),
        call(
            16,
            "crawl.start",
            json!({
                "run_id": "code-free",
                "seeds": [{ "url": "https://shop.test/more/more/more/", "label": "page" }],
                "extraction": { "formats": { "markdown": true }, "follow": "none" },
                "settings": { "fetch": fetch, "network": network },
            }),
        ),
        // A Run at `off`, whose file the writer reopens below.
        call(
            17,
            "run.start",
            json!({ "run_id": "unclean", "settings": { "run_state": { "durability": "off" } } }),
        ),
        call(5, "compat.bound", json!({})),
        call(
            6,
            "run.set_progress_message",
            json!({ "run_id": "paused", "message": "Waiting for the next page" }),
        ),
        call(7, "run.checkpoint", json!({ "run_id": "paused" })),
        call(8, "run.pause", json!({ "run_id": "paused" })),
    ]
}

/// The digest of the Clean-up rules the bound Run records.
const CLEAN_UP_DIGEST: &str = "6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b";

/// Starts the Run `paused`, bound to a Browser host and holding to
/// [`CLEAN_UP_DIGEST`], which no Client's call sets yet.
const BOUND: Operation = Operation {
    name: "compat.bound",
    description: "Starts a Run bound to a Browser host with a Clean-up rules digest.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Stores a wrapped data key in a live Run's run state: one of the
/// layout's shape (version, nonce, 32 bytes and their tag), fixed, since a
/// real one is random and the set must be the same at each write.
const KEYED: Operation = Operation {
    name: "compat.keyed",
    description: "Stores a fixed wrapped data key in a Run's run state.",
    ..BOUND
};

async fn keyed(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap_or_default()).expect("a Run ID");
    let run = context.runs().live(run_id).await.expect("the Run is live");
    let stored = StoredKey {
        wrapped: Wrapped::from_bytes(
            std::iter::once(1_u8)
                .chain((0..72).map(|byte: u8| byte.wrapping_mul(37)))
                .collect(),
        ),
        key_reference: "secretfile:run-state-key".to_owned(),
    };
    run.state()
        .write(move |connection| run_key::store(connection, &stored))
        .await
        .expect("the wrapped data key is kept");
    Ok(json!({ "keyed": true }))
}

/// Hands the Frontier of a live crawl the pages of its `list.test/items`
/// seeds, as a fetch tier would once it fetched them, before the site
/// served from memory answers with pages that read the same way: item `n`'s page reads alike with or without
/// `ref`, so `ref` is learned to have no effect, which ends a request of
/// item 3 admitted pending just before, and a seed differing from
/// an admitted one only by `ref`, admitted in the same write so that no
/// attempt of the crawl's runs between, is merged. Each page of the
/// `list.test/rows` listing then links a cursor of its own, which the
/// third teaches.
const LEARNED: Operation = Operation {
    name: "compat.learned",
    description: "Hands a crawl's Frontier the pages of its `list.test/items` seeds.",
    ..BOUND
};

async fn learned(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap_or_default()).expect("a Run ID");
    let run = context.runs().live(run_id).await.expect("the Run is live");
    run.state()
        .write(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, url FROM work_items WHERE url LIKE 'https://list.test/items%' ORDER BY position",
            )?;
            let pages = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let get = HttpMethod::get();
            let scope = goodfetch_frontier::Scope::read(connection)?;
            let policy = goodfetch_engine::egress::Policy::new(&["*"], &[] as &[&str])
                .expect("every host is a policy");
            let pending = goodfetch_frontier::admit(
                connection,
                &scope,
                &policy,
                goodfetch_frontier::Candidate::Seed {
                    url: "https://list.test/items?n=3&ref=d",
                    declared: goodfetch_frontier::Declared {
                        method: &get,
                        body: None,
                        key: None,
                        keeps_fragment: false,
                    },
                },
                |url| work_item(connection, url, "pending"),
            )?;
            assert!(
                matches!(pending, goodfetch_protocol::foxhound::frontier::Enqueued::Added { .. }),
                "the request the rule ends is admitted: {pending:?}"
            );
            for (crawl_request, url) in pages {
                let url = goodfetch_frontier::Url::parse(&url).expect("a seed's URL parses");
                let item = url
                    .query_pairs()
                    .find(|(name, _)| name == "n")
                    .map_or(0, |(_, value)| value.bytes().next().unwrap_or(0));
                // Item 1 names its `http` URL as its canonical, evidence
                // short of the `scheme` form.
                let canonical = (url.query() == Some("n=1"))
                    .then(|| goodfetch_frontier::Url::parse("http://list.test/items?n=1"))
                    .transpose()
                    .expect("a canonical parses");
                goodfetch_frontier::learn(
                    connection,
                    PageSeen {
                        crawl_request: &crawl_request,
                        url: &url,
                        method: &get,
                        body: None,
                        signature: [item; 32],
                        canonical: canonical.as_ref(),
                    },
                    &mut Ending,
&goodfetch_counted_work::CountedWork::default(),
                )?;
            }
            learn_guide(connection)?;
            let merged = goodfetch_frontier::admit(
                connection,
                &scope,
                &policy,
                goodfetch_frontier::Candidate::Seed {
                    url: "https://list.test/items?n=1&ref=z",
                    declared: goodfetch_frontier::Declared {
                        method: &get,
                        body: None,
                        key: None,
                        keeps_fragment: false,
                    },
                },
                |_| Err(rusqlite::Error::InvalidQuery),
            )?;
            assert!(
                matches!(merged, goodfetch_protocol::foxhound::frontier::Enqueued::NotAdded { .. }),
                "the learned rule merges the seed"
            );
            link_cursors(connection, &scope, &policy)?;
            read_sitemap(connection)
        })
        .await
        .expect("the pages are learned from");
    Ok(json!({ "learned": true }))
}

/// Hands the Frontier the guide of `list.test`: it redirects to its slashed
/// URL, which teaches the trailing slash's form, and then both URLs serve
/// pages that differ, which stops it.
fn learn_guide(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    let get = HttpMethod::get();
    let mut statement = connection.prepare(
        "SELECT id, url FROM work_items WHERE url LIKE 'https://list.test/guide%' ORDER BY position",
    )?;
    let guides = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let urls: Vec<goodfetch_frontier::Url> = guides
        .iter()
        .map(|(_, url)| goodfetch_frontier::Url::parse(url).expect("a seed's URL parses"))
        .collect();
    goodfetch_frontier::learn_redirect(
        connection,
        goodfetch_frontier::Redirect {
            from: &urls[0],
            to: &urls[1],
            method: &get,
            body: None,
        },
        &mut Ending,
        &goodfetch_counted_work::CountedWork::default(),
    )?;
    for (index, ((crawl_request, _), url)) in guides.iter().zip(&urls).enumerate() {
        goodfetch_frontier::learn(
            connection,
            PageSeen {
                crawl_request,
                url,
                method: &get,
                body: None,
                signature: [u8::try_from(index).expect("two guides") + b'g'; 32],
                canonical: None,
            },
            &mut Ending,
            &goodfetch_counted_work::CountedWork::default(),
        )?;
    }
    Ok(())
}

/// Admits the link each page of the `list.test/rows` listing gives to the
/// rows after it by a cursor of its own, a token that ends as the scope's
/// `include` asks, which the third page's link teaches.
fn link_cursors(
    connection: &rusqlite::Connection,
    scope: &goodfetch_frontier::Scope,
    policy: &goodfetch_engine::egress::Policy,
) -> rusqlite::Result<()> {
    for (offset, after) in [(0, "r20"), (20, "r40"), (40, "r60")] {
        let page =
            goodfetch_frontier::Url::parse(&format!("https://list.test/rows?offset={offset}"))
                .expect("a listing's page parses");
        let cursor = goodfetch_frontier::admit(
            connection,
            scope,
            policy,
            goodfetch_frontier::Candidate::Href {
                href: &format!("?after={after}/sale"),
                base: &page,
                encoding: goodfetch_frontier::Encoding::for_label(b"utf-8")
                    .expect("UTF-8 is an encoding"),
                keeps_fragment: false,
                from: &page,
                narrowing: goodfetch_frontier::Narrowing::none(),
                depth: 1,
            },
            |url| work_item(connection, url, "succeeded"),
        )?;
        assert!(
            matches!(
                cursor,
                goodfetch_protocol::foxhound::frontier::Enqueued::Added { .. }
            ),
            "the cursor link is admitted: {cursor:?}"
        );
    }
    Ok(())
}

/// Admits a sitemap seed on `maps.test` whose fetch already ended, since
/// no fetch tier runs over the in-memory network, and reads its sitemap as
/// listing `/about`, which its rule does not match (`no_rule`), and
/// `/books/1`, which the scope's `include` refuses (`out_of_scope`); and a
/// second seed whose sitemap answered 404, its fetch failed.
fn read_sitemap(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    use goodfetch_frontier::sitemap;
    use goodfetch_protocol::foxhound::crawl::{SitemapRule, SitemapSeed};
    use goodfetch_protocol::foxhound::frontier::Enqueued;
    use goodfetch_protocol::foxhound::outcomes::{AttemptError, Ending};

    let policy = goodfetch_engine::egress::Policy::new(&["*"], &[] as &[&str])
        .expect("every host is a policy");
    let seed = SitemapSeed {
        sitemap: "https://maps.test/sitemap.xml".to_owned(),
        rules: vec![SitemapRule {
            pattern: "**/books/*".to_owned(),
            label: "home".to_owned(),
        }],
    };
    let admit = |seed: &SitemapSeed, status: &str| {
        let answer = sitemap::admit_seed(connection, &policy, seed, |url| {
            work_item(connection, url, status)
        })?;
        let Enqueued::Added { id, .. } = answer else {
            panic!("the sitemap seed is admitted: {answer:?}");
        };
        Ok::<String, rusqlite::Error>(id)
    };
    let id = admit(&seed, "succeeded")?;
    sitemap::read(
        connection,
        &goodfetch_frontier::Scope::read(connection)?,
        &policy,
        &id,
        &sitemap::Document::Entries(vec!["/about".to_owned(), "/books/1".to_owned()]),
        &goodfetch_counted_work::CountedWork::default(),
        |_, _| Err(rusqlite::Error::InvalidQuery),
    )?;
    let missing = SitemapSeed {
        sitemap: "https://maps.test/missing.xml".to_owned(),
        rules: seed.rules.clone(),
    };
    let id = admit(&missing, "failed")?;
    let mut details = serde_json::Map::new();
    details.insert("url".to_owned(), json!(missing.sitemap));
    details.insert("status".to_owned(), json!(404));
    sitemap::ended(
        connection,
        &id,
        &Ending::Failed {
            error: AttemptError {
                attempt: 1,
                code: "response_not_ok".to_owned(),
                message: format!(
                    "The sitemap {} answered 404, not a sitemap.",
                    missing.sitemap
                ),
                details,
            },
        },
    )
}

/// Adds the work item of `url`, already `status`, as the writer's own
/// admissions do where no fetch tier runs: at the position after every one
/// the run file holds or an attempt's staged enqueue reserved, as the
/// crawl's own adds take theirs. Answers its ID.
fn work_item(
    connection: &rusqlite::Connection,
    url: &goodfetch_frontier::Url,
    status: &str,
) -> rusqlite::Result<String> {
    connection.query_row(
        "INSERT INTO work_items
             (id, position, url, method, label, depth, status, attempts, retried_from, lease_epoch, domain_key, priority)
         SELECT CAST(next AS TEXT), next, ?1, 'GET', '', 0, ?2, 1, 0, 1, ?3, 0
         FROM (SELECT max(
             (SELECT coalesce(max(position), 0) FROM work_items),
             (SELECT coalesce(max(reserved), 0) FROM work_item_positions)
         ) + 1 AS next)
         RETURNING id",
        [url.as_str(), status, url.host_str().unwrap_or_default()],
        |row| row.get(0),
    )
}

/// The writer's side of a rule learned later: of the requests the rule
/// shows to be one page, it ends each still pending, `skipped`, as the
/// crawl ends them. The one rule it stops had ended none, so it has none
/// to give back.
struct Ending;

impl goodfetch_frontier::EndMerged for Ending {
    fn end_merged(
        &mut self,
        connection: &rusqlite::Connection,
        crawl_requests: &std::collections::BTreeSet<String>,
    ) -> rusqlite::Result<Vec<goodfetch_frontier::Ended>> {
        let mut ended = Vec::new();
        for crawl_request in crawl_requests {
            let url = connection
                .query_row(
                    "UPDATE work_items SET status = 'skipped'
                     WHERE id = ?1 AND status = 'pending' RETURNING url",
                    [crawl_request],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    error => Err(error),
                })?;
            ended.extend(url.map(|url| goodfetch_frontier::Ended {
                crawl_request: crawl_request.clone(),
                url,
            }));
        }
        Ok(ended)
    }

    fn give_back(
        &mut self,
        _: &rusqlite::Connection,
        _: &[String],
    ) -> rusqlite::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

fn bound() -> Contribution {
    Contribution::new("compat-run-state")
        .with_handler(Handler::new(&KEYED, keyed))
        .with_handler(Handler::new(&LEARNED, learned))
        .with_handler(Handler::new(
        &BOUND,
        |_: Value, context: CallContext| async move {
            let started = context
                .runs()
                .start(Start {
                    run_id: Some(RunId::parse("paused").expect("a Run ID")),
                    tags: Tags::default(),
                    kind: RunKind::Collie,
                    needs: Product::Collie,
                    inputs: BTreeMap::new(),
                    ended_by: EndedBy::Finish,
                    initialize: None,
                    policy_version: None,
                    weakened: Vec::new(),
                    binding: Some(RunBinding::BrowserHost {
                        browser_host: "laptop".to_owned(),
                    }),
                    clean_up_digest: Some(CLEAN_UP_DIGEST.to_owned()),
                    checkpoint_interval: None,
                    timeout: None,
                    settings: None,
                })
                .await
                .map_err(goodfetch_engine::run::start_error)?;
            if let Started::New(run) = started {
                let warning = RunWarning {
                    code: "storage_low".to_owned(),
                    message: "The run folder's filesystem met the `low` storage level.".to_owned(),
                    sessions: vec!["laptop".to_owned()],
                    folder: Some("runs/paused".to_owned()),
                };
                run.warn(warning).await.expect("the warning is kept");
                run.nothing_done(NothingDone::NothingPending)
                    .await
                    .expect("`nothingDone` is kept");
                run.charge_part("proxy_bytes", "datacenter", 4_096, None)
                    .await
                    .expect("the proxy bytes are counted");
                // Its program's own navigation read two origins' robots.txt
                // into the Run's own copies (ADR 0098): one the site
                // answered, one it never did.
                for (location, answer) in [
                    (
                        "https://shop.test/robots.txt",
                        Answer::Response {
                            status: 200,
                            body: b"User-agent: *\nDisallow: /cart/\n".to_vec(),
                        },
                    ),
                    (
                        "https://down.test/robots.txt",
                        Answer::Unreachable(
                            "the navigation to https://down.test/robots.txt failed: net::ERR_CONNECTION_REFUSED"
                                .to_owned(),
                        ),
                    ),
                ] {
                    let location = url::Url::parse(location).expect("a robots.txt URL");
                    run.own_robots_txt(&location, async { Ok(answer) })
                        .await
                        .expect("the copy is read");
                }
                run.keep_file(
                    FileKind::Download,
                    "books.csv".to_owned(),
                    std::io::Cursor::new(b"title,price\nDune,9.99\n".to_vec()),
                )
                .await
                .expect("the Download's file is kept");
                run.state()
                    .write(|connection| {
                        connection
                            .execute(
                                r#"INSERT INTO page_sessions (position, is_default, closed, settings, identity_seed, source) VALUES (1, 1, 0, '{}', 6178731391398722387, NULL), (2, 0, 1, '{"filter_lists.ads":false,"proxy.server":"http://proxy.example:3128","proxy.username":"user","proxy.password":"env:PROXY_PASSWORD"}', -2937466112876520115, NULL), (3, 0, 1, '{}', 4417291851073961183, '{"url":"ws://127.0.0.1:9222/devtools/browser/lent","layer":"run","acting_for_model":false}')"#,
                                [],
                            )
                            .map(drop)?;
                        // What a Label learned on a domain (#926): a split
                        // Label, one segment on `http`, one still sampled.
                        connection
                            .execute(
                                r#"INSERT INTO tier_choices (label, domain_key, unit) VALUES ('home', 'shop.test', '{"kind":"split","segments":[{"name":"products","matched":3,"tier":"http"},{"name":"blog","matched":0,"tier":"browser"},{"name":"mixed","matched":1,"tier":null}],"since":1790000000000}')"#,
                                [],
                            )
                            .map(drop)?;
                        // A request added through a link its served
                        // document's markup hides (#1153).
                        goodfetch_frontier::trap_links::suspect(
                            connection,
                            "2",
                            goodfetch_frontier::trap_links::Mark::Hidden,
                        )
                    })
                    .await
                    .expect("the Sessions, a tier choice and a suspected trap link are kept");
            }
            Ok::<Value, CallError>(json!({ "run_id": "paused" }))
        },
    ))
}

/// The handler's enqueue, from the home page's attempt, of a request that
/// goes ahead of its domain's pending ones, on a domain of its own so the
/// home page's domain still waits when `run.finish` ends the crawl. It is
/// sent once `compat.learned`'s write committed, so the crawl's writes
/// interleave alike at every write, however soon the attempt starts.
fn enqueue(request: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "enqueue",
        "method": "crawl_request.enqueue",
        "params": {
            "run_id": request["params"]["run_id"],
            "crawl_request_id": request["params"]["crawl_request"]["id"],
            "lease_epoch": request["params"]["lease_epoch"],
            "url": "https://outlet.test/sale",
            "label": "home",
            "forefront": true,
        },
    })
}

/// The handler's push, from the home page's attempt, of a `pages` Record
/// without its `url`, which the engine's check rejects, so its rejection
/// enters `pages`' rejection table with the attempt.
fn reject(request: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "reject",
        "method": "crawl_request.push",
        "params": {
            "run_id": request["params"]["run_id"],
            "crawl_request_id": request["params"]["crawl_request"]["id"],
            "lease_epoch": request["params"]["lease_epoch"],
            "dataset": "pages",
            "data": { "title": "No address" },
        },
    })
}

/// What the home page's attempt sends after `received`, the reply to its
/// enqueue or to its push: the rejected push, then its answer, which ends
/// the attempt; `None` for any other message.
fn after_enqueue(enqueuing: &mut Option<Value>, received: &Value) -> Result<Option<Value>, String> {
    let Some(request) = enqueuing.as_ref() else {
        return Ok(None);
    };
    if received["id"] == "enqueue" {
        if received["result"]["kind"] != "added" {
            return Err(format!("the enqueue failed: {received}"));
        }
        return Ok(Some(reject(request)));
    }
    if received["id"] == "reject" {
        if received["error"]["data"]["code"] != "record_invalid" {
            return Err(format!("the push was not rejected: {received}"));
        }
        return Ok(enqueuing.take().map(|request| handle(&request)));
    }
    Ok(None)
}

/// The handler's answer to one handler request.
fn handle(request: &Value) -> Value {
    let id = &request["id"];
    if request["params"]["crawl_request"]["label"] == "guarded" {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32003,
                "message": "the site refused the address",
                "data": { "code": "blocked", "retryable": false, "cause": "ip_block" },
            },
        })
    } else if request["params"]["crawl_request"]["label"] == "home" {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "records": [
                { "dataset": "pages", "data": { "url": "https://shop.test/", "title": "Shop" } },
                { "dataset": "links", "data": { "href": "https://shop.test/broken" } },
            ] },
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32005,
                "message": "no first byte",
                "data": {
                    "code": "deadline_exceeded",
                    "retryable": false,
                    "limit": { "key": "timeout.first_byte", "value": 30_000, "layer": "default" },
                    "elapsed": 30_000,
                    "waited_for": "first_byte",
                },
            },
        })
    }
}

fn dependencies(home: &Path) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()),
        // 2026-01-05T09:00:00Z, test support's `START_TIME`.
        clock: Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001)),
        entropy: Entropy::seeded(0),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

fn line(message: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    Ok(line)
}

/// Opens the run file at `path` at `off` again and ends its writer without
/// its clean stop, as a crash ends it: the file keeps its "unclean" mark
/// (ADR 0138).
async fn leave_unclean(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let options = RunStateOptions {
        durability: Durability::Off,
        ..RunStateOptions::default()
    };
    let writer = RunState::open(path.to_owned(), &options, Vec::new(), 1, &owner).await?;
    drop(writer);
    owner.wait().await;
    Ok(())
}

/// Writes each Run's file in `runs`, as SQL text, into `folder`.
fn dump_run_files(runs: &Path, folder: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for run_id in ["crawl", "code-free", "aborted", "paused", "unclean"] {
        let connection = rusqlite::Connection::open(runs.join(run_id).join(RUN_FILE))?;
        std::fs::write(
            folder.join(format!("{run_id}.sql")),
            goodfetch_test_support::sql_dump(&connection)?,
        )?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: compat-run-state <folder>")?,
    );
    let version = ProductVersion::parse("0.0.0")?;
    let identity = Identity {
        product: Product::Foxhound,
        product_version: version.clone(),
        channel: Channel::CratesIo,
    };
    let tiers = FetchTiers::serving(Arc::new(site::Site::new(SITE_LATENCY, &[])));
    let composition = goodfetch_foxhound::composition_with(tiers).with(bound());
    let engine = Engine::new(identity, &composition)?;
    let home = tempfile::tempdir()?;
    // The folder of the engine's own pinned version, not of the workspace's.
    let engine_folder = per_major(home.path(), &version);
    std::fs::create_dir_all(&engine_folder)?;
    std::fs::write(
        engine_folder.join(OPERATOR_POLICY_FILE),
        "version = \"0.1\"\n",
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    let paused = runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await?.split();
            let mut messages = BufReader::new(reader).lines();
            let mut last = Value::Null;
            let mut enqueuing = None;
            // The home page's handler request, held until `compat.learned`
            // answered.
            let mut home = None;
            let mut learned = false;
            for message in scenario() {
                writer.write_all(&line(&message)?).await?;
                loop {
                    let received: Value = serde_json::from_str(
                        &messages
                            .next_line()
                            .await?
                            .ok_or("the engine closed early")?,
                    )?;
                    if received["method"] == "crawl_request.handle" {
                        if received["params"]["crawl_request"]["url"] == "https://shop.test/" {
                            if learned {
                                writer.write_all(&line(&enqueue(&received))?).await?;
                                enqueuing = Some(received);
                            } else {
                                home = Some(received);
                            }
                        } else {
                            writer.write_all(&line(&handle(&received))?).await?;
                        }
                    } else if let Some(next) = after_enqueue(&mut enqueuing, &received)? {
                        writer.write_all(&line(&next)?).await?;
                    } else if received["id"] == message["id"] {
                        if received.get("error").is_some() {
                            return Err(format!("the scenario failed: {received}").into());
                        }
                        learned |= message["method"] == "compat.learned";
                        if learned && let Some(request) = home.take() {
                            writer.write_all(&line(&enqueue(&request))?).await?;
                            enqueuing = Some(request);
                        }
                        last = received;
                        break;
                    }
                }
            }
            writer.shutdown().await?;
            while messages.next_line().await?.is_some() {}
            Ok::<_, Box<dyn std::error::Error>>(last)
        };
        let (served, talked) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client
        );
        served?;
        talked
    })?;
    let runs = engine_folder.join(RUNS);
    runtime.block_on(leave_unclean(&runs.join("unclean").join(RUN_FILE)))?;
    dump_run_files(&runs, &folder)?;
    std::fs::copy(
        runs.join("paused").join("checkpoint.sqlite"),
        folder.join("checkpoint.sqlite"),
    )?;
    let checkpointed = rusqlite::Connection::open(runs.join("paused").join("checkpoint.sqlite"))?;
    std::fs::write(
        folder.join("checkpoint.sql"),
        goodfetch_test_support::sql_dump(&checkpointed)?,
    )?;
    let mut checkpoint = paused["result"]["checkpoint"].clone();
    checkpoint["location"] = json!("checkpoint.sqlite");
    let mut text = serde_json::to_string_pretty(&checkpoint)?;
    text.push('\n');
    std::fs::write(folder.join("checkpoint.json"), text)?;
    Ok(())
}
