//! Opening the Console through Collie's executable (spec #592's Seam 1;
//! ADRs 0024, 0031, 0067 and 0069): `console --json` starts the major's
//! Daemon unless one runs and prints the sign-in link, whose fragment
//! carries the Daemon's token, opening no browser under `--no-browser`;
//! `POST /sign-in` swaps that token once for an `HttpOnly`,
//! `SameSite=Strict` cookie, and `GET /session` answers whether a request
//! carries one; the Console's WebSocket is a column of the endpoint table,
//! its handshake refusing a foreign `Host`, a foreign or missing `Origin`
//! and a missing or wrong cookie, each refusal checked in the Daemon's log
//! until the Audit log records it (area 4).
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the Daemon's log file itself"
)]

use std::collections::BTreeMap;
use std::net::SocketAddr;

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_support::{
    AuditUnreadable, ConsoleState, Endpoint, EndpointRunner, Listener, ListenerKind, Rule,
    RuntimeFile, TestEnv, cli,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::daemon::StopsTheDaemon;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Daemon's runtime file in `env`'s home.
fn runtime_file(env: &TestEnv) -> RuntimeFile {
    RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap()
}

/// The URL the Daemon serves its Console at.
fn console_url(env: &TestEnv) -> String {
    match runtime_file(env).console {
        ConsoleState::Serving(url) => url,
        other => panic!("the Daemon serves no Console: {other:?}"),
    }
}

/// An HTTP agent that reads every status as an answer.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// The Console's origin, `http://127.0.0.1:<port>`.
fn origin(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// The sign-in cookie's name of the Console at `url`.
fn cookie_name(url: &str) -> String {
    let port = origin(url).rsplit(':').next().unwrap();
    format!("goodfetch_console_{port}")
}

/// Swaps `token` for a sign-in at the Console at `url`, as its page does:
/// the status, and the `Set-Cookie` header if any.
fn sign_in(url: &str, token: &str) -> (u16, Option<String>) {
    let response = agent()
        .post(format!("{}/sign-in", origin(url)))
        .header("Origin", origin(url))
        .content_type("text/plain;charset=UTF-8")
        .send(token)
        .unwrap();
    let cookie = response
        .headers()
        .get("set-cookie")
        .map(|value| value.to_str().unwrap().to_owned());
    (response.status().as_u16(), cookie)
}

/// What `GET /session` answers a request carrying `cookie`, if any.
fn session(url: &str, cookie: Option<&str>) -> u16 {
    let mut request = agent().get(format!("{}/session", origin(url)));
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    request.call().unwrap().status().as_u16()
}

/// Covers: story:12-1, command:console
#[rstest]
#[case::daemon(true)]
#[case::no_daemon(false)]
fn console_json_prints_the_sign_in_link_and_opens_no_browser(#[case] running: bool) {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    if running {
        let started = cli(&env, COLLIE).args(["daemon", "start"]).run().unwrap();
        assert_eq!(started.exit, Some(0), "{}", started.stderr);
    }

    let opened = cli(&env, COLLIE)
        .args(["console", "--json", "--no-browser"])
        .json("ConsoleOpened")
        .unwrap();

    assert_eq!(opened.output.exit, Some(0), "{}", opened.output.stderr);
    let found = runtime_file(&env);
    assert_eq!(
        opened.documents[0]["url"],
        format!("{}#token={}", console_url(&env), found.token)
    );
    assert!(
        !opened.output.stderr.contains(&found.token),
        "the token is in no log: {}",
        opened.output.stderr
    );
    let mut settings = opened.output.settings();
    settings.add_filter("#token=[0-9a-f]{64}", "#token=[TOKEN]");
    settings.bind(|| insta::assert_snapshot!("console_json", opened.output.stdout));
}

/// Covers: story:12-1
#[test]
fn the_sign_in_link_swaps_its_token_once_for_an_http_only_cookie() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let opened = cli(&env, COLLIE)
        .args(["console", "--json", "--no-browser"])
        .json("ConsoleOpened")
        .unwrap();
    let link = opened.documents[0]["url"].as_str().unwrap().to_owned();
    let (url, token) = link.split_once("#token=").unwrap();

    let (status, cookie) = sign_in(url, token);

    assert_eq!(status, 204);
    let cookie = cookie.expect("the sign-in sets its cookie");
    let attributes: Vec<&str> = cookie.split("; ").collect();
    assert_eq!(&attributes[1..], ["HttpOnly", "SameSite=Strict", "Path=/"]);
    let (name, value) = attributes[0].split_once('=').unwrap();
    assert_eq!(name, cookie_name(url));
    assert_ne!(value, token, "the cookie is not the token");
    assert_eq!(session(url, Some(attributes[0])), 204);
    assert_eq!(session(url, None), 401, "a request with no cookie");
    assert_eq!(
        session(url, Some(&format!("{name}={token}"))),
        401,
        "the token is no cookie"
    );
}

