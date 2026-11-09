use serde_json::{Value, json};

use super::*;
use crate::ObservedRequest;

/// What a Proving ground's observations hold after a browser loaded one
/// page and then another of `catalog.test`: the engine read the origin's
/// robots.txt between them, and the browser asked for its icon.
fn observations() -> Value {
    let requests: Vec<Value> = ["/catalog/1", "/robots.txt", "/favicon.ico", "/catalog/2"]
        .iter()
        .enumerate()
        .map(|(sequence, target)| request(sequence, target))
        .collect();
    json!({ "requests": requests, "connections": [] })
}

/// A `GET` of `target` on `catalog.test`, as the Proving ground lists it.
fn request(sequence: usize, target: &str) -> Value {
    json!({
        "sequence": sequence, "connection": 0, "task": null, "host": "catalog.test",
        "protocol": "http/1.1", "kind": "request", "method": "GET", "target": target,
        "pseudo_header_order": null, "headers": [], "cookies": [], "message_signature": null,
        "tls": null, "http2": null, "status": 200,
        "timing": { "arrived_microseconds": sequence, "answered_microseconds": null },
        "bytes": { "received": 0, "sent": 0 },
    })
}

fn targets<'a>(requests: impl Iterator<Item = &'a ObservedRequest>) -> Vec<&'a str> {
    requests.map(|request| request.target.as_str()).collect()
}

#[test]
fn the_requests_of_the_case_leave_out_the_robots_txt_read_and_the_icon() {
    let observations = observations();

    let received = ReceivedRequests::of(&observations);

    assert_eq!(
        targets(received.of_the_case()),
        ["/catalog/1", "/catalog/2"]
    );
}

#[test]
fn every_request_keeps_the_robots_txt_read_and_the_icon_in_arrival_order() {
    let observations = observations();

    let received = ReceivedRequests::of(&observations);

    assert_eq!(
        targets(received.every()),
        ["/catalog/1", "/robots.txt", "/favicon.ico", "/catalog/2"]
    );
}
