use std::collections::HashMap;

use serde_json::{Value, json};

use super::{MARKER, Site, observe};
use crate::compare::trials::{LatencyEnd, Percentiles};

/// A request the Proving ground recorded on `scale.test`, arrived and
/// answered at these microseconds of its clock.
fn request(connection: u64, target: &str, status: u16, arrived: u64, answered: u64) -> Value {
    json!({
        "host": "scale.test:4000",
        "connection": connection,
        "method": "GET",
        "target": target,
        "status": status,
        "tls": null,
        "timing": { "arrived_microseconds": arrived, "answered_microseconds": answered },
        "bytes": { "received": 100, "sent": 600 },
    })
}

/// A trial of the 4-page site (page 1 links pages 2 to 4) whose marker
/// came at 1 s: robots.txt asked first and not found, the four pages
/// answered at 1.25, 1.55, 1.65 and 1.75 s, and page 3 fetched twice, on two
/// connections besides the marker's.
fn observations() -> Value {
    json!({
        "requests": [
            request(1, MARKER, 404, 1_000_000, 1_000_100),
            request(2, "/robots.txt", 404, 1_050_000, 1_060_000),
            request(2, "/sites/4/1", 200, 1_100_000, 1_250_000),
            request(2, "/sites/4/2", 200, 1_400_000, 1_550_000),
            request(3, "/sites/4/3", 200, 1_500_000, 1_650_000),
            request(3, "/sites/4/4", 200, 1_600_000, 1_750_000),
            request(3, "/sites/4/3", 200, 1_800_000, 1_900_000),
        ],
        "unrecorded_requests": 0,
        "hosts": { "scale.test": { "received": 700, "sent": 4200 } },
    })
}

const SITE: Site<'static> = Site {
    hosts: &["scale.test"],
    pages: 4,
    tls: false,
};

#[test]
fn a_trials_work_network_and_times_come_from_the_proving_grounds_observations() {
    let observed = observe(&observations(), &SITE, 1.0, None).unwrap();

    assert_eq!(
        (
            observed.work.pages,
            observed.work.missing.len(),
            observed.work.requests,
            observed.work.bytes,
            observed.work.first_bytes,
        ),
        (4, 0, 5, 3000, 2400)
    );
    assert_eq!(
        (
            observed.requests_sent,
            observed.duplicates,
            observed.errors,
            observed.sent_bytes,
            observed.received_bytes,
            observed.connections,
            observed.tls_connections,
        ),
        (6, 1, 1, 600, 3600, 2, 0)
    );
    assert!((observed.startup_seconds - 0.05).abs() < 1e-9);
    assert!((observed.first_page_seconds - 0.25).abs() < 1e-9);
    assert_eq!(observed.first_record_seconds, None);
    assert_eq!(observed.latency_end, Some(LatencyEnd::Fetch));
}

/// Each page's time runs from its parent's answer to its own: 0.25, 0.3,
/// 0.4 and 0.5 s, whose nearest-rank p50 is 0.3 and p95 and p99 0.5.
#[test]
fn a_pages_latency_runs_from_its_parents_answer_to_its_own() {
    let observed = observe(&observations(), &SITE, 1.0, None).unwrap();

    assert_percentiles(observed.latency, [0.3, 0.5, 0.5]);
}

/// With Records, a page's time runs to its Record's commit instead.
#[test]
fn a_tool_with_records_times_each_page_to_its_record() {
    let records: HashMap<String, f64> = (1..=4)
        .map(|page| (format!("/sites/4/{page}"), 1.0))
        .collect();

    let observed = observe(&observations(), &SITE, 1.0, Some(&records)).unwrap();

    // 1.0 s less each parent's answer: 1.0, then 0.75 three times.
    assert_percentiles(observed.latency, [0.75, 1.0, 1.0]);
    assert_eq!(observed.first_record_seconds, Some(1.0));
    assert_eq!(observed.latency_end, Some(LatencyEnd::Record));
}

/// The pages answered in each tenth of the trial: one in each of its
/// third, sixth, seventh and eighth tenths of 0.1 s.
#[test]
fn the_curve_counts_each_tenths_pages_per_second() {
    let observed = observe(&observations(), &SITE, 1.0, None).unwrap();

    let rounded: Vec<f64> = observed.curve.iter().map(|rate| rate.round()).collect();
    assert_eq!(
        rounded,
        vec![0.0, 0.0, 10.0, 0.0, 0.0, 10.0, 10.0, 10.0, 0.0, 0.0]
    );
}

#[test]
fn a_page_never_answered_is_missing() {
    let mut observations = observations();
    observations["requests"].as_array_mut().unwrap().remove(5);

    let observed = observe(&observations, &SITE, 1.0, None).unwrap();

    assert_eq!(observed.work.missing, vec!["/sites/4/4".to_owned()]);
}

/// The site's work is every `GET` of its pages answered `200` on its host,
/// named with or without its port; a `HEAD` and a page answered `404` are
/// none of it.
#[test]
fn a_trials_work_is_its_answered_page_gets_on_the_sites_host() {
    let mut observations = observations();
    let requests = observations["requests"].as_array_mut().unwrap();
    let mut head = request(3, "/sites/4/1", 200, 1_910_000, 1_920_000);
    head["method"] = json!("HEAD");
    let mut portless = request(3, "/sites/4/2", 200, 1_930_000, 1_940_000);
    portless["host"] = json!("scale.test");
    requests.extend([
        head,
        request(3, "/sites/4/5", 404, 1_950_000, 1_960_000),
        portless,
    ]);

    let observed = observe(&observations, &SITE, 1.0, None).unwrap();

    assert_eq!(
        (
            observed.work.pages,
            observed.work.extra.len(),
            observed.work.requests,
            observed.work.bytes,
        ),
        (4, 0, 6, 3600)
    );
}

