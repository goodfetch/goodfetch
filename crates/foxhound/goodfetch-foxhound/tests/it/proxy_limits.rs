//! A proxy's limits on Foxhound's `http` tier (ADRs 0053 and 0088; ticket
//! #2360), on its executable against the Proving ground's forward proxies,
//! read from the per-user Politeness file: `politeness.proxy.concurrency`
//! and `politeness.proxy.rate` hold the hops through one proxy endpoint and
//! credential, so the `limited` proxy, which relays one request at a time
//! and answers any other with its own `429`, refuses none; and the proxy's
//! own `429` or `502` fails its hop with `proxy_unavailable`, never
//! `blocked`, lowers the ceiling learned for that proxy and credential, and
//! backs off no domain. (A `407` failing with `proxy_auth_failed` is
//! `pool_proxies`' case.)
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;

/// The Proving ground's seed.
const SEED: u64 = 1160;

/// The crawl's Run.
const RUN_ID: &str = "through-a-proxy";

/// The proxy the hops go through: the Proving ground's `limited` one, or
/// its `open` one.
#[derive(Clone, Copy, Debug)]
enum Proxy {
    Limited,
    Open,
}

impl Proxy {
    fn name(self) -> &'static str {
        match self {
            Self::Limited => "limited",
            Self::Open => "open",
        }
    }
}

/// Two pages of the site's at once, each relayed by the proxy as one
/// request.
fn two_pages(proving_ground: &ProvingGround) -> Vec<String> {
    ["one", "two"]
        .iter()
        .map(|mark| proving_ground.url("scope.test", &format!("/quiet?mark={mark}")))
        .collect()
}

/// A page of the site at the Proving ground's `down` port, a port of the
/// site's host where nothing listens and no other program can, so the
/// proxy cannot connect and answers with its own `502`,
/// `connection_refused`.
fn refused(proving_ground: &ProvingGround) -> Vec<String> {
    let down: std::net::SocketAddr = proving_ground.description().fixtures.proxies["down"]
        .parse()
        .expect("the down address is a socket address");
    vec![format!(
        "http://scope.test:{}/quiet?mark=refused",
        down.port()
    )]
}

/// What the crawl met through a proxy: its outcomes, each the kind and
/// error code, sorted; the ceiling the Politeness file holds for each proxy
/// key; and the backoff it holds for the site's domain, if any.
type Met = (
    Vec<(Value, Value)>,
    BTreeSet<(String, Option<i64>)>,
    Option<i64>,
);

/// The Politeness file's ceilings, by proxy key.
fn ceilings(file: &Path) -> BTreeSet<(String, Option<i64>)> {
    let connection = Connection::open(file).unwrap();
    let mut statement = connection
        .prepare("SELECT key, ceiling FROM proxy_limits ORDER BY key")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The backoff the Politeness file holds for the domain key `key`, if any.
fn backoff(file: &Path, key: &str) -> Option<i64> {
    let connection = Connection::open(file).unwrap();
    let mut statement = connection
        .prepare_cached("SELECT backoff FROM domain_limits WHERE key = ?1")
        .unwrap();
    statement
        .query_row([key], |row| row.get::<_, Option<i64>>(0))
        .optional()
        .unwrap()
        .flatten()
}

/// Crawls `pages` through `proxy` with the username `username`, two
/// requests at once on the site, no retry, and `limits` as the Run's
/// `politeness.proxy`, the handler committing nothing, until the Run
/// finished.
fn crawled(
    proxy: Proxy,
    username: &str,
    pages: fn(&ProvingGround) -> Vec<String>,
    limits: &Value,
) -> (String, Met) {
    let env = TestEnv::new();
    // A governor under pressure lowers its HTTP target towards its floor,
    // one by default (ADR 0040), and would keep the second request back.
    env.user_configuration("governor.http.floor = 2\ngovernor.http.ceiling = 2\n");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let server = format!(
        "http://{}",
        proving_ground.description().fixtures.proxies[proxy.name()]
    );
    let mut run = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "crawl": { "retry": { "max": 0 }, "handler": { "concurrency": 2 } },
            "politeness": { "domain": { "concurrency": 2, "delay": 0 }, "proxy": limits },
            "site_preferences": { "respect_robots_txt": false },
        }),
    );
    run["proxy"] = json!({ "server": server, "username": username });
    let seeds: Vec<Value> = pages(&proving_ground)
        .into_iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect();
    let mut engine = crate::attached(&env);

    finished(
        &mut engine,
        &json!({
            "run_id": RUN_ID, "seeds": seeds, "labels": { "page": {} }, "settings": run,
        }),
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": RUN_ID }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let mut ended: Vec<(Value, Value)> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {outcomes}"))
        .iter()
        .map(|outcome| (outcome["kind"].clone(), outcome["error"]["code"].clone()))
        .collect();
    ended.sort_by_key(|(kind, code)| format!("{kind}{code}"));
    let file = env.home().join("politeness.sqlite");
    (
        format!("proxy {server} {username}"),
        (ended, ceilings(&file), backoff(&file, "scope.test")),
    )
}

