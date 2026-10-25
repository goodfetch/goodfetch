//! Server-side observations: every request a page host receives, as the
//! Proving ground saw it on the wire (its headers in order, its TLS
//! `ClientHello` and JA4, its HTTP/2 settings and pseudo-header order, its
//! cookies, HTTP message signature and timing) and the bytes each request and each host
//! received and sent, which the oracle and the suites read at
//! `GET /observations`, never from a tool's report.

use std::sync::Arc;

use regex::Regex;
use serde_json::{Value, json};

use crate::clients::{self, Http2Settings};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 159;

const HTTP2: Http2Settings = Http2Settings {
    initial_window_size: 1_048_576,
    initial_connection_window_size: 2_097_152,
    max_concurrent_streams: 77,
    header_table_size: 8192,
};

/// The record of one request, its timing taken out after checking its
/// order.
fn without_timing(mut record: Value) -> Value {
    let timing = record
        .as_object_mut()
        .and_then(|record| record.remove("timing"))
        .expect("a record has its timing");
    let arrived = timing["arrived_microseconds"].as_u64().expect("an arrival");
    let answered = timing["answered_microseconds"].as_u64().expect("an answer");
    assert!(arrived <= answered, "{timing}");
    record
}

#[test]
fn a_request_is_recorded_with_its_headers_in_the_order_sent() {
    let proving_ground = ProvingGround::start(SEED);
    let request = b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nX-Second: b\r\naccept: */*\r\nCookie: session=s1; theme=dark\r\nX-First: a\r\nConnection: close\r\n\r\n";
    let exchange = clients::raw(proving_ground.address(), request);

    let observations = proving_ground.observations();

    assert_eq!(exchange.status(), 200);
    assert_eq!(
        without_timing(observations["requests"][0].clone()),
        json!({
            "sequence": 0,
            "connection": 0,
            "task": null,
            "host": "catalog.test",
            "protocol": "http/1.1",
            "kind": "request",
            "method": "GET",
            "target": "/catalog/1",
            "pseudo_header_order": null,
            "headers": [
                ["Host", "catalog.test"],
                ["X-Second", "b"],
                ["accept", "*/*"],
                ["Cookie", "session=s1; theme=dark"],
                ["X-First", "a"],
                ["Connection", "close"],
            ],
            "cookies": [["session", "s1"], ["theme", "dark"]],
            "message_signature": null,
            "tls": null,
            "http2": null,
            "status": 200,
            "bytes": { "received": request.len(), "sent": exchange.read },
        })
    );
}

#[test]
fn a_signed_request_is_recorded_with_its_message_signature() {
    let proving_ground = ProvingGround::start(SEED);
    clients::raw(
        proving_ground.address(),
        b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nSignature-Agent: \"https://agent.test\"\r\nSignature-Input: sig1=(\"@authority\");tag=\"web-bot-auth\"\r\nSignature: sig1=:AAEC:\r\nConnection: close\r\n\r\n",
    );

    let observations = proving_ground.observations();

    assert_eq!(
        observations["requests"][0]["message_signature"],
        json!({
            "signature_agent": "\"https://agent.test\"",
            "signature_input": "sig1=(\"@authority\");tag=\"web-bot-auth\"",
            "signature": "sig1=:AAEC:",
        })
    );
}

#[test]
fn requests_to_the_proving_grounds_own_address_are_not_observed() {
    let proving_ground = ProvingGround::start(SEED);
    proving_ground.tasks();

    let observations = proving_ground.observations();

    assert_eq!(observations["requests"], json!([]));
    assert_eq!(observations["hosts"], json!({}));
}

#[test]
fn a_request_over_tls_is_recorded_with_its_client_hello_and_ja4() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    clients::raw_tls(
        proving_ground.address(),
        "catalog.test",
        config,
        b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n",
    )
    .expect("the certificate verifies");

    let observations = proving_ground.observations();

    let tls = &observations["requests"][0]["tls"];
    assert_eq!(tls["sni"], "catalog.test", "{tls}");
    assert_eq!(tls["alpn"], json!(["http/1.1"]), "{tls}");
    assert_eq!(tls["version"], "13", "{tls}");
    assert_eq!(tls["resumed"], false, "{tls}");
    let ja4 = tls["ja4"].as_str().expect("a JA4");
    assert!(
        Regex::new(r"^t13d\d{4}h1_[0-9a-f]{12}_[0-9a-f]{12}$")
            .unwrap()
            .is_match(ja4),
        "{ja4}"
    );
    assert_eq!(tls["ja4_base"], ja4);
}

