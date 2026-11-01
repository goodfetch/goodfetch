//! The Console's fallback transport through Collie's executable (spec #592's
//! Seam 1; ADR 0031), each test with a Daemon and home of its own, its
//! Console on port 0: behind a proxy that refuses `WebSocket`s, a signed-in
//! page carries a Connection over Server-Sent Events plus `POST`. `GET
//! /protocol/events` streams each message the engine sends as one event's
//! data, after a `stream` event naming where the page posts; each `POST
//! /protocol/messages/<stream>` carries the page's messages, one per line,
//! and is refused without its sign-in's CSRF token, each refusal checked in
//! the Audit log.

use std::io::{BufRead as _, BufReader, Read};

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

use crate::daemon::StopsTheDaemon;
use crate::lasting_sign_ins::audit_entries;
use crate::sign_in::{agent, console_url, origin, runtime_file};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A signed-in page of the Console of `env`'s Daemon: its URL, its cookie
/// pair and its CSRF token.
struct Page {
    url: String,
    cookie: String,
    csrf: String,
}

impl Page {
    /// Starts the Daemon of `env`'s home and signs in through the link.
    fn signed_in(env: &TestEnv) -> Self {
        let started = cli(env, COLLIE).args(["daemon", "start"]).run().unwrap();
        assert_eq!(started.exit, Some(0), "{}", started.stderr);
        let url = console_url(env);
        let (status, set) = crate::sign_in::sign_in(&url, &runtime_file(env).token);
        assert_eq!(status, 204);
        let cookie = set.unwrap().split("; ").next().unwrap().to_owned();
        let body = agent()
            .get(format!("{}/session", origin(&url)))
            .header("Cookie", &cookie)
            .call()
            .unwrap()
            .body_mut()
            .read_to_string()
            .unwrap();
        let session: Value = serde_json::from_str(&body).unwrap();
        let csrf = session["csrf_token"].as_str().unwrap().to_owned();
        Self { url, cookie, csrf }
    }

    /// Opens the event stream with the page's cookie: its status, its
    /// `Content-Type` and its events.
    fn events(&self) -> (u16, Option<String>, Events) {
        let response = agent()
            .get(format!("{}/protocol/events", origin(&self.url)))
            .header("Cookie", &self.cookie)
            .header("Accept", "text/event-stream")
            .call()
            .unwrap();
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .map(|value| value.to_str().unwrap().to_owned());
        let reader: Box<dyn Read + Send> = Box::new(response.into_body().into_reader());
        (status, content_type, Events(BufReader::new(reader)))
    }

    /// Posts `body` to the stream `stream`, with `csrf` if given: the status.
    fn post(&self, stream: &str, body: &str, csrf: Option<&str>) -> u16 {
        let mut request = agent()
            .post(format!("{}/protocol/messages/{stream}", origin(&self.url)))
            .header("Origin", origin(&self.url))
            .header("Cookie", &self.cookie)
            .content_type("text/plain;charset=UTF-8");
        if let Some(csrf) = csrf {
            request = request.header("X-CSRF-Token", csrf);
        }
        request.send(body).unwrap().status().as_u16()
    }
}

/// An event stream's events, as `(name, data)`.
struct Events(BufReader<Box<dyn Read + Send>>);

impl Events {
    /// The next event: its name (`message` when it names none) and its data.
    fn next(&mut self) -> (String, String) {
        let mut name = "message".to_owned();
        let mut data = Vec::new();
        loop {
            let mut line = String::new();
            assert_ne!(self.0.read_line(&mut line).unwrap(), 0, "the stream ended");
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if data.is_empty() {
                    continue;
                }
                return (name, data.join("\n"));
            }
            if let Some(value) = line.strip_prefix("event: ") {
                value.clone_into(&mut name);
            } else if let Some(value) = line.strip_prefix("data: ") {
                data.push(value.to_owned());
            }
        }
    }

    /// The next message's JSON.
    fn message(&mut self) -> Value {
        let (name, data) = self.next();
        assert_eq!(name, "message");
        serde_json::from_str(&data).unwrap()
    }
}

/// The `hello` the Console's page sends, as a line.
fn hello() -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "hello",
        "params": {
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "surface": "console",
            "form": "catalog",
            "needs": "collie",
            "product_version": VERSION,
        },
    })
    .to_string()
}

/// Covers: story:12-124, operation:engine.info
#[test]
fn a_signed_in_page_carries_a_connection_over_server_sent_events_and_post() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let page = Page::signed_in(&env);

    let (status, content_type, mut events) = page.events();
    let (first, stream) = events.next();
    let info = json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {} });
    let posted = page.post(&stream, &format!("{}\n{info}", hello()), Some(&page.csrf));
    let greeted = events.message();
    let answered = events.message();

    assert_eq!(status, 200);
    assert_eq!(content_type.as_deref(), Some("text/event-stream"));
    assert_eq!(first, "stream");
    assert_eq!(posted, 202);
    assert_eq!(greeted["id"], json!(0));
    assert_eq!(greeted["result"]["product"], json!("collie"), "{greeted}");
    assert_eq!(answered["id"], json!(1));
    assert_eq!(answered["result"]["product"], json!("collie"), "{answered}");
}

/// Covers: story:12-124, story:12-110
#[test]
fn each_post_of_the_event_stream_without_its_csrf_token_is_refused() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let page = Page::signed_in(&env);
    let (_, _, mut events) = page.events();
    let (_, stream) = events.next();

    let missing = page.post(&stream, &hello(), None);
    let wrong = page.post(
        &stream,
        &hello(),
        Some(&page.csrf.replace(|_: char| true, "0")),
    );
    let unknown = page.post("no-such-stream", &hello(), Some(&page.csrf));
    let accepted = page.post(&stream, &hello(), Some(&page.csrf));

    assert_eq!((missing, wrong, unknown, accepted), (403, 403, 404, 202));
    assert_eq!(
        events.message()["id"],
        json!(0),
        "only the accepted hello came"
    );
    let reasons: Vec<Value> = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == json!("endpoint_authentication_failed"))
        .map(|entry| entry["details"]["reason"].clone())
        .collect();
    assert_eq!(
        reasons,
        [json!("csrf_token_missing"), json!("csrf_token_wrong")]
    );
}

/// Covers: story:12-124
#[test]
fn the_event_stream_opens_only_for_a_sign_in() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let page = Page::signed_in(&env);
    let signed_out = Page {
        cookie: "goodfetch_console_other=1".to_owned(),
        ..page
    };

    let (status, _, _) = signed_out.events();

    assert_eq!(status, 401);
}
