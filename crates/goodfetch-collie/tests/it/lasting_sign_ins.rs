//! Sign-ins that last, through Collie's executable (spec #592's Seam 1;
//! ADRs 0031 and 0098), each test with a Daemon and home of its own, its
//! Console on port 0: a sign-in's cookie, `HttpOnly` and `SameSite=Strict`,
//! lasts its idle expiry and outlives a Daemon restart; it ends unused past
//! the Operator policy's idle expiry, when revoked, and when the Console
//! key rotates; one address may try 10 times a minute; a signed-in `POST`
//! carries its CSRF token; and on a single-user Daemon the Audit log
//! records each action with the OS user and the surface `console`.
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own WebSocket to the Console, outside the engine (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes the Operator policy and waits out the short idle expiry it set"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::TcpStream;
use std::time::Duration;

use goodfetch_engine::home::Home;
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};
use tungstenite::handshake::client::generate_key;
use tungstenite::{Message, WebSocket};

use crate::daemon::StopsTheDaemon;
use crate::sign_in::{agent, console_url, origin, runtime_file};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Thirty days, in seconds.
const THIRTY_DAYS: u64 = 30 * 24 * 60 * 60;

/// A test's home whose Daemon runs as the OS user `ada`.
fn env() -> TestEnv {
    TestEnv::new().with_variable("USER", "ada")
}

/// Starts the Daemon of `env`'s home: the URL of its Console.
fn started(env: &TestEnv) -> String {
    let started = cli(env, COLLIE).args(["daemon", "start"]).run().unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    console_url(env)
}

/// Signs in through the link with the Daemon's token, as the Console's
/// page does: the status, and the `Set-Cookie` header if any.
fn signed_in(env: &TestEnv, url: &str) -> (u16, Option<String>) {
    crate::sign_in::sign_in(url, &runtime_file(env).token)
}

/// The cookie pair, `name=value`, a `Set-Cookie` header sets.
fn pair(set_cookie: &str) -> String {
    set_cookie.split("; ").next().unwrap().to_owned()
}

/// A sign-in's cookie pair through the link.
fn cookie(env: &TestEnv, url: &str) -> String {
    let (status, set) = signed_in(env, url);
    assert_eq!(status, 204);
    pair(&set.expect("the sign-in sets its cookie"))
}

/// What `GET /session` answers a request carrying `cookie`: its status, its
/// body as JSON, if any, and its `Set-Cookie` header, if any.
fn session(url: &str, cookie: &str) -> (u16, Value, Option<String>) {
    let mut response = agent()
        .get(format!("{}/session", origin(url)))
        .header("Cookie", cookie)
        .call()
        .unwrap();
    let set = response
        .headers()
        .get("set-cookie")
        .map(|value| value.to_str().unwrap().to_owned());
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::Null),
        set,
    )
}

/// `POST /sign-out` with `cookie` and, when given, the CSRF token: its
/// status and its `Set-Cookie` header, if any.
fn sign_out(url: &str, cookie: &str, csrf: Option<&str>) -> (u16, Option<String>) {
    let mut request = agent()
        .post(format!("{}/sign-out", origin(url)))
        .header("Origin", origin(url))
        .header("Cookie", cookie);
    if let Some(csrf) = csrf {
        request = request.header("X-CSRF-Token", csrf);
    }
    let response = request.send_empty().unwrap();
    let set = response
        .headers()
        .get("set-cookie")
        .map(|value| value.to_str().unwrap().to_owned());
    (response.status().as_u16(), set)
}

