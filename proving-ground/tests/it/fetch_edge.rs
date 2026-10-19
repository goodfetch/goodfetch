//! The fetch edge pages (ADR 0118), each served on demand over TLS and
//! judged from the Proving ground's own observations: cut bodies against
//! the length their head declared, requests a site never processed with the
//! connection and protocol each attempt came on and whether it repeats one,
//! the time from an answer to the next request, refresh stubs, redirects, and
//! the address family of each connection.

use std::io::Read as _;
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;
use crate::raw_clients::{self, Http2Outcome};

const SEED: u64 = 162;
const HOST: &str = "fetch-edge.test";
const HTTP1: &str = "h1.fetch-edge.test";
const HTTP2: &str = "h2.fetch-edge.test";

/// The recorded requests to `host` for `target`, in order.
fn requests_for(observations: &Value, host: &str, target: &str) -> Vec<Value> {
    observations["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .filter(|request| request["host"] == host && request["target"] == target)
        .cloned()
        .collect()
}

/// The one recorded request to `host` for `target`.
fn request_for(observations: &Value, host: &str, target: &str) -> Value {
    let found = requests_for(observations, host, target);
    assert_eq!(found.len(), 1, "{observations}");
    found[0].clone()
}

/// Covers: story:2-40
#[test]
fn a_body_ending_before_its_content_length_is_recorded_against_it() {
    let proving_ground = ProvingGround::start(SEED);

    let arrived = raw_clients::get(&proving_ground, HTTP1, "/cut/content-length", &[]);

    let declared: usize = arrived
        .header("content-length")
        .and_then(|length| length.parse().ok())
        .expect("a declared length");
    assert_eq!(arrived.status(), Some(200));
    assert!(arrived.body().len() < declared, "{}", arrived.body().len());
    assert!(!arrived.closed_cleanly);
    let delivery =
        &request_for(&proving_ground.observations(), HTTP1, "/cut/content-length")["delivery"];
    assert_eq!(delivery["declared_length"], declared);
    assert_eq!(delivery["body_sent"], arrived.body().len());
    assert_eq!(delivery["end"], json!({ "kind": "connection_closed" }));
}

#[test]
fn a_chunked_body_breaks_off_without_its_last_chunk() {
    let proving_ground = ProvingGround::start(SEED);

    let arrived = raw_clients::get(&proving_ground, HTTP1, "/cut/chunked", &[]);

    assert_eq!(
        arrived.header("transfer-encoding").as_deref(),
        Some("chunked")
    );
    assert!(!arrived.body().ends_with(b"0\r\n\r\n"));
    assert!(!arrived.closed_cleanly);
    let delivery = &request_for(&proving_ground.observations(), HTTP1, "/cut/chunked")["delivery"];
    assert_eq!(delivery["declared_length"], Value::Null);
    assert_eq!(delivery["end"], json!({ "kind": "connection_closed" }));
}

#[test]
fn an_http2_stream_is_reset_after_its_headers_or_ended_by_its_connection() {
    let proving_ground = ProvingGround::start(SEED);

    let reset = raw_clients::http2(&proving_ground, HTTP2, &["/cut/reset"]);
    let ended = raw_clients::http2(&proving_ground, HTTP2, &["/cut/connection"]);

    assert!(
        matches!(reset[..], [Http2Outcome::BodyCut { status: 200, .. }]),
        "{reset:?}"
    );
    assert!(
        matches!(
            ended[..],
            [Http2Outcome::BodyCut { status: 200, .. } | Http2Outcome::GoneAway(_)]
        ),
        "{ended:?}"
    );
    let observations = proving_ground.observations();
    let reset = &request_for(&observations, HTTP2, "/cut/reset")["delivery"];
    assert_eq!(
        reset["end"],
        json!({ "kind": "stream_reset", "error_code": "INTERNAL_ERROR" })
    );
    assert!(reset["body_sent"].as_u64() < reset["declared_length"].as_u64());
    let ended = &request_for(&observations, HTTP2, "/cut/connection")["delivery"];
    assert_eq!(ended["end"], json!({ "kind": "connection_closed" }));
}

#[test]
fn a_gzip_body_stops_before_its_stream_ends() {
    let proving_ground = ProvingGround::start(SEED);

    let arrived = raw_clients::get(&proving_ground, HTTP1, "/cut/gzip", &[]);

    assert_eq!(arrived.header("content-encoding").as_deref(), Some("gzip"));
    let mut decoded = Vec::new();
    let decoding = flate2::read::GzDecoder::new(arrived.body()).read_to_end(&mut decoded);
    assert!(decoding.is_err(), "the stream decoded whole");
    let delivery = &request_for(&proving_ground.observations(), HTTP1, "/cut/gzip")["delivery"];
    assert_eq!(delivery["declared_length"], delivery["body_sent"]);
}

#[test]
fn a_body_delimited_by_its_connections_close_is_whole() {
    let proving_ground = ProvingGround::start(SEED);

    let arrived = raw_clients::get(&proving_ground, HTTP1, "/close-delimited", &[]);

    assert_eq!(arrived.header("content-length"), None);
    assert!(arrived.body().ends_with(b"</html>"));
    assert!(arrived.closed_cleanly);
    let delivery =
        &request_for(&proving_ground.observations(), HTTP1, "/close-delimited")["delivery"];
    assert_eq!(delivery["end"], json!({ "kind": "complete" }));
}

/// Covers: story:2-40
#[test]
fn a_request_above_a_goaways_last_stream_is_resent_once_on_a_new_connection() {
    let proving_ground = ProvingGround::start(SEED);
    let host = "goaway.fetch-edge.test";

    let first = raw_clients::http2(&proving_ground, host, &["/first", "/second"]);
    let resent = raw_clients::http2(&proving_ground, host, &["/second"]);

    assert!(
        matches!(
            first[..],
            [
                Http2Outcome::Answered { status: 200, .. },
                Http2Outcome::GoneAway(_)
            ]
        ),
        "{first:?}"
    );
    assert!(matches!(
        resent[..],
        [Http2Outcome::Answered { status: 200, .. }]
    ));
    let observations = proving_ground.observations();
    let attempts = requests_for(&observations, host, "/second");
    let answered = request_for(&observations, host, "/first");
    assert_eq!(attempts.len(), 2, "{observations}");
    assert_eq!(
        attempts[0]["delivery"]["end"],
        json!({
            "kind": "go_away",
            "last_stream": answered["http2"]["stream"],
            "error_code": "NO_ERROR",
        })
    );
    assert_eq!(attempts[0]["connection"], answered["connection"]);
    assert_eq!(attempts[0]["method"], "GET");
    assert_eq!(attempts[0]["protocol"], "h2");
    assert_ne!(attempts[1]["connection"], attempts[0]["connection"]);
    assert_eq!(attempts[1]["repeats"], attempts[0]["sequence"]);
    assert_eq!(attempts[0].get("repeats"), None);
}

#[test]
fn refused_stream_answers_only_the_resend() {
    let proving_ground = ProvingGround::start(SEED);

    let refused = raw_clients::http2(&proving_ground, HTTP2, &["/refused"]);
    let resent = raw_clients::http2(&proving_ground, HTTP2, &["/refused"]);
    let always = raw_clients::http2(&proving_ground, HTTP2, &["/refused/always"]);

    assert_eq!(refused, [Http2Outcome::Reset(h2::Reason::REFUSED_STREAM)]);
    assert!(matches!(
        resent[..],
        [Http2Outcome::Answered { status: 200, .. }]
    ));
    assert_eq!(always, [Http2Outcome::Reset(h2::Reason::REFUSED_STREAM)]);
    let attempts = requests_for(&proving_ground.observations(), HTTP2, "/refused");
    assert_eq!(
        attempts[0]["delivery"]["end"],
        json!({ "kind": "stream_reset", "error_code": "REFUSED_STREAM" })
    );
    assert_eq!(attempts[1]["repeats"], attempts[0]["sequence"]);
}

#[test]
fn a_misdirected_request_on_a_coalesced_connection_is_answered_421() {
    let proving_ground = ProvingGround::start(SEED);
    let other = "misdirected.fetch-edge.test";

    let coalesced = raw_clients::http2_coalesced(&proving_ground, HTTP2, other, &["/target"]);
    let own = raw_clients::http2(&proving_ground, other, &["/target"]);

    assert!(matches!(
        coalesced[..],
        [Http2Outcome::Answered { status: 421, .. }]
    ));
    assert!(matches!(
        own[..],
        [Http2Outcome::Answered { status: 200, .. }]
    ));
    let attempts = requests_for(&proving_ground.observations(), other, "/target");
    assert_eq!(attempts[0]["tls"]["sni"], HTTP2);
    assert_eq!(attempts[1]["tls"]["sni"], other);
    assert_eq!(attempts[1]["repeats"], attempts[0]["sequence"]);
}

#[test]
fn http_1_1_required_is_answered_over_http_1_1() {
    let proving_ground = ProvingGround::start(SEED);

    let refused = raw_clients::http2(&proving_ground, HTTP2, &["/http1-required"]);
    let resent = raw_clients::get(&proving_ground, HTTP2, "/http1-required", &[]);

    assert_eq!(
        refused,
        [Http2Outcome::Reset(h2::Reason::HTTP_1_1_REQUIRED)]
    );
    assert_eq!(resent.status(), Some(200));
    let attempts = requests_for(&proving_ground.observations(), HTTP2, "/http1-required");
    assert_eq!(attempts[0]["protocol"], "h2");
    assert_eq!(attempts[1]["protocol"], "http/1.1");
    assert_eq!(attempts[1]["repeats"], attempts[0]["sequence"]);
}

#[test]
fn a_keep_alive_connection_closes_or_answers_408_as_it_is_reused() {
    let proving_ground = ProvingGround::start(SEED);

    let closed = raw_clients::keep_alive(
        &proving_ground,
        HTTP1,
        &[("POST", "/target"), ("POST", "/reuse/close")],
    );
    let resent = raw_clients::keep_alive(&proving_ground, HTTP1, &[("POST", "/reuse/close")]);
    let timed_out = raw_clients::keep_alive(
        &proving_ground,
        HTTP1,
        &[("GET", "/target"), ("GET", "/reuse/408")],
    );

    assert_eq!(
        closed[0].as_ref().and_then(raw_clients::Arrived::status),
        Some(200)
    );
    assert!(closed[1].is_none());
    assert_eq!(
        resent[0].as_ref().and_then(raw_clients::Arrived::status),
        Some(200)
    );
    assert_eq!(
        timed_out[1].as_ref().and_then(raw_clients::Arrived::status),
        Some(408)
    );
    let attempts = requests_for(&proving_ground.observations(), HTTP1, "/reuse/close");
    assert_eq!(attempts[0]["method"], "POST");
    assert_eq!(attempts[0]["status"], Value::Null);
    assert_eq!(
        attempts[0]["delivery"],
        json!({ "declared_length": null, "body_sent": 0, "end": { "kind": "connection_closed" } })
    );
    assert_ne!(attempts[1]["connection"], attempts[0]["connection"]);
    assert_eq!(attempts[1]["repeats"], attempts[0]["sequence"]);
}

#[test]
fn enhance_your_calm_is_sent_and_the_next_request_is_timed_from_it() {
    let proving_ground = ProvingGround::start(SEED);

    let calmed = raw_clients::http2(&proving_ground, HTTP2, &["/calm"]);
    let again = raw_clients::http2(&proving_ground, HTTP2, &["/calm"]);

    assert_eq!(calmed, [Http2Outcome::Reset(h2::Reason::ENHANCE_YOUR_CALM)]);
    assert!(matches!(
        again[..],
        [Http2Outcome::Answered { status: 200, .. }]
    ));
    let attempts = requests_for(&proving_ground.observations(), HTTP2, "/calm");
    assert_eq!(
        attempts[0]["next_request"]["sequence"],
        attempts[1]["sequence"]
    );
    assert!(attempts[0]["next_request"]["after_microseconds"].is_u64());
}

/// Covers: story:2-40
#[test]
fn retry_after_is_sent_in_seconds_and_each_http_date_format_on_429_503_and_302() {
    let proving_ground = ProvingGround::start(SEED);
    let formats = [
        ("seconds", r"^2$"),
        (
            "imf-fixdate",
            r"^(Mon|Tue|Wed|Thu|Fri|Sat|Sun), \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT$",
        ),
        (
            "rfc850",
            r"^[A-Z][a-z]+day, \d{2}-[A-Z][a-z]{2}-\d{2} \d{2}:\d{2}:\d{2} GMT$",
        ),
        (
            "asctime",
            r"^[A-Z][a-z]{2} [A-Z][a-z]{2} [ \d]\d \d{2}:\d{2}:\d{2} \d{4}$",
        ),
    ];

    for status in [429, 503, 302] {
        for (form, pattern) in formats {
            let path = format!("/retry-after/{status}/{form}");
            let first = raw_clients::get(&proving_ground, HOST, &path, &[]);
            let second = raw_clients::get(&proving_ground, HOST, &path, &[]);

            assert_eq!(first.status(), Some(status), "{path}");
            let retry_after = first.header("retry-after").expect("a Retry-After");
            assert!(
                Regex::new(pattern).unwrap().is_match(&retry_after),
                "{path}: {retry_after}"
            );
            assert!(first.header("date").is_some(), "{path}");
            if status == 302 {
                assert_eq!(first.header("location").as_deref(), Some("/target"));
            }
            assert_eq!(second.status(), Some(200), "{path}");
        }
    }
    let observations = proving_ground.observations();
    let attempts = requests_for(&observations, HOST, "/retry-after/503/seconds");
    assert_eq!(
        attempts[0]["next_request"]["sequence"],
        attempts[1]["sequence"]
    );
    let long = raw_clients::get(&proving_ground, HOST, "/retry-after/503/long", &[]);
    assert_eq!(long.header("retry-after").as_deref(), Some("3600"));
}

#[test]
fn refresh_stubs_name_their_targets() {
    let proving_ground = ProvingGround::start(SEED);
    let stubs = [
        ("/refresh/meta/0", "content=\"0; url=/target\""),
        ("/refresh/meta/5", "content=\"5; url=/target\""),
        ("/refresh/meta/200", "content=\"200; url=/target\""),
        (
            "/refresh/noscript",
            "<noscript><meta http-equiv=\"refresh\"",
        ),
        ("/refresh/no-url", "content=\"0\""),
        ("/refresh/self", "url=/refresh/self"),
        ("/refresh/script", "location.replace(\"/target\")"),
    ];

    for (path, directive) in stubs {
        let stub = raw_clients::get(&proving_ground, HOST, path, &[]);

        assert_eq!(stub.status(), Some(200), "{path}");
        assert!(
            String::from_utf8_lossy(stub.body()).contains(directive),
            "{path}"
        );
    }
    let header = raw_clients::get(&proving_ground, HOST, "/refresh/header", &[]);
    assert_eq!(header.header("refresh").as_deref(), Some("0; url=/target"));
    let target = raw_clients::get(&proving_ground, HOST, "/target", &[]);
    assert_eq!(target.status(), Some(200));
    assert_eq!(
        requests_for(&proving_ground.observations(), HOST, "/target").len(),
        1
    );
}

#[test]
fn redirects_are_served_as_each_case_needs() {
    let proving_ground = ProvingGround::start(SEED);
    let port = proving_ground.port().to_owned();
    let cross_origin = format!("https://{HTTP1}:{port}/target");
    let redirects = [
        ("/redirect/loop", 302, Some("/redirect/loop")),
        ("/redirect/301", 301, Some("/target")),
        ("/redirect/302", 302, Some("/target")),
        ("/redirect/303", 303, Some("/target")),
        ("/redirect/307", 307, Some("/target")),
        ("/redirect/308", 308, Some("/target")),
        ("/redirect/300", 300, Some("/target")),
        ("/redirect/no-location", 302, None),
        ("/redirect/ftp", 302, Some("ftp://fetch-edge.test/target")),
        ("/redirect/cross-origin", 302, Some(cross_origin.as_str())),
        ("/redirect/hops/2", 302, Some("/redirect/hops/1")),
        ("/redirect/robots", 302, Some("/private/target")),
    ];

    for (path, status, location) in redirects {
        let answer = raw_clients::get(&proving_ground, HOST, path, &[]);

        assert_eq!(answer.status(), Some(status), "{path}");
        assert_eq!(answer.header("location").as_deref(), location, "{path}");
    }
    let robots = raw_clients::get(&proving_ground, HOST, "/robots.txt", &[]);
    assert!(String::from_utf8_lossy(robots.body()).contains("Disallow: /private/"));
    let cookie = raw_clients::get(&proving_ground, HOST, "/redirect/cookie", &[]);
    assert_eq!(
        cookie.header("location").as_deref(),
        Some("/redirect/cookie")
    );
    let set = cookie.header("set-cookie").expect("a cookie");
    let back = raw_clients::get(
        &proving_ground,
        HOST,
        "/redirect/cookie",
        &[("Cookie", set.split(';').next().unwrap())],
    );
    assert_eq!(back.status(), Some(200));
}

#[test]
fn the_ipv6_path_drops_connections_while_ipv4_answers() {
    let proving_ground = ProvingGround::start(SEED);
    let host = "ipv6.fetch-edge.test";

    let answered = raw_clients::get(&proving_ground, host, "/target", &[]);

    assert_eq!(answered.status(), Some(200));
    let observations = proving_ground.observations();
    let request = request_for(&observations, host, "/target");
    let connection = observations["connections"]
        .as_array()
        .expect("connections")
        .iter()
        .find(|connection| connection["connection"] == request["connection"])
        .cloned()
        .expect("the connection is recorded");
    assert_eq!(connection["family"], "ipv4");
    assert!(connection["accepted_microseconds"].is_u64());
    if ipv6_loopback() {
        let port: u16 = proving_ground.port().parse().unwrap();
        #[expect(
            clippy::disallowed_types,
            reason = "the test connects to the Proving ground's IPv6 path itself"
        )]
        let attempt = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)),
            Duration::from_millis(500),
        );
        // Linux drops the SYN at the full queue, so the attempt waits out
        // its timeout; other kernels may reset it or queue it, unanswered.
        if cfg!(target_os = "linux") {
            assert_eq!(
                attempt.as_ref().map(|_| ()).map_err(std::io::Error::kind),
                Err(std::io::ErrorKind::TimedOut)
            );
        }
        if let Ok(mut stream) = attempt {
            use std::io::Write as _;
            // A connection the kernel has already reset refuses the
            // option; its read then fails at once.
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let _ = stream.write_all(b"GET /target HTTP/1.1\r\nHost: ipv6.fetch-edge.test\r\n\r\n");
            let mut answer = [0; 16];
            assert!(
                !matches!(stream.read(&mut answer), Ok(read) if read > 0),
                "the IPv6 path answered"
            );
        }
    }
}

/// Whether this host has IPv6 loopback.
fn ipv6_loopback() -> bool {
    #[expect(
        clippy::disallowed_types,
        reason = "the test checks whether IPv6 loopback exists"
    )]
    std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0)).is_ok()
}

#[test]
fn a_plain_text_request_to_a_raw_host_is_misdirected() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.get(&proving_ground.url(HOST, "/target"));

    assert_eq!(answer.status, 421);
    assert_eq!(answer.json(), json!({ "error": "tls_required" }));
}
