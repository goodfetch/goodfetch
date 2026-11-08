//! The `live` family (#1536): a board kept busy by a WebSocket, a
//! server-sent events stream and periodic requests, which stop on
//! `live/1`'s page and never on `live/2`'s. A tool that waits for
//! `networkidle` with the WebSocket and the stream left out reads the
//! settled board; one that reads it early reads a stale one; and on the
//! board that never settles, the only right result is a failure. The
//! observations name each request's kind.
//! Covers: story:2-33
#![expect(
    clippy::disallowed_types,
    reason = "the test opens its own sockets to the connections the board holds open"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 1536;

/// The board of task `ordinal`.
fn board(proving_ground: &ProvingGround, ordinal: u32) -> String {
    let answer = proving_ground.get(&proving_ground.url(&format!("{ordinal}.live.test"), "/board"));
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.body
}

/// Opens a connection to the Proving ground and sends `head`'s request to
/// `path` on task 1's host; the reader of its answer.
fn open(proving_ground: &ProvingGround, path: &str, head: &str) -> BufReader<TcpStream> {
    let mut stream =
        TcpStream::connect(proving_ground.address()).expect("the Proving ground accepts");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 1.live.test:{}\r\n{head}\r\n",
        proving_ground.port()
    )
    .expect("the request is sent");
    BufReader::new(stream)
}

/// The answer's head, up to its blank line.
fn head(reader: &mut BufReader<TcpStream>) -> String {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("the head is readable");
        if line == "\r\n" || line.is_empty() {
            return head;
        }
        head.push_str(&line);
    }
}

/// The kinds the observations give task `task`'s requests to `target`.
fn kinds(observations: &Value, task: &str, target: &str) -> Vec<Value> {
    observations["requests"]
        .as_array()
        .expect("`requests` is an array")
        .iter()
        .filter(|request| request["task"] == task && request["target"] == target)
        .map(|request| request["kind"].clone())
        .collect()
}

#[test]
fn the_two_boards_differ_only_in_whether_their_periodic_requests_stop() {
    let proving_ground = ProvingGround::start(SEED);

    let settling = board(&proving_ground, 1);
    let never = board(&proving_ground, 2);

    assert!(settling.contains("data-updates=\"6\""), "{settling}");
    assert_eq!(
        settling.replace("data-updates=\"6\"", "data-updates=\"unbounded\""),
        never
    );
}

#[test]
fn the_settling_boards_periodic_requests_end_at_its_last_and_the_other_boards_go_on() {
    let proving_ground = ProvingGround::start(SEED);
    let update = |ordinal: u32, number: u32| {
        proving_ground.get(&proving_ground.url(
            &format!("{ordinal}.live.test"),
            &format!("/updates/{number}"),
        ))
    };

    let last = update(1, 6);
    let past_last = update(1, 7);
    let never_ending = update(2, 7);

    assert_eq!(last.status, 200);
    assert_eq!(last.json()["update"], 6);
    assert_eq!(past_last.status, 404);
    assert_eq!(never_ending.status, 200);
}

#[test]
fn the_event_stream_sends_numbered_events_and_is_observed_as_one() {
    let proving_ground = ProvingGround::start(SEED);
    let mut reader = open(&proving_ground, "/events", "Accept: text/event-stream\r\n");

    let head = head(&mut reader);
    let mut chunk = [0; 256];
    let read = reader.read(&mut chunk).expect("an event arrives");

    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: text/event-stream"),
        "{head}"
    );
    assert!(
        String::from_utf8_lossy(&chunk[..read]).contains("data: {\"event\":1}"),
        "{:?}",
        &chunk[..read]
    );
    assert_eq!(
        kinds(&proving_ground.observations(), "live/1", "/events"),
        [json!("event_stream")]
    );
}