/// The Console's WebSocket, signed in with `cookie`, past its `hello`.
fn console_socket(url: &str, cookie: &str) -> WebSocket<TcpStream> {
    let authority = origin(url).trim_start_matches("http://").to_owned();
    let request = tungstenite::http::Request::builder()
        .uri(format!("ws://{authority}/protocol"))
        .header("Host", &authority)
        .header("Origin", origin(url))
        .header("Cookie", cookie)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .body(())
        .unwrap();
    let stream = TcpStream::connect(&authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let mut socket = tungstenite::client(request, stream).unwrap().0;
    let hello = json!({
        "protocol_version": PROTOCOL_VERSION.to_string(),
        "surface": "console",
        "form": "catalog",
        "needs": "collie",
        "product_version": VERSION,
    });
    let opened = call(&mut socket, 0, "hello", &hello);
    assert!(opened.get("result").is_some(), "{opened}");
    socket
}

/// The reply to calling `method` with `params` as the call `id`.
fn call(socket: &mut WebSocket<TcpStream>, id: i64, method: &str, params: &Value) -> Value {
    let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    socket.send(Message::text(message.to_string())).unwrap();
    loop {
        if let Message::Text(text) = socket.read().unwrap() {
            let reply: Value = serde_json::from_str(&text).unwrap();
            if reply["id"] == json!(id) {
                return reply;
            }
        }
    }
}

/// The identifier of the sign-in `cookie` carries: its value's first part.
fn id_of(cookie: &str) -> String {
    let (_, value) = cookie.split_once('=').unwrap();
    value.split_once('.').unwrap().0.to_owned()
}

/// Every Audit log entry of `env`'s home, as JSON.
pub(crate) fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let home = Home::new(env.home().to_path_buf());
    let Ok(connection) = Connection::open(home.audit_log()) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare("SELECT entry FROM entries ORDER BY sequence")
    else {
        return Vec::new();
    };
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| serde_json::from_str(&entry.unwrap()).unwrap())
        .collect()
}

/// Covers: story:12-5, story:12-110
#[test]
fn a_sign_in_lasts_its_idle_expiry_and_outlives_a_daemon_restart() {
    let env = env();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let (_, set) = signed_in(&env, &url);
    let set = set.expect("the sign-in sets its cookie");
    let cookie = pair(&set);

    let stopped = cli(&env, COLLIE).args(["daemon", "stop"]).run().unwrap();
    assert_eq!(stopped.exit, Some(0), "{}", stopped.stderr);
    let restarted = started(&env);
    let (status, _, refreshed) = session(&restarted, &cookie);

    let attributes: Vec<&str> = set.split("; ").skip(1).collect();
    assert_eq!(
        attributes,
        [
            "HttpOnly",
            "SameSite=Strict",
            "Path=/",
            &format!("Max-Age={THIRTY_DAYS}")
        ]
    );
    assert_eq!(status, 200, "the restarted Daemon still knows the sign-in");
    let refreshed = refreshed.expect("each use sets the cookie's expiry again");
    assert_eq!(pair(&refreshed), cookie);
    assert!(refreshed.contains("SameSite=Strict"), "{refreshed}");
}

/// Covers: story:12-5, setting:console.sign_in.idle_expiry
#[test]
fn a_sign_in_unused_past_the_idle_expiry_an_operator_policy_sets_ends() {
    let env = env();
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[settings.console.sign_in]\nidle_expiry = 1000\n",
    )
    .unwrap();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let cookie = cookie(&env, &url);

    // The short idle expiry the test set itself, waited out (ADR 0064).
    std::thread::sleep(Duration::from_millis(1500));

    assert_eq!(session(&url, &cookie).0, 401);
}