/// A client that trusts none of the Proving ground's certificates ends its
/// handshake with no request, yet its `ClientHello` is in the record of
/// TLS handshakes, which lists every one, a request's included, so a
/// browser that cannot trust the Proving ground still shows its
/// fingerprint.
#[test]
fn every_tls_handshake_is_recorded_with_its_client_hello_whether_or_not_it_completed() {
    let proving_ground = ProvingGround::start(SEED);
    let trusted = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let untrusted = clients::trusting("", &["http/1.1"]);
    clients::raw_tls(
        proving_ground.address(),
        "catalog.test",
        trusted,
        b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n",
    )
    .expect("the certificate verifies");
    let refused = clients::raw_tls(
        proving_ground.address(),
        "1.access.test",
        untrusted,
        b"GET / HTTP/1.1\r\nHost: 1.access.test\r\nConnection: close\r\n\r\n",
    );

    let observations = proving_ground.observations();

    assert!(refused.is_err());
    let handshakes = observations["tls_handshakes"].as_array().expect("a list");
    assert_eq!(
        handshakes
            .iter()
            .map(|handshake| handshake["sni"].clone())
            .collect::<Vec<_>>(),
        vec![json!("catalog.test"), json!("1.access.test")]
    );
    assert_eq!(
        handshakes[0]["connection"],
        observations["requests"][0]["connection"]
    );
    assert_ne!(handshakes[0]["connection"], handshakes[1]["connection"]);
    assert_eq!(
        handshakes[0]["ja4"],
        observations["requests"][0]["tls"]["ja4"]
    );
    assert_eq!(observations["requests"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_resumed_connection_is_recorded_with_its_pre_shared_key() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let request = b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n";
    for _ in 0..2 {
        clients::raw_tls(
            proving_ground.address(),
            "catalog.test",
            Arc::clone(&config),
            request,
        )
        .expect("the certificate verifies");
    }

    let observations = proving_ground.observations();

    let [first, second] = [0, 1].map(|index| observations["requests"][index]["tls"].clone());
    assert_eq!(
        (&first["resumed"], &second["resumed"]),
        (&json!(false), &json!(true))
    );
    // `pre_shared_key` (41), which a resumed `ClientHello` carries last.
    assert_eq!(
        second["extensions"]
            .as_array()
            .and_then(|types| types.last()),
        Some(&json!(41))
    );
    assert_ne!(first["ja4"], second["ja4"]);
}

#[test]
fn an_http2_request_is_recorded_with_its_settings_and_pseudo_header_order() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["h2"]);
    clients::http2(
        proving_ground.address(),
        "catalog.test",
        config,
        &HTTP2,
        &["/catalog/1", "/catalog/2"],
        &[("x-second", "b"), ("x-first", "a")],
    );

    let observations = proving_ground.observations();

    let record = &observations["requests"][1];
    assert_eq!(record["protocol"], "h2", "{record}");
    assert_eq!(record["method"], "GET", "{record}");
    assert_eq!(record["target"], "/catalog/2", "{record}");
    assert_eq!(record["host"], "catalog.test", "{record}");
    assert_eq!(record["status"], 200, "{record}");
    assert_eq!(
        record["pseudo_header_order"],
        json!([":method", ":scheme", ":authority", ":path"]),
        "{record}"
    );
    assert_eq!(
        record["headers"],
        json!([["x-second", "b"], ["x-first", "a"]])
    );
    // h2 sends the settings it was given in their identifiers' order.
    assert_eq!(
        record["http2"],
        json!({
            "settings": [[1, 8192], [2, 0], [3, 77], [4, 1_048_576]],
            "window_update": 2_097_152 - 65_535,
            "stream": 3,
        })
    );
    assert_eq!(record["tls"]["alpn"], json!(["h2"]));
}

/// Covers: story:2-75
#[test]
fn each_request_is_counted_in_bytes_as_it_crosses_the_wire() {
    let proving_ground = ProvingGround::start(SEED);
    let request = b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n";
    let exchange = clients::raw(proving_ground.address(), request);

    let observations = proving_ground.observations();

    assert_eq!(
        observations["requests"][0]["bytes"],
        json!({ "received": request.len(), "sent": exchange.read })
    );
    assert_eq!(
        observations["hosts"],
        json!({ "catalog.test": { "received": exchange.written, "sent": exchange.read } })
    );
}

