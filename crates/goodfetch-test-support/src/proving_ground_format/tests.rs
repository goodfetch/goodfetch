use serde_json::json;

use super::*;

/// A request as the Proving ground lists it, every field present.
fn listed() -> Value {
    json!({
        "sequence": 0, "connection": 3, "task": "catalog/1", "host": "catalog.test",
        "protocol": "http/1.1", "kind": "request", "method": "GET", "target": "/catalog/1",
        "pseudo_header_order": null, "headers": [["Host", "catalog.test"], ["Accept", "*/*"]],
        "cookies": [], "message_signature": null, "tls": null, "http2": null, "status": 200,
        "timing": { "arrived_microseconds": 10, "answered_microseconds": 12 },
        "bytes": { "received": 1, "sent": 2 },
    })
}

#[test]
fn a_listed_request_reads_back_typed_and_writes_back_the_same() {
    let request: ObservedRequest = serde_json::from_value(listed()).unwrap();

    assert_eq!(
        (
            request.host.as_str(),
            request.target.as_str(),
            request.header("accept"),
            request.timing.arrived_microseconds,
            serde_json::to_value(&request).unwrap(),
        ),
        ("catalog.test", "/catalog/1", Some("*/*"), 10, listed())
    );
}

#[test]
fn a_request_with_a_field_the_format_lacks_is_refused() {
    let mut request = listed();
    request["renamed_host"] = json!("catalog.test");

    let read = serde_json::from_value::<ObservedRequest>(request);

    assert!(read.is_err());
}

#[test]
fn the_observations_keep_the_rest_of_the_record_by_name() {
    let document = json!({
        "requests": [listed()], "unrecorded_requests": 0,
        "dns_queries": [{ "name": "a.test", "source": "127.0.0.1:5353" }],
    });

    let observations: Observations = serde_json::from_value(document).unwrap();

    assert_eq!(
        (
            observations.requests.len(),
            observations
                .record
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        ),
        (1, vec!["dns_queries"])
    );
}

#[test]
fn an_older_tags_serve_line_without_a_udp_fixture_reads_with_none() {
    let line = json!({
        "address": "127.0.0.1:4000", "version": "0.0.0+old", "seed": 7, "hosts": ["a.test"],
        "certificate_authority": "PEM", "network": { "profile": "none", "shaped": null },
        "proxies": { "down": "127.0.0.1:1" },
        "proxy_credentials": { "username": "u", "password": "p" },
        "dns": "127.0.0.1:2", "target": "127.0.0.1:3",
    });

    let read: ServeLine = serde_json::from_value(line).unwrap();

    assert_eq!(
        (read.address.port(), read.fixtures.udp.as_str()),
        (4000, "")
    );
}

#[test]
fn observations_without_a_list_of_requests_are_refused() {
    let read = ReceivedRequests::read(&json!({ "connections": [] }));

    assert!(read.is_err());
}