/// Covers: story:12-111, operation:sign_in.list, operation:sign_in.revoke, operation:console.rotate_key
#[test]
fn a_revoked_sign_in_ends_and_rotating_the_console_key_ends_them_all() {
    let env = env();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let mine = cookie(&env, &url);
    let lost = cookie(&env, &url);
    let mut socket = console_socket(&url, &mine);

    let listed = call(&mut socket, 1, "sign_in.list", &json!({}));
    let revoked = call(
        &mut socket,
        2,
        "sign_in.revoke",
        &json!({ "id": id_of(&lost) }),
    );

    let ids: Vec<&Value> = listed["result"]["sign_ins"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .iter()
        .map(|sign_in| &sign_in["id"])
        .collect();
    assert_eq!(ids, [&json!(id_of(&mine)), &json!(id_of(&lost))]);
    assert_eq!(revoked["result"], json!({ "revoked": true }), "{revoked}");
    assert_eq!(session(&url, &lost).0, 401, "the revoked sign-in");
    assert_eq!(session(&url, &mine).0, 200, "the other sign-in");

    let rotated = call(&mut socket, 3, "console.rotate_key", &json!({}));

    assert_eq!(rotated["result"], json!({ "signed_out": 1 }), "{rotated}");
    assert_eq!(session(&url, &mine).0, 401, "every sign-in ended");
    assert_eq!(
        cookie(&env, &url).len(),
        mine.len(),
        "signing in again works"
    );
}

/// Covers: story:12-110, setting:console.sign_in.rate
#[test]
fn the_eleventh_sign_in_attempt_in_a_minute_from_one_address_is_refused() {
    let env = env();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let token = runtime_file(&env).token;
    let wrong = token.replace(|_: char| true, "0");

    let attempts: Vec<u16> = (0..10)
        .map(|_| crate::sign_in::sign_in(&url, &wrong).0)
        .collect();
    let eleventh = crate::sign_in::sign_in(&url, &token);

    assert_eq!(attempts, [401; 10]);
    assert_eq!(eleventh, (429, None), "even with the right token");
    let last_refusal = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == json!("endpoint_authentication_failed"))
        .map(|entry| entry["details"]["reason"].clone())
        .next_back();
    assert_eq!(last_refusal, Some(json!("sign_in_rate_exceeded")));
}

/// Covers: story:12-110
#[test]
fn a_signed_in_post_without_its_csrf_token_is_refused() {
    let env = env();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let cookie = cookie(&env, &url);
    let (status, body, _) = session(&url, &cookie);
    let csrf = body["csrf_token"]
        .as_str()
        .unwrap_or_else(|| panic!("{body}"));

    let missing = sign_out(&url, &cookie, None);
    let wrong = sign_out(&url, &cookie, Some(&csrf.replace(|_: char| true, "0")));
    let signed_out = sign_out(&url, &cookie, Some(csrf));

    assert_eq!(status, 200);
    assert_eq!(missing, (403, None));
    assert_eq!(wrong, (403, None));
    assert_eq!(signed_out.0, 204);
    let cleared = signed_out.1.expect("signing out clears the cookie");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert_eq!(session(&url, &cookie).0, 401, "the sign-in ended");
}

/// Covers: story:12-109
#[test]
fn on_a_single_user_daemon_the_audit_log_records_the_os_user_and_the_console() {
    let env = env();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let mine = cookie(&env, &url);
    let other = cookie(&env, &url);
    let mut socket = console_socket(&url, &mine);

    let revoked = call(
        &mut socket,
        1,
        "sign_in.revoke",
        &json!({ "id": id_of(&other) }),
    );
    // An action outside the sign-in driver names the same person.
    let written = call(
        &mut socket,
        2,
        "config.user_write",
        &json!({ "changes": [{ "key": "timeout.navigation", "value": 45_000 }] }),
    );

    assert_eq!(revoked["result"], json!({ "revoked": true }), "{revoked}");
    assert!(written.get("result").is_some(), "{written}");
    let entries = audit_entries(&env);
    let actions: Vec<(&Value, &Value, &Value)> = entries
        .iter()
        .map(|entry| (&entry["kind"], &entry["actor"], &entry["surface"]))
        .collect();
    let ada = json!({ "kind": "person", "name": "ada" });
    let console = json!("console");
    assert_eq!(
        actions,
        [
            (&json!("signed_in"), &ada, &console),
            (&json!("signed_in"), &ada, &console),
            (&json!("sign_in_revoked"), &ada, &console),
            (&json!("user_configuration_written"), &ada, &console),
        ]
    );
    assert_eq!(entries[2]["details"]["sign_in"], json!(id_of(&other)));
}
