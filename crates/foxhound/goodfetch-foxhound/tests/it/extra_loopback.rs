//! Tests that need the extra loopback addresses `127.0.0.2` to `127.0.0.9`
//! (TESTING.md, Storage, network and OS services): a host without them skips
//! this module.
//!
//! A crawl's local addresses on Foxhound's executable over stdio, against
//! the Proving ground (spec #565's stories 108 and 109; ADR 0044): every
//! request of a crawl on the `http` tier reaches the site from one of
//! `egress.local_addresses`, each Session from its own; a Session a block
//! retired is replaced by one outside every `/30` block blocked on its
//! domain, and, with every block blocked, by one within a block, which the
//! Run report warns of. A replacement serving its domain on the browser
//! tier binds its relay to the pool's address too. (A Run that sets a proxy
//! beside the `http` tier fails its start until #1640 sends that tier
//! through the proxy: `pool_proxies`.)
//! Until the engine detects blocks itself (area 5), the test's handler
//! names the Block cause of the pages it is told to.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use goodfetch_test_support::ObservedRequest;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, ReceivedRequests, TestEnv, browser_cache,
};
use serde_json::{Value, json};

use crate::attached;
use crate::attached_with;
use crate::frontier::{crawl_declaring, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 682;

/// The settings of a crawl on the `http` tier sending from `addresses`,
/// spread over `/30` blocks.
fn settings(proving_ground: &ProvingGround, addresses: &[&str]) -> Value {
    json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { "access.test": [proving_ground.address().ip().to_string()] } },
        "egress": {
            "private_ranges": ["access.test"],
            "local_addresses": addresses,
            "rotation_block": { "ipv4": 30 },
        },
    })
}

/// The source address of the connection each request `chosen` keeps came
/// on, in arrival order. Every request, robots.txt's included: the tests
/// claim where it leaves from, and read the observations once the crawl
/// finished, so after its robots.txt reading, which its pages waited on.
fn sources(observations: &Value, chosen: impl Fn(&ObservedRequest) -> bool) -> Vec<IpAddr> {
    let connections = observations["connections"].as_array().unwrap();
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| chosen(request))
        .map(|request| {
            let connection = connections
                .iter()
                .find(|connection| connection["id"] == request.connection)
                .unwrap_or_else(|| panic!("no connection for {request:?}"));
            connection["source"]
                .as_str()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
        })
        .collect()
}

/// The source address of the connection each request for the target
/// `target` came on, in arrival order.
fn sources_of(observations: &Value, target: &str) -> Vec<IpAddr> {
    sources(observations, |request| request.target == target)
}

/// Every source address a request to the crawled site came from: the
/// test's own reading of the observations, from `127.0.0.1`, is no
/// request to it.
fn every_source(observations: &Value) -> BTreeSet<IpAddr> {
    sources(observations, |request| {
        request.host.starts_with("access.test")
    })
    .into_iter()
    .collect()
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

/// A handler's answer: the Error code `blocked` with `ip_block` for a page
/// whose path is in `blocked`, else a Record naming the page.
fn answer(params: &Value, blocked: &[&str]) -> Value {
    let url = params["crawl_request"]["url"].as_str().unwrap();
    if blocked.iter().any(|path| url.ends_with(path)) {
        json!({ "error": {
            "code": -32003,
            "message": "the page failed",
            "data": { "code": "blocked", "retryable": false, "cause": "ip_block" },
        } })
    } else {
        json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] } })
    }
}

/// Without a block, each Session sends from the next address in turn
/// across the blocks, robots.txt from the first, and nothing reaches the
/// site from another address.
///
/// Covers: story:4-108, row:SA-25, story:4-44
#[test]
fn a_crawl_reaches_the_site_from_its_sessions_local_addresses() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let pages = ["/consistency/1", "/consistency/2", "/version"];
    let seeds: Vec<Value> = pages
        .iter()
        .map(|path| json!({ "url": proving_ground.url("access.test", path), "label": "page" }))
        .collect();
    let mut engine = attached(&env);

    let report = crawl_with(
        &mut engine,
        "spread",
        &seeds,
        &settings(&proving_ground, &["127.0.0.2", "127.0.0.4"]),
        &mut |_, handle| answer(handle, &[]),
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    assert_eq!(
        (
            report["result"]["lifecycle"].clone(),
            sources_of(&observations, "/robots.txt"),
            pages.map(|page| sources_of(&observations, page)),
            every_source(&observations),
        ),
        (
            json!("finished"),
            vec![ip("127.0.0.2")],
            [
                vec![ip("127.0.0.2")],
                vec![ip("127.0.0.4")],
                vec![ip("127.0.0.2")],
            ],
            BTreeSet::from([ip("127.0.0.2"), ip("127.0.0.4")]),
        ),
        "{report}"
    );
}

