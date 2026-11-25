//! J-042, "Drive one page and see nothing leave the machine beyond its site
//! and pinned downloads", on the `cli` surface (ADRs 0043 and 0063): a
//! person drives one product page of the catalog through Collie's shipped
//! executable, on a Browser cache that holds no browser and with nothing
//! configured to leave the machine, so the first page downloads the pinned
//! Chrome for Testing build from the Proving ground's mirror. The Proving
//! ground's record then holds the page's requests to its site and that
//! download, and nothing else: no telemetry, no update check and no
//! browser background traffic. A connection to any other host would leave
//! the Proving ground's record, which CI's outbound deny refuses and each
//! test job's `check` step fails on (ADR 0043).

use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{
    BROWSER_BUILDS_HOST, CHROME_FOR_TESTING_BUILD, JourneyRun, ReceivedRequests,
};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's task whose product page the person reads.
const TASK: &str = "catalog/2";

/// The fields the person reads of the product page, as `page read` takes
/// them: the product's ID, name and price, as task `catalog/2` names them.
const FIELDS: &str = r#"{"id":{"locator":{"kind":"css","selector":"span.id"}},"name":{"locator":{"kind":"css","selector":"h1.name"}},"price":{"locator":{"kind":"css","selector":"span.price"},"attribute":"data-price","transform":{"kind":"number","locale":"en"}}}"#;

/// Stops the Daemon `run start` started when the test ends, whether it
/// passed or not, by running its `daemon stop`, built before the run's
/// steps borrow the run.
struct StopsTheDaemon<F: FnMut()>(F);

impl<F: FnMut()> Drop for StopsTheDaemon<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// What the executable answered `args` with `--json`, which it must
/// answer successfully.
fn answer<C>(run: &JourneyRun<C>, args: &[&str]) -> Value {
    let output = run.command().args(args).arg("--json").output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{args:?}: {output:?}"))
}

/// The host a page URL such as `http://2.catalog.test:51234/products/P-1`
/// names, without its port.
fn host_of(url: &str) -> &str {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = authority.split('/').next().unwrap_or(authority);
    authority.split(':').next().unwrap_or(authority)
}

/// Each request to the browser builds' mirror, as a test reads it: its
/// headers by name, since the claim is on which it sends, not their order.
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

/// The plain GET of the pinned Chrome for Testing build ADR 0043 allows,
/// over HTTPS to the mirror on `port`: the pinned path,
/// `<build>/<platform>/chrome-<platform>.zip`, and nothing about the
/// install but the user agent every install sends. The platform is the one
/// `target` names, which must be one Chrome for Testing builds for.
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

/// Everything the Proving ground's record holds beyond the page's
/// requests to `site` and the downloads from the mirror: each request to
/// another host, each connection a listener other than the sites' accepted,
/// each relay, DNS query and UDP datagram, and each entry past the record's
/// bound.
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
        "J-042",
        "cli",
        Path::new(COLLIE),
        ResultDocument::journey,
    )
    .expect("J-042's cli variant starts");
    let proving_ground = run.proving_ground().expect("J-042 has a Proving ground");
    let product = proving_ground.start_url(TASK).unwrap();
    let port = proving_ground.address().port();
    let mut stop = run.command();
    stop.args(["daemon", "stop"]);
    let _stops = StopsTheDaemon(move || {
        let _ = stop.output();
    });
    let mut run_id = String::new();
    let mut page_id = String::new();
    let mut read = Value::Null;
    let mut observations = Value::Null;

    run.step(
        "Start a run with nothing configured to leave the machine, on a Browser cache that holds no browser",
        |run| {
            let started = answer(run, &["run", "start"]);
            started["run_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{started}"))
                .clone_into(&mut run_id);
        },
    );
    run.step(
        "Open a page, whose browser is downloaded on first use",
        |run| {
            let opened = answer(run, &["run", "new-page", &run_id]);
            opened["page_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{opened}"))
                .clone_into(&mut page_id);
        },
    );
    run.step("Go to a product page of the site", |run| {
        answer(run, &["page", "goto", &page_id, "--url", &product]);
    });
    run.step(
        "Read the product's ID, name and price from the page",
        |run| {
            read = answer(run, &["page", "read", &page_id, "--fields", FIELDS]);
        },
    );
    run.step("Finish the run", |run| {
        answer(run, &["run", "finish", &run_id]);
        observations = run
            .proving_ground()
            .expect("J-042 has a Proving ground")
            .observations()
            .unwrap();
    });

    run.expect_task(
        "The oracle verifies the product the page's fields answer",
        &json!({ "status": "success", "records": [read["values"]] }),
    );
    run.check(
        "The browser build was downloaded as one plain GET of its pinned path, with no cookie, no query and no header but its host and the user agent `goodfetch-download`",
        |_| {
            let downloads = downloads(&observations);
            let target = downloads
                .first()
                .and_then(|download| download["target"].as_str())
                .unwrap_or_default();
            assert_eq!(downloads, vec![plain_get(target, port)]);
        },
    );
    run.check(
        "Every request the Proving ground received went to the product page's host or was that download: no telemetry, no update check and no browser background traffic",
        |_| {
            assert_eq!(
                beyond_the_site(&observations, host_of(&product)),
                Vec::<Value>::new()
            );
        },
    );
    run.finish()
        .expect("J-042's document is written")
        .assert_proved();
}