/// Covers: story:2-75
#[test]
fn a_hosts_bytes_over_tls_count_every_byte_of_its_connections() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let request = b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n";
    let exchange = clients::raw_tls(proving_ground.address(), "catalog.test", config, request)
        .expect("the certificate verifies");

    let observations = proving_ground.observations();

    assert_eq!(
        observations["requests"][0]["bytes"]["received"],
        request.len()
    );
    assert_eq!(
        observations["hosts"],
        json!({ "catalog.test": { "received": exchange.written, "sent": exchange.read } })
    );
}

/// Covers: story:2-75
#[test]
fn a_retried_request_is_counted_again() {
    let proving_ground = ProvingGround::start(SEED);
    let request = b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n";
    let first = clients::raw(proving_ground.address(), request);
    let second = clients::raw(proving_ground.address(), request);

    let observations = proving_ground.observations();

    assert_eq!(observations["requests"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        observations["hosts"]["catalog.test"],
        json!({
            "received": first.written + second.written,
            "sent": first.read + second.read,
        })
    );
}

/// Covers: story:2-75
#[test]
fn an_http2_streams_bytes_are_counted_per_request() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["h2"]);
    let answers = clients::http2(
        proving_ground.address(),
        "catalog.test",
        config,
        &HTTP2,
        &["/catalog/1"],
        &[],
    );

    let observations = proving_ground.observations();

    let bytes = &observations["requests"][0]["bytes"];
    let sent = bytes["sent"].as_u64().expect("a count");
    // The page's DATA frames, at least: its body and one 9-byte frame
    // header.
    assert!(sent >= answers[0].body.len() as u64 + 9, "{bytes}");
    let host = &observations["hosts"]["catalog.test"];
    assert!(host["sent"].as_u64().expect("a count") > sent, "{host}");
}

/// Covers: story:2-75
#[test]
fn a_tasks_requests_and_bytes_are_recorded_for_that_task() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let exchange = clients::raw_tls(
        proving_ground.address(),
        "1.catalog.test",
        config,
        b"GET /catalog/1 HTTP/1.1\r\nHost: 1.catalog.test\r\nConnection: close\r\n\r\n",
    )
    .expect("the certificate verifies");

    let observations = proving_ground.observations();

    assert_eq!(observations["requests"][0]["task"], "catalog/1");
    assert_eq!(
        observations["tasks"],
        json!({ "catalog/1": { "received": exchange.written, "sent": exchange.read } })
    );
    assert_eq!(
        observations["hosts"],
        json!({ "1.catalog.test": { "received": exchange.written, "sent": exchange.read } })
    );
}

#[rstest::rstest]
#[case::rendered("1.rendered.test", "rendered.test", "/feed", "rendered/1")]
#[case::login("2.login.test", "login.test", "/login", "login/2")]
#[case::schedule("1.schedule.test", "schedule.test", "/board", "schedule/1")]
fn a_tasks_host_serves_the_page_of_its_family_for_that_task(
    #[case] task_host: &str,
    #[case] family_host: &str,
    #[case] path: &str,
    #[case] task: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let on_task_host = proving_ground.get(&proving_ground.url(task_host, path));
    let on_family_host = proving_ground.get(&proving_ground.url(family_host, path));

    assert_eq!(on_task_host.status, 200, "{}", on_task_host.body);
    assert_eq!(on_task_host.body, on_family_host.body);
    let observations = proving_ground.observations();
    assert_eq!(observations["requests"][0]["task"], task, "{observations}");
    assert_eq!(
        observations["requests"][1]["task"],
        Value::Null,
        "{observations}"
    );
}

#[test]
fn an_effect_the_site_carries_out_is_recorded_for_its_task() {
    let proving_ground = ProvingGround::start(SEED);

    proving_ground.post_form(
        "http://3.catalog.test/orders",
        &[("product", "P-0042")],
        None,
    );

    assert_eq!(
        proving_ground.observations()["effects"],
        json!([{ "task": "catalog/3", "kind": "order", "product": "P-0042" }])
    );
}