/// A block retires its Session, and the next Session opened on the domain
/// skips the address in the blocked `/30` that round robin would give it;
/// with every block blocked, the replacement rotates within one, and the
/// Run report warns of it, naming the Session.
///
/// Covers: story:4-109
#[test]
fn a_replacement_after_a_block_reaches_the_site_from_an_unblocked_block() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = |pages: &[&str]| -> Vec<Value> {
        pages
            .iter()
            .map(|path| json!({ "url": proving_ground.url("access.test", path), "label": "page" }))
            .collect()
    };
    let mut engine = attached(&env);

    let outside = crawl_with(
        &mut engine,
        "outside",
        &seeds(&["/consistency/1", "/consistency/2", "/version"]),
        &settings(&proving_ground, &["127.0.0.2", "127.0.0.3", "127.0.0.4"]),
        &mut |_, handle| answer(handle, &["/consistency/1"]),
    );
    let first = proving_ground.observations().unwrap();
    let within = crawl_with(
        &mut engine,
        "within",
        &seeds(&["/probe", "/cdp-probe", "/identity"]),
        &settings(&proving_ground, &["127.0.0.5", "127.0.0.8"]),
        &mut |_, handle| answer(handle, &["/probe", "/cdp-probe"]),
    );
    let second = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let warnings: Vec<Value> = within["result"]["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|warning| warning["code"] == "local_address_rotated_within_block")
        .map(|warning| warning["sessions"].clone())
        .collect();
    assert_eq!(
        (
            ["/consistency/1", "/consistency/2", "/version"].map(|page| sources_of(&first, page)),
            outside["result"]["warnings"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|warning| warning["code"] == "local_address_rotated_within_block"),
            ["/probe", "/cdp-probe", "/identity"].map(|page| sources_of(&second, page)),
            warnings,
        ),
        (
            [
                vec![ip("127.0.0.2")],
                vec![ip("127.0.0.4")],
                vec![ip("127.0.0.4")],
            ],
            false,
            [
                vec![ip("127.0.0.5")],
                vec![ip("127.0.0.8")],
                vec![ip("127.0.0.5")],
            ],
            vec![json!(["session-3"])],
        ),
        "{within}"
    );
}

/// A replacement that serves its domain on the browser tier, after a block
/// its Session met on the `http` tier, binds its browser's relay to the
/// address the pool gave it, outside the blocked `/30`, not to the one its
/// browser context's own count would draw (ADR 0044; #682).
///
/// Covers: story:4-109
#[test]
fn a_replacement_on_the_browser_tier_reaches_the_site_from_the_pools_unblocked_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let seeds = [
        json!({ "url": proving_ground.url("access.test", "/consistency/1"), "label": "http" }),
        json!({ "url": proving_ground.url("access.test", "/corpus/article?n=1"), "label": "browser" }),
    ];
    let mut settings = settings(&proving_ground, &["127.0.0.2", "127.0.0.5"]);
    settings["fetch"] = json!({});
    settings["session"] = json!({ "pool": { "max_size": 1 } });

    let report = crawl_declaring(
        &mut engine,
        "browser-replacement",
        &seeds,
        &json!({
            "http": { "settings": { "fetch": { "tier": "http" } } },
            "browser": { "settings": { "fetch": { "tier": "browser" } } },
        }),
        &settings,
        &mut |_, handle| answer(handle, &["/consistency/1"]),
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            ["/consistency/1", "/corpus/article?n=1"].map(|page| sources_of(&observations, page)),
        ),
        (
            &json!("finished"),
            [vec![ip("127.0.0.2")], vec![ip("127.0.0.5")]],
        ),
        "{report}"
    );
}