#[test]
fn the_websocket_answers_its_handshake_sends_messages_and_is_observed_as_one() {
    let proving_ground = ProvingGround::start(SEED);
    let mut reader = open(
        &proving_ground,
        "/socket",
        "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n",
    );

    let head = head(&mut reader);
    let mut frame = [0; 2];
    reader.read_exact(&mut frame).expect("a message arrives");
    let mut message = vec![0; usize::from(frame[1])];
    reader
        .read_exact(&mut message)
        .expect("the message is whole");

    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(
        head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
        "the accept value is RFC 6455's: {head}"
    );
    assert_eq!(
        (frame[0], message.as_slice()),
        (0x81, b"{\"message\":1}".as_slice())
    );
    assert_eq!(
        kinds(&proving_ground.observations(), "live/1", "/socket"),
        [json!("websocket")]
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test measures how long the Proving ground holds a request, outside any engine's Clock (ADR 0059)"
)]
fn the_late_board_starts_its_one_request_at_its_load_and_is_held_past_the_quiet() {
    let proving_ground = ProvingGround::start(SEED);

    let late = |ordinal: u32| {
        proving_ground.get(&proving_ground.url(&format!("{ordinal}.live.test"), "/late"))
    };
    let (settling, never) = (late(1), late(2));
    let asked = Instant::now();
    let update = proving_ground.get(&proving_ground.url("1.live.test", "/late/updates/1"));
    let held = asked.elapsed();

    assert_eq!(settling.status, 200, "{}", settling.body);
    assert!(
        settling
            .body
            .contains("data-updates=\"1\" data-at-load data-requests=\"/late/updates/\""),
        "{}",
        settling.body
    );
    assert_eq!(settling.body, never.body);
    assert_eq!(
        (update.status, update.json()["update"].clone()),
        (200, json!(1))
    );
    assert!(held >= Duration::from_millis(800), "{held:?}");
}

#[test]
fn a_websocket_request_without_its_handshake_is_refused() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.get(&proving_ground.url("1.live.test", "/socket"));

    assert_eq!(answer.status, 400);
}

#[test]
fn a_board_read_once_its_network_is_idle_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "live/1", "board-when-idle", json!({}));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_board_read_as_soon_as_its_document_loads_is_a_stale_idle_and_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(&proving_ground, "live/1", "board-at-load", json!({}));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let wrong: Vec<&Value> = answer["wrong"]
        .as_array()
        .expect("`wrong` is an array")
        .iter()
        .map(|wrong| &wrong["field"])
        .collect();
    assert_eq!(wrong, ["reading", "state", "updates"], "{answer}");
}

#[test]
fn a_browser_on_the_settling_board_holds_a_websocket_and_an_event_stream_beside_its_requests() {
    let proving_ground = ProvingGround::start(SEED);

    let _answer = browser::judged(&proving_ground, "live/1", "board-when-idle", json!({}));
    let observations = proving_ground.observations();

    assert_eq!(
        kinds(&observations, "live/1", "/socket"),
        [json!("websocket")]
    );
    assert_eq!(
        kinds(&observations, "live/1", "/events"),
        [json!("event_stream")]
    );
    assert_eq!(
        kinds(&observations, "live/1", "/updates/6"),
        [json!("request")]
    );
    assert_eq!(
        kinds(&observations, "live/1", "/updates/7"),
        Vec::<Value>::new()
    );
}

#[test]
fn the_never_settling_board_never_becomes_idle_so_a_failure_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let result = browser::run(
        "live/2",
        "board-when-idle",
        &browser::start_url(&proving_ground, "live/2"),
        json!({ "deadline_ms": 3000 }),
    );
    let answer = proving_ground.judge(&result);

    assert_eq!(result["status"], "failure", "{result}");
    assert_eq!(answer["outcome"], "verified", "{answer}");
    let observations = proving_ground.observations();
    assert_eq!(
        kinds(&observations, "live/2", "/socket"),
        [json!("websocket")]
    );
    assert_eq!(
        kinds(&observations, "live/2", "/events"),
        [json!("event_stream")]
    );
}

#[test]
fn a_board_claimed_from_the_never_settling_page_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let record = json!({ "id": "board", "state": "updating", "updates": "9", "reading": "R-0000" });

    let answer = proving_ground.judge(&submission("live/2", "success", &[record]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}