/// A redirect the site answers under its pages' path, such as the
/// realistic site's `/sites/<n>/moved/<page>`, is work the tool did, its
/// request and bytes counted, but no page, no extra page and no error: its
/// page is the one it leads to. Asked for again, it is a duplicate.
#[test]
fn a_redirect_on_the_site_is_work_but_no_page_and_no_error() {
    let mut observations = observations();
    let requests = observations["requests"].as_array_mut().unwrap();
    requests.push(request(3, "/sites/4/moved/4", 301, 1_910_000, 1_920_000));
    requests.push(request(3, "/sites/4/moved/4", 301, 1_930_000, 1_940_000));

    let observed = observe(&observations, &SITE, 1.0, None).unwrap();

    assert_eq!(
        (
            observed.work.pages,
            observed.work.extra.len(),
            observed.work.requests,
            observed.work.bytes,
            observed.errors,
            observed.duplicates,
        ),
        (4, 0, 7, 4200, 1, 2)
    );
}

#[test]
fn unrecorded_requests_measure_nothing() {
    let mut observations = observations();
    observations["unrecorded_requests"] = json!(1);

    assert!(observe(&observations, &SITE, 1.0, None).is_err());
}

/// A trial of a site on two hosts, each holding two of its four pages:
/// both hosts' requests are its work and their bytes its network's, and a
/// request to another host is none of the trial's.
#[test]
fn a_site_on_several_hosts_is_observed_on_each_of_them() {
    let on = |host: &str, mut request: Value| {
        request["host"] = json!(format!("{host}:4000"));
        request
    };
    let observations = json!({
        "requests": [
            on("polite-1.localhost", request(1, MARKER, 404, 1_000_000, 1_000_100)),
            on("polite-1.localhost", request(2, "/sites/4/1", 200, 1_100_000, 1_250_000)),
            on("polite-2.localhost", request(3, "/sites/4/2", 200, 1_400_000, 1_550_000)),
            on("polite-1.localhost", request(2, "/sites/4/3", 200, 1_500_000, 1_650_000)),
            on("polite-2.localhost", request(3, "/sites/4/4", 200, 1_600_000, 1_750_000)),
            on("elsewhere.test", request(4, "/sites/4/2", 200, 1_700_000, 1_800_000)),
        ],
        "unrecorded_requests": 0,
        "hosts": {
            "polite-1.localhost": { "received": 300, "sent": 1800 },
            "polite-2.localhost": { "received": 200, "sent": 1200 },
            "elsewhere.test": { "received": 100, "sent": 600 },
        },
    });
    let site = Site {
        hosts: &["polite-1.localhost", "polite-2.localhost"],
        pages: 4,
        tls: false,
    };

    let observed = observe(&observations, &site, 1.0, None).unwrap();

    assert_eq!(
        (
            observed.work.pages,
            observed.work.requests,
            observed.duplicates,
            observed.requests_sent,
            observed.sent_bytes,
            observed.received_bytes,
            observed.connections,
        ),
        (4, 4, 0, 4, 400, 2400, 2)
    );
}

fn assert_percentiles(latency: Percentiles, expected: [f64; 3]) {
    let got = [latency.p50, latency.p95, latency.p99];
    assert!(
        got.iter()
            .zip(expected)
            .all(|(got, expected)| (got - expected).abs() < 1e-9),
        "{got:?} against {expected:?}"
    );
}

/// A request the Proving ground recorded on `article.localhost` over
/// `protocol`, with a TLS handshake where `tls`: 500 bytes of body sent in
/// 640 framed ones.
fn over(connection: u64, target: &str, protocol: &str, tls: bool, arrived: u64) -> Value {
    json!({
        "host": "article.localhost:4000",
        "connection": connection,
        "protocol": protocol,
        "method": "GET",
        "target": target,
        "status": 200,
        "tls": if tls { json!({ "sni": "article.localhost" }) } else { Value::Null },
        "timing": { "arrived_microseconds": arrived, "answered_microseconds": arrived + 100 },
        "bytes": { "received": 90, "sent": 640 },
        "body": { "received": 0, "sent": 500 },
    })
}

/// Over TLS and HTTP/2 (#2284), a trial's work counts each answer's body
/// bytes, which no client's framing or header compression changes, and
/// counts apart each request for the site's pages that came over anything
/// else: here page 3, over plain HTTP/1.1. The marker, the harness's own
/// request over plain HTTP, is none of them.
#[test]
fn over_tls_a_trials_work_counts_body_bytes_and_requests_over_another_protocol() {
    let observations = json!({
        "requests": [
            over(1, MARKER, "http/1.1", false, 1_000_000),
            over(2, "/sites/4/1", "h2", true, 1_100_000),
            over(2, "/sites/4/2", "h2", true, 1_200_000),
            over(3, "/sites/4/3", "http/1.1", false, 1_300_000),
            over(2, "/sites/4/4", "h2", true, 1_400_000),
        ],
        "unrecorded_requests": 0,
        "hosts": { "article.localhost": { "received": 450, "sent": 3200 } },
    });
    let site = Site {
        hosts: &["article.localhost"],
        pages: 4,
        tls: true,
    };
    let loopback = Site { tls: false, ..site };

    let observed = observe(&observations, &site, 1.0, None).unwrap();
    let on_loopback = observe(&observations, &loopback, 1.0, None).unwrap();

    assert_eq!(
        (
            observed.work.pages,
            observed.work.requests,
            observed.work.bytes,
            observed.work.first_bytes,
            observed.work.other_protocol,
            observed.tls_connections,
            on_loopback.work.bytes,
            on_loopback.work.other_protocol,
        ),
        (4, 4, 2000, 2000, 1, 1, 2560, 0)
    );
}
