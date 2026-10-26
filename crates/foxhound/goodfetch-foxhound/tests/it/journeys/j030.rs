//! J-030, "Send a crawl's traffic from a chosen local address and see the
//! site receive it from there", on the `cli` surface (ADRs 0044 and 0063;
//! ticket #682): a person's program starts Foxhound's shipped executable as
//! its engine and crawls three pages of the Proving ground's site over
//! HTTP, choosing two of the extra loopback addresses (`127.0.0.2` and
//! `127.0.0.4`, TESTING.md) for the Crawl's traffic; the Proving ground's
//! connection record names the address each request came from.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, ReceivedRequests};
use serde_json::{Map, Value, json};

use crate::crawl::Attached;
use crate::frontier::crawl_with;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The two local addresses the person chooses, in different `/24`s' stead:
/// `/30` blocks, so the Crawl's Sessions spread over both.
const CHOSEN: [&str; 2] = ["127.0.0.2", "127.0.0.4"];

/// The pages the Crawl fetches.
const PAGES: [&str; 3] = ["/consistency/1", "/consistency/2", "/version"];

/// The source address of the connection of each request to `access.test`,
/// robots.txt's included: the check claims where every request left from,
/// once the Crawl finished, so after its robots.txt reading.
fn sources(observations: &Value) -> Vec<IpAddr> {
    let connections = observations["connections"].as_array().unwrap();
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|host| host.starts_with("access.test"))
        })
        .map(|request| {
            let connection = connections
                .iter()
                .find(|connection| connection["id"] == request["connection"])
                .unwrap_or_else(|| panic!("no connection for {request}"));
            connection["source"]
                .as_str()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
        })
        .collect()
}

/// Covers: story:4-44, story:4-108, row:SA-25
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-030", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-030's cli variant starts");
    let mut engine: Option<Attached> = None;
    let mut report = Value::Null;
    let mut targets: Vec<String> = Vec::new();

    run.step("Start Foxhound's executable as a program's engine", |run| {
        let mut command = run.env().process_command(run.engine());
        command.arg("attached-engine");
        engine = Some(Attached::spawned(run.env(), None, &mut command));
    });
    run.step(
        "Crawl three pages of a site over HTTP, choosing two of the machine's local addresses for the Crawl's traffic",
        |run| {
            let proving_ground = run.proving_ground().expect("J-030 has a Proving ground");
            let resolve: Map<String, Value> = proving_ground
                .description()
                .resolve
                .iter()
                .map(|(host, address)| (host.clone(), json!([address])))
                .collect();
            let settings = json!({
                "fetch": { "tier": "http" },
                "network": { "resolve": resolve },
                "egress": {
                    "private_ranges": resolve.keys().collect::<Vec<_>>(),
                    "local_addresses": CHOSEN,
                    "rotation_block": { "ipv4": 30 },
                },
            });
            let seeds: Vec<Value> = PAGES
                .iter()
                .map(|path| json!({ "url": proving_ground.url("access.test", path), "label": "page" }))
                .collect();
            let engine = engine.as_mut().expect("the engine runs");
            let finished = crawl_with(engine, "from-chosen", &seeds, &settings, &mut |_, handle| {
                let url = handle["crawl_request"]["url"].clone();
                targets.push(url.to_string());
                json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] } })
            });
            report = finished["result"].clone();
            assert!(!report.is_null(), "{finished}");
        },
    );
    run.step("Let the Crawl finish and read its report", |_| {
        let engine = engine.take().expect("the engine runs");
        assert!(engine.finish());
    });

    run.check("The Crawl finished", |_| {
        assert_eq!(
            (report["lifecycle"].clone(), targets.len()),
            (json!("finished"), PAGES.len()),
            "{report}"
        );
    });
    run.check(
        "The site received every request of the Crawl, robots.txt's included, from one of the two chosen addresses, each address serving at least one, and none from another address",
        |run| {
            let proving_ground = run.proving_ground().expect("J-030 has a Proving ground");
            let observations = proving_ground.observations().unwrap();
            let seen = sources(&observations);
            let chosen: BTreeSet<IpAddr> = CHOSEN.iter().map(|address| address.parse().unwrap()).collect();
            assert_eq!(
                (
                    seen.len() > PAGES.len(),
                    seen.iter().copied().collect::<BTreeSet<_>>(),
                ),
                (true, chosen),
                "{seen:?}"
            );
        },
    );
    run.finish()
        .expect("J-030's document is written")
        .assert_proved();
}
