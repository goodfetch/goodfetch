//! Network profiles through the user-space latency proxy, which any OS runs
//! without privileges (ADR 0025): `serve` calibrates the declared profile
//! before it prints its first line, which records the calibration, and then
//! serves every page through the proxy. What the calibration measured is
//! recorded, never asserted (ADR 0064); the proxy's delays themselves are
//! checked on Tokio's paused clock, in its module's tests.

use goodfetch_test_support::{LOOPBACK_PORT_0, serve_arguments};
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

fn shaped(seed: u64, profile: &str) -> ProvingGround {
    let mut arguments = serve_arguments(seed, LOOPBACK_PORT_0, profile).to_vec();
    arguments.extend(["--shaper".to_owned(), "proxy".to_owned()]);
    ProvingGround::start_with(&arguments)
}

fn network(proving_ground: &ProvingGround) -> Value {
    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");
    line["network"].clone()
}

/// Covers: story:2-44, story:2-45
#[test]
fn a_shaped_profile_is_calibrated_before_serve_reports_its_address() {
    let proving_ground = shaped(160, "rtt-20");

    let network = network(&proving_ground);

    assert_eq!(network["profile"], "rtt-20");
    assert_eq!(network["round_trip_microseconds"], 20_000);
    assert_eq!(network["loss_percent"], 0);
    assert_eq!(network["shaper"], "proxy");
    assert_eq!(network["calibration"]["round_trip_microseconds"], 20_000);
    assert_eq!(network["calibration"]["samples"], 50);
    assert!(
        ["available", "unavailable"].contains(&network["time_metrics"].as_str().unwrap()),
        "{network}"
    );
}

/// Covers: story:2-45
#[test]
fn the_latency_proxy_serves_the_pages_served_unshaped() {
    let unshaped = ProvingGround::start(160);
    let proving_ground = shaped(160, "rtt-0");

    let page = proving_ground.get("http://catalog.test/catalog/1");

    assert_eq!(page.status, 200);
    assert_eq!(
        page.body,
        unshaped.get("http://catalog.test/catalog/1").body
    );
}

/// Covers: story:2-45
#[test]
fn task_urls_behind_the_latency_proxy_name_the_port_it_reported() {
    let proving_ground = shaped(160, "rtt-0");

    let tasks = proving_ground.tasks();

    assert_eq!(
        tasks["tasks"][0]["start_url"],
        format!("http://1.catalog.test:{}/catalog/1", proving_ground.port())
    );
}

#[test]
fn serve_without_a_profile_reports_no_shaping() {
    let proving_ground = ProvingGround::start(160);

    let network = network(&proving_ground);

    assert_eq!(network, json!({ "profile": "none" }));
}
