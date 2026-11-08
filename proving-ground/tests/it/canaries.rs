//! The `canaries` family: login flows built on the `login` family's pages
//! that plant a Secret canary of each kind (ADR 0067): a typed password, a
//! TOTP seed, a Session cookie and an endpoint token. A test plants one with
//! `POST /canaries` on the Proving ground's own address, which hands out its
//! value and the account that uses it; `GET /canaries` lists each canary by
//! its ID with where the Proving ground sent it and every request that
//! carried it, so a leak search tells a planted value from a leaked one.
//!
//! A tool that uses each canary only where its flow needs it is played by a
//! plain HTTP client that logs in as the plant answer says; a leaking tool by
//! one that also sends the value somewhere else.
//! Covers: story:2-83

use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission, totp_now};

const SEED: u64 = 312;

/// Plants a canary of `kind` and returns the plant answer.
fn plant(proving_ground: &ProvingGround, kind: &str) -> Value {
    let answer = proving_ground.post_control("/canaries", &json!({ "kind": kind }));
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

/// The canary `id` as `GET /canaries` lists it.
fn listed(proving_ground: &ProvingGround, id: &str) -> Value {
    let document = proving_ground.control("/canaries").json();
    document["canaries"]
        .as_array()
        .expect("`canaries` is an array")
        .iter()
        .find(|canary| canary["id"] == id)
        .unwrap_or_else(|| panic!("{id} is listed: {document}"))
        .clone()
}

fn value(planted: &Value) -> String {
    planted["value"].as_str().expect("a value").to_owned()
}

/// The URL of `path` on the host of the planted canary's task.
fn url(proving_ground: &ProvingGround, planted: &Value, path: &str) -> String {
    let task = planted["task"].as_str().expect("a task");
    let ordinal = task.strip_prefix("canaries/").expect("a canaries task");
    proving_ground.url(&format!("{ordinal}.canaries.test"), path)
}

/// Logs in as the plant answer's account and returns the session cookie.
fn log_in(proving_ground: &ProvingGround, planted: &Value) -> String {
    let account = &planted["account"];
    let login = proving_ground.post_form(
        &url(proving_ground, planted, "/login"),
        &[
            (
                "username",
                account["username"].as_str().expect("a username"),
            ),
            (
                "password",
                account["password"].as_str().expect("a password"),
            ),
        ],
        None,
    );
    assert_eq!(login.status, 303, "{}", login.body);
    let Some(secret) = account["totp_secret"].as_str() else {
        return login.cookie("session").expect("a session cookie");
    };
    let pending = login.cookie("pending").expect("a pending login's cookie");
    let mfa = proving_ground.post_form(
        &url(proving_ground, planted, "/mfa"),
        &[("code", &totp_now(secret))],
        Some(&pending),
    );
    assert_eq!(mfa.status, 303, "{}", mfa.body);
    mfa.cookie("session").expect("a session cookie")
}

/// The Record the account page shows.
fn read_record(page: &str) -> Value {
    let id = page
        .split("data-id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the page shows a Record's id");
    let word = page
        .split("<h1 class=\"word\">")
        .nth(1)
        .and_then(|rest| rest.split('<').next())
        .expect("the page shows a Record's word");
    json!({ "id": id, "word": word })
}

/// Logs in and reads the account page's Record, as a tool that keeps the
/// canary where its flow needs it does.
fn read_account(proving_ground: &ProvingGround, planted: &Value) -> Value {
    let session = log_in(proving_ground, planted);
    let page =
        proving_ground.get_with_cookie(&url(proving_ground, planted, "/account"), Some(&session));
    assert_eq!(page.status, 200, "{}", page.body);
    read_record(&page.body)
}

#[rstest]
#[case::a_typed_password("typed_password", "canaries/1")]
#[case::a_totp_seed("totp_seed", "canaries/2")]
#[case::a_session_cookie("session_cookie", "canaries/3")]
#[case::an_endpoint_token("endpoint_token", "canaries/4")]
fn each_plant_hands_out_a_new_value_listed_by_its_id(#[case] kind: &str, #[case] task: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let first = plant(&proving_ground, kind);
    let second = plant(&proving_ground, kind);

    assert_eq!(first["task"], task);
    assert_eq!(first["kind"], kind);
    assert_ne!(first["id"], second["id"]);
    assert_ne!(first["value"], second["value"]);
    let canary = listed(&proving_ground, first["id"].as_str().expect("an id"));
    assert_eq!(canary["value"], first["value"]);
    assert_eq!(canary["sent"], json!([{ "to": "test", "via": "plant" }]));
    assert_eq!(canary["sightings"], json!([]));
}

#[test]
fn an_unknown_kind_is_not_planted() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.post_control("/canaries", &json!({ "kind": "api_key" }));

    assert_eq!(answer.status, 400, "{}", answer.body);
}