/// Covers: story:12-1
#[test]
fn a_wrong_token_signs_nobody_in() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let started = cli(&env, COLLIE).args(["daemon", "start"]).run().unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    let url = console_url(&env);
    let wrong = runtime_file(&env).token.replace(|_: char| true, "0");

    assert_eq!(sign_in(&url, &wrong), (401, None));
    assert_eq!(sign_in(&url, ""), (401, None));
}

/// The endpoint table's reason each rule's refusal is logged with.
fn reason(rule: Rule) -> Option<&'static str> {
    Some(match rule {
        Rule::TokenMissing | Rule::TokenInQueryString => "token_missing",
        Rule::TokenWrong | Rule::TokenFromAnotherListener => "token_wrong",
        Rule::HostForeign => "host_foreign",
        Rule::OriginForeign => "origin_foreign",
        Rule::OriginMissing => "origin_missing",
        Rule::HelloNotFirst => "hello_not_first",
        Rule::ProbesAnswerStatusOnly => return None,
    })
}

/// Covers: story:12-1
#[test]
fn the_consoles_websocket_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let started = cli(&env, COLLIE).args(["daemon", "start"]).run().unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    let url = console_url(&env);
    let daemon_token = runtime_file(&env).token;
    let (_, cookie) = sign_in(&url, &daemon_token);
    let cookie = cookie.unwrap();
    let signed_in = cookie
        .split("; ")
        .next()
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.to_owned())
        .unwrap();
    let address: SocketAddr = origin(&url).trim_start_matches("http://").parse().unwrap();
    let hello = json!({
        "protocol_version": PROTOCOL_VERSION.to_string(),
        "surface": "console",
        "form": "catalog",
        "needs": "collie",
        "product_version": VERSION,
    });
    let Value::Object(hello) = hello else {
        unreachable!("the hello is an object")
    };
    let listener = Listener::new(
        ListenerKind::Console,
        Endpoint::SignedInWebSocket {
            address,
            cookie: cookie_name(&url),
            hello,
        },
        signed_in,
    )
    .with_other_listener_token(daemon_token);
    let log = env
        .home()
        .join("logs")
        .join(format!("goodfetch-collie-{}.jsonl", runtime_file(&env).pid));
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let verifier = move |_: ListenerKind, rule: Rule| {
        let Some(reason) = reason(rule) else {
            return Ok(false);
        };
        let text = std::fs::read_to_string(&log).map_err(AuditUnreadable::new)?;
        let refusals = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|record| {
                record["target"] == json!("goodfetch::rpc::refusal")
                    && record["fields"]["listener"] == json!("console")
                    && record["fields"]["reason"] == json!(reason)
            })
            .count();
        let earlier = seen.insert(reason, refusals).unwrap_or(0);
        Ok(refusals > earlier)
    };

    let ran = EndpointRunner::new(verifier).run(&listener);

    assert_eq!(ran, Ok(()));
}