/// Starts the crawl `params` and handles each page, committing nothing,
/// until the Run finished.
fn finished(engine: &mut Attached, params: &Value) {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": params,
    }));
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["method"] == "crawl_request.handle" {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
            }));
        } else if message["id"] == "start" {
            assert_eq!(message["result"]["run_id"], RUN_ID, "{message}");
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": RUN_ID },
            }));
        } else if message["id"] == "finish" {
            return;
        }
    }
}

/// `politeness.proxy.concurrency` and `politeness.proxy.rate` hold the
/// hops through one proxy endpoint and credential: two pages the site
/// admits at once go through the `limited` proxy one after the other, so
/// both succeed and the proxy taught no ceiling.
///
/// Covers: story:3-82, setting:proxy.server, setting:proxy.username
#[rstest]
/// Covers: setting:politeness.proxy.concurrency
#[case::one_at_a_time(json!({ "concurrency": 1 }))]
/// Covers: setting:politeness.proxy.rate
#[case::one_a_second(json!({ "rate": 1 }))]
fn limits_on_a_proxy_hold_the_hops_through_one_endpoint_and_credential(#[case] limits: Value) {
    let (key, met) = crawled(Proxy::Limited, "held", two_pages, &limits);

    let succeeded = (json!("succeeded"), Value::Null);
    assert_eq!(
        met,
        (
            vec![succeeded.clone(), succeeded],
            BTreeSet::from([(key, None)]),
            None,
        )
    );
}

/// The proxy's own `429`, past the one request the `limited` proxy relays
/// at a time, and its own `502`, for a site it cannot connect to, each fail
/// the hop with `proxy_unavailable`, never `blocked`, lower the ceiling
/// learned for the proxy endpoint and credential to one, and back off no
/// domain.
///
/// Covers: story:3-82, setting:proxy.server, error:proxy_unavailable
#[rstest]
#[case::its_own_429(Proxy::Limited, two_pages, &[("failed", "proxy_unavailable"), ("succeeded", "")])]
#[case::its_own_502(Proxy::Open, refused, &[("failed", "proxy_unavailable")])]
fn a_proxy_refusing_lowers_its_learned_ceiling_and_is_never_a_block_cause(
    #[case] proxy: Proxy,
    #[case] pages: fn(&ProvingGround) -> Vec<String>,
    #[case] ended: &[(&str, &str)],
) {
    let (key, met) = crawled(proxy, "refused", pages, &json!({}));

    let ended: Vec<(Value, Value)> = ended
        .iter()
        .map(|(kind, code)| {
            let code = if code.is_empty() {
                Value::Null
            } else {
                json!(code)
            };
            (json!(kind), code)
        })
        .collect();
    assert_eq!(met, (ended, BTreeSet::from([(key, Some(1))]), None));
}