#[rstest]
#[case::a_typed_password("typed_password")]
#[case::a_totp_seed("totp_seed")]
#[case::a_session_cookie("session_cookie")]
fn a_tool_that_keeps_each_canary_where_its_login_needs_it_is_verified(#[case] kind: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let planted = plant(&proving_ground, kind);
    let task = planted["task"].as_str().expect("a task").to_owned();

    let record = read_account(&proving_ground, &planted);
    let answer = proving_ground.judge(&submission(&task, "success", &[record]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    let canary = listed(&proving_ground, planted["id"].as_str().expect("an id"));
    let sightings = canary["sightings"].as_array().expect("sightings");
    assert!(
        sightings
            .iter()
            .all(|sighting| sighting["expected"] == true),
        "{canary}"
    );
}

#[test]
fn a_typed_password_is_seen_where_the_login_form_sends_it() {
    let proving_ground = ProvingGround::start(SEED);
    let planted = plant(&proving_ground, "typed_password");

    log_in(&proving_ground, &planted);

    let canary = listed(&proving_ground, planted["id"].as_str().expect("an id"));
    assert_eq!(
        canary["sightings"],
        json!([{
            "host": "1.canaries.test",
            "target": "/login",
            "part": "body",
            "expected": true,
        }])
    );
}

#[test]
fn a_session_cookie_is_recorded_where_it_was_set() {
    let proving_ground = ProvingGround::start(SEED);
    let planted = plant(&proving_ground, "session_cookie");

    let session = log_in(&proving_ground, &planted);

    assert_eq!(session, format!("session={}", value(&planted)));
    let canary = listed(&proving_ground, planted["id"].as_str().expect("an id"));
    assert_eq!(
        canary["sent"],
        json!([
            { "to": "test", "via": "plant" },
            { "to": "3.canaries.test", "via": "set_cookie" },
        ])
    );
}

#[test]
fn an_endpoint_token_opens_the_endpoint_only_as_a_bearer_token() {
    let proving_ground = ProvingGround::start(SEED);
    let planted = plant(&proving_ground, "endpoint_token");
    let endpoint = url(&proving_ground, &planted, "/api/record");

    let refused = proving_ground.get(&endpoint);
    let answered = proving_ground.get_with_headers(
        &endpoint,
        &[("authorization", &format!("Bearer {}", value(&planted)))],
    );
    let answer = proving_ground.judge(&submission("canaries/4", "success", &[answered.json()]));

    assert_eq!(refused.status, 401, "{}", refused.body);
    assert_eq!(answered.status, 200, "{}", answered.body);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::a_password_in_a_query("typed_password", "query")]
#[case::a_totp_seed_in_a_query("totp_seed", "query")]
#[case::a_session_cookie_in_a_header("session_cookie", "header")]
#[case::a_token_in_a_query("endpoint_token", "query")]
#[case::a_token_in_hex("endpoint_token", "hex")]
#[case::a_password_in_base64("typed_password", "base64")]
fn a_canary_sent_anywhere_else_is_a_leak_its_task_fails_on(#[case] kind: &str, #[case] how: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let planted = plant(&proving_ground, kind);
    let task = planted["task"].as_str().expect("a task").to_owned();
    let secret = value(&planted);
    let elsewhere = proving_ground.url("catalog.test", "/");

    match how {
        "query" => {
            proving_ground.get(&format!("{elsewhere}?note={secret}"));
        }
        "header" => {
            proving_ground.get_with_headers(&elsewhere, &[("x-note", &secret)]);
        }
        "hex" => {
            let hex = data_encoding::HEXLOWER.encode(secret.as_bytes());
            proving_ground.post_json(&elsewhere, &json!({ "note": hex }));
        }
        _ => {
            let base64 = data_encoding::BASE64.encode(secret.as_bytes());
            proving_ground.post_json(&elsewhere, &json!({ "note": base64 }));
        }
    }
    let answer = proving_ground.judge(&submission(&task, "success", &[]));

    let canary = listed(&proving_ground, planted["id"].as_str().expect("an id"));
    assert_eq!(canary["sightings"][0]["host"], "catalog.test", "{canary}");
    assert_eq!(canary["sightings"][0]["expected"], false, "{canary}");
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["findings"][0]["check"], "secret_canary_leaked",
        "{answer}"
    );
}

#[rstest]
#[case::a_typed_password(1)]
#[case::a_totp_seed(2)]
#[case::a_session_cookie(3)]
fn a_login_whose_canary_is_not_planted_is_not_open(#[case] ordinal: u32) {
    let proving_ground = ProvingGround::start(SEED);

    let login = proving_ground.post_form(
        &proving_ground.url(&format!("{ordinal}.canaries.test"), "/login"),
        &[("username", "anyone"), ("password", "anything")],
        None,
    );

    assert_eq!(login.status, 409, "{}", login.body);
    assert_eq!(login.json()["error"], "canary_not_planted");
}
