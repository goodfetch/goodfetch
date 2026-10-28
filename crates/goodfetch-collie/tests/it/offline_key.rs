//! The offline cache's key, through Collie's executable (spec #592's
//! `offline`; ADR 0031), each test with a Daemon and home of its own, its
//! Console on port 0: `GET /session` says whether `console.offline_cache`
//! is on; `POST /offline-key` gives a signed-in page a new cache key and its
//! wrapped form, and unwraps that form again only for the cookie of the
//! sign-in it belongs to, from the Console's own origin with its CSRF
//! token; once the sign-in ends, nothing unwraps it, so the cache stays
//! unreadable; and with the setting off no key is given.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

use crate::daemon::StopsTheDaemon;
use crate::sign_in::{agent, console_url, origin, runtime_file};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Starts the Daemon of `env`'s home: the URL of its Console.
fn started(env: &TestEnv) -> String {
    let started = cli(env, COLLIE).args(["daemon", "start"]).run().unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    console_url(env)
}

/// A sign-in's cookie pair through the link.
fn cookie(env: &TestEnv, url: &str) -> String {
    let (status, set) = crate::sign_in::sign_in(url, &runtime_file(env).token);
    assert_eq!(status, 204);
    set.expect("the sign-in sets its cookie")
        .split("; ")
        .next()
        .unwrap()
        .to_owned()
}

/// What `GET /session` answers `cookie`, as JSON.
fn session(url: &str, cookie: &str) -> Value {
    let mut response = agent()
        .get(format!("{}/session", origin(url)))
        .header("Cookie", cookie)
        .call()
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap()
}

/// What `POST /offline-key` answers `body` sent with `cookie`, when given,
/// and `csrf`, when given, from `from`: its status and its body as JSON, if
/// any.
fn offline_key(
    url: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    from: &str,
    body: &Value,
) -> (u16, Value) {
    let mut request = agent()
        .post(format!("{}/offline-key", origin(url)))
        .header("Origin", from)
        .header("Content-Type", "application/json");
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    if let Some(csrf) = csrf {
        request = request.header("X-CSRF-Token", csrf);
    }
    let mut response = request.send(body.to_string()).unwrap();
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// The CSRF token of the sign-in `cookie`.
fn csrf_of(url: &str, cookie: &str) -> String {
    session(url, cookie)["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Covers: story:12-126, setting:console.offline_cache
#[test]
fn a_sign_ins_offline_key_unwraps_for_its_cookie_alone_until_it_ends() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let own = cookie(&env, &url);
    let other = cookie(&env, &url);
    let csrf = csrf_of(&url, &own);
    let console = origin(&url).to_owned();

    let (status, created) = offline_key(&url, Some(&own), Some(&csrf), &console, &json!({}));
    let key = created["key"]
        .as_str()
        .unwrap_or_else(|| panic!("{created}"));
    let wrapped = created["wrapped"].as_str().unwrap();
    let again = json!({ "wrapped": wrapped });
    let unwrapped = offline_key(&url, Some(&own), Some(&csrf), &console, &again);
    let by_another = offline_key(
        &url,
        Some(&other),
        Some(&csrf_of(&url, &other)),
        &console,
        &again,
    );
    let without_csrf = offline_key(&url, Some(&own), None, &console, &again);
    let without_cookie = offline_key(&url, None, None, &console, &again);
    let foreign = offline_key(
        &url,
        Some(&own),
        Some(&csrf),
        "http://elsewhere.test",
        &again,
    );

    assert_eq!(session(&url, &own)["offline_cache"], json!(true));
    assert_eq!(status, 200);
    assert_eq!(key.len(), 43, "32 bytes in unpadded base64url: {key}");
    assert!(
        !wrapped.contains(key),
        "the wrapped form holds no clear key"
    );
    assert_eq!(unwrapped.0, 200);
    assert_eq!(unwrapped.1["key"], json!(key));
    assert_eq!(by_another.0, 422, "another sign-in's cookie opens nothing");
    assert_eq!(by_another.1, Value::Null);
    assert_eq!(without_csrf.0, 403);
    assert_eq!(without_cookie.0, 401);
    assert_eq!(foreign.0, 403);

    let signed_out = agent()
        .post(format!("{}/sign-out", origin(&url)))
        .header("Origin", &console)
        .header("Cookie", &own)
        .header("X-CSRF-Token", &csrf)
        .send_empty()
        .unwrap();
    assert_eq!(signed_out.status().as_u16(), 204);
    let ended = offline_key(&url, Some(&own), Some(&csrf), &console, &again);
    assert_eq!(ended.0, 401, "an ended sign-in unwraps nothing");
    let reasons: Vec<Value> = crate::lasting_sign_ins::audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == json!("endpoint_authentication_failed"))
        .map(|entry| entry["details"]["reason"].clone())
        .collect();
    assert_eq!(
        reasons,
        [
            json!("csrf_token_missing"),
            json!("token_missing"),
            json!("origin_foreign"),
            json!("token_wrong"),
        ],
        "each refusal is in the Audit log"
    );
}

/// Covers: story:12-126, setting:console.offline_cache
#[test]
fn with_console_offline_cache_off_no_offline_key_is_given() {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\nconsole.offline_cache = false\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let own = cookie(&env, &url);
    let csrf = csrf_of(&url, &own);

    let given = offline_key(&url, Some(&own), Some(&csrf), origin(&url), &json!({}));

    assert_eq!(session(&url, &own)["offline_cache"], json!(false));
    assert_eq!(given, (404, Value::Null));
}

/// `console.offline_cache` switched off while the Daemon runs takes effect
/// at the next request, with no restart: `GET /session` says so, and no
/// key is given any more.
///
/// Covers: story:12-126, setting:console.offline_cache
#[test]
fn console_offline_cache_switched_off_while_the_daemon_runs_takes_effect_at_once() {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let url = started(&env);
    let own = cookie(&env, &url);
    let csrf = csrf_of(&url, &own);
    let before = session(&url, &own)["offline_cache"].clone();

    env.user_configuration("console.address = \"127.0.0.1:0\"\nconsole.offline_cache = false\n");
    let given = offline_key(&url, Some(&own), Some(&csrf), origin(&url), &json!({}));

    assert_eq!(
        (before, session(&url, &own)["offline_cache"].clone(), given),
        (json!(true), json!(false), (404, Value::Null))
    );
}
