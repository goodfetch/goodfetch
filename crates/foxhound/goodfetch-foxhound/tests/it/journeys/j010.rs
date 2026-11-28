//! J-010, "Run a Crawl and see nothing leave the machine beyond its targets
//! and pinned downloads", on the `cli` surface (ADRs 0043 and 0063; spec
//! #565, ticket #687): a person crawls the Proving ground's `catalog/1`
//! through Foxhound's shipped executable with default settings, on an empty
//! Browser cache whose mirror is the Proving ground's. The Crawl returns
//! every product of the catalog, verified by the oracle, and the Proving
//! ground's connection record holds only requests to the catalog's host and
//! any digest-pinned download from the mirror — no telemetry, no update
//! check, and no browser background traffic.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the fields file and read the result document outside any async worker"
)]

use std::path::Path;

use goodfetch_protocol::journey::Journey;
use goodfetch_protocol::result_document::{
    LoadedDocument, ResultDocument, read, read_journey_document,
};
use goodfetch_test_support::{
    BROWSER_BUILDS_HOST, CHROME_FOR_TESTING_BUILD, JourneyRun, ReceivedRequests,
    cli as command_line,
};
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The crawl's Run ID.
const RUN: &str = "j010-catalog";

/// The Proving ground task J-010 crawls.
const TASK: &str = "catalog/1";

/// The product fields read from each catalog product page.
const PRODUCT_FIELDS: [&str; 3] = ["id", "name", "price"];

/// Stops the Daemon when the test ends, whether it passed or not.
struct StopsTheDaemon<F: FnMut()>(F);

impl<F: FnMut()> Drop for StopsTheDaemon<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// The named fields the crawl reads from each page beside its Markdown.
fn fields_read() -> Value {
    json!({
        "id": { "locator": { "kind": "css", "selector": "article.product .id" } },
        "name": { "locator": { "kind": "css", "selector": "article.product .name" } },
        "price": {
            "locator": { "kind": "css", "selector": "article.product .price" },
            "attribute": "data-price",
        },
    })
}

/// The host a URL names, without its port.
fn host_of(url: &str) -> &str {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = authority.split('/').next().unwrap_or(authority);
    authority.split(':').next().unwrap_or(authority)
}

/// Each request to the browser builds' mirror, with sorted headers.
fn downloads(observations: &Value) -> Vec<Value> {
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| request.host == BROWSER_BUILDS_HOST)
        .map(|request| {
            let mut headers = request.headers.clone();
            headers.sort();
            json!({
                "method": request.method,
                "target": request.target,
                "over_tls": request.tls.is_some(),
                "headers": headers,
                "cookies": request.cookies,
            })
        })
        .collect()
}

/// The plain HTTPS GET of the pinned Chrome for Testing build ADR 0043 allows.
fn plain_get(target: &str, port: u16) -> Value {
    let platform = ["linux64", "mac-arm64", "mac-x64", "win64"]
        .into_iter()
        .find(|platform| target.contains(&format!("/{platform}/")))
        .unwrap_or("<a Chrome for Testing platform>");
    json!({
        "method": "GET",
        "target": format!("/{CHROME_FOR_TESTING_BUILD}/{platform}/chrome-{platform}.zip"),
        "over_tls": true,
        "headers": [
            ["host", format!("{BROWSER_BUILDS_HOST}:{port}")],
            ["user-agent", "goodfetch-download"],
        ],
        "cookies": [],
    })
}

/// Every entry in the Proving ground's record beyond the catalog host and
/// pinned browser downloads.
fn beyond_the_site(observations: &Value, site: &str) -> Vec<Value> {
    let requests = ReceivedRequests::of(observations)
        .every()
        .filter(|request| request.host != site && request.host != BROWSER_BUILDS_HOST)
        .map(|request| json!(["request", request.host, request.target]))
        .collect::<Vec<_>>();
    let connections = observations["connections"]
        .as_array()
        .unwrap_or_else(|| panic!("{observations}"))
        .iter()
        .filter(|connection| connection["listener"] != "site")
        .map(|connection| json!(["connection", connection]));
    let others = ["relays", "dns_queries", "udp_datagrams"]
        .into_iter()
        .flat_map(|part| {
            observations[part]
                .as_array()
                .unwrap_or_else(|| panic!("{part}: {observations}"))
                .iter()
                .map(move |entry| json!([part, entry]))
        });
    let unrecorded = [
        "unrecorded_requests",
        "unrecorded_connections",
        "unrecorded_relays",
        "unrecorded_dns_queries",
        "unrecorded_udp_datagrams",
    ]
    .into_iter()
    .filter(|count| observations[*count] != 0)
    .map(|count| json!([count, observations[count]]));
    requests
        .into_iter()
        .chain(connections)
        .chain(others)
        .chain(unrecorded)
        .collect()
}

/// Covers: story:4-80, story:4-81, row:SC-10
#[test]
fn cli() {
    let mut run = JourneyRun::start_downloading_browser(
        "J-010",
        "cli",
        Path::new(FOXHOUND),
        ResultDocument::journey,
    )
    .expect("J-010's cli variant starts");
    let proving_ground = run.proving_ground().expect("J-010 has a Proving ground");
    let start = proving_ground.start_url(TASK).unwrap();
    let port = proving_ground.address().port();
    let mut stop = run.command();
    stop.args(["daemon", "stop"]);
    let _stops = StopsTheDaemon(move || {
        let _ = stop.output();
    });
    let mut committed: Vec<Value> = Vec::new();
    let mut observations = Value::Null;

    run.step(
        "Crawl the catalog with Foxhound's default settings",
        |run| {
            let fields = run.env().folder().join("fields.json");
            std::fs::write(&fields, fields_read().to_string()).unwrap();
            let settings_path = run.env().folder().join("settings.json");
            std::fs::write(
                &settings_path,
                json!({ "politeness": { "domain": { "delay": 0 } } }).to_string(),
            )
            .unwrap();
            let started = command_line(run.env(), run.engine())
                .args(["crawl", "start", &start, "--run-id", RUN])
                .args(["--format", "markdown", "--links", "scope"])
                .args(["--fields", fields.to_str().unwrap()])
                .args(["--settings", settings_path.to_str().unwrap(), "--json"])
                .json("RunId")
                .unwrap();
            assert_eq!(started.documents[0], RUN, "{}", started.output);
        },
    );
    run.step("Wait for the Crawl to finish", |run| {
        let ended = command_line(run.env(), run.engine())
            .args(["run", "outcomes", RUN, "--follow", "--json"])
            .json_lines("CrawlRequestOutcome")
            .unwrap();
        assert!(!ended.documents.is_empty(), "{}", ended.output);
        let read_records = command_line(run.env(), run.engine())
            .args(["run", "records", RUN, "--dataset", "pages", "--json"])
            .json_array("RecordRead")
            .unwrap();
        committed = read_records.documents[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["record"].clone())
            .collect();
        observations = run
            .proving_ground()
            .expect("J-010 has a Proving ground")
            .observations()
            .unwrap();
    });

    let products: Vec<Value> = committed
        .iter()
        .filter(|record| record["value"]["id"].is_string())
        .map(|record| {
            PRODUCT_FIELDS
                .iter()
                .map(|field| ((*field).to_owned(), record["value"][*field].clone()))
                .collect::<serde_json::Map<String, Value>>()
                .into()
        })
        .collect();
    run.expect_task(
        "The oracle verifies the Crawl's Records",
        &json!({ "status": "success", "records": products }),
    );
    run.check(
        "Every connection Foxhound's processes opened went to the catalog's hosts or fetched a digest-pinned download, and none carried telemetry or an update check",
        |_| {
            assert_eq!(
                beyond_the_site(&observations, host_of(&start)),
                Vec::<Value>::new()
            );
            for download in downloads(&observations) {
                let target = download["target"].as_str().unwrap_or_default();
                assert_eq!(download, plain_get(target, port));
            }
        },
    );

    let finished = run.finish().expect("J-010's document is written");
    finished.assert_proved();
    let journey: Journey = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../journeys/J-010.toml"
    )))
    .unwrap();
    let bytes = std::fs::read(finished.path()).unwrap();
    let LoadedDocument::Journey(loaded) = read(&bytes).unwrap() else {
        panic!("J-010's document is a journey document");
    };
    read_journey_document(loaded, &journey).expect("the reader accepts J-010's document");
}
