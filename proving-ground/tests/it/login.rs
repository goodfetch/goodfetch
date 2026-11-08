//! The `login` family: orders behind a login, with the accounts the task list
//! names, derived from the seed (ADR 0063). One account logs in with a
//! password and a TOTP code (RFC 6238); the other's login expires after
//! three pages, so a crawl logs in again to read every order.
//!
//! The known-correct results come from logging in as the task's account over
//! plain HTTP, its TOTP code computed here from the account's secret, and
//! reading each order page, logging in again whenever the site asks.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac as _};
use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};
use sha1::Sha1;

use crate::proving_ground::{Answer, ProvingGround, submission};

const SEED: u64 = 166;
const HOST: &str = "login.test";

/// The TOTP code (RFC 6238: HMAC-SHA-1, 30-second steps, 6 digits) of a
/// base32 secret at `unix_seconds`.
fn totp(secret: &str, unix_seconds: u64) -> String {
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.as_bytes())
        .expect("the secret is base32");
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).expect("HMAC takes any key");
    mac.update(&(unix_seconds / 30).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 0x0f);
    let code =
        u32::from_be_bytes(hash[offset..offset + 4].try_into().expect("four bytes")) & 0x7fff_ffff;
    format!("{:06}", code % 1_000_000)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970")
        .as_secs()
}

fn account(task: &Value) -> (String, String, Option<String>) {
    let account = &task["account"];
    (
        account["username"].as_str().expect("a username").to_owned(),
        account["password"].as_str().expect("a password").to_owned(),
        account["totp_secret"].as_str().map(str::to_owned),
    )
}

fn post_password(proving_ground: &ProvingGround, username: &str, password: &str) -> Answer {
    proving_ground.post_form(
        &proving_ground.url(HOST, "/login"),
        &[("username", username), ("password", password)],
        None,
    )
}

/// Logs in as the task's account, its TOTP code included when it has MFA,
/// and returns the session's cookie.
fn log_in(proving_ground: &ProvingGround, task: &Value) -> String {
    let (username, password, secret) = account(task);
    let login = post_password(proving_ground, &username, &password);
    assert_eq!(login.status, 303, "{}", login.body);
    let Some(secret) = secret else {
        return login.cookie("session").expect("a session cookie");
    };
    let pending = login.cookie("pending").expect("a pending login's cookie");
    let mfa = proving_ground.post_form(
        &proving_ground.url(HOST, "/mfa"),
        &[("code", &totp(&secret, now()))],
        Some(&pending),
    );
    assert_eq!(mfa.status, 303, "{}", mfa.body);
    mfa.cookie("session").expect("a session cookie")
}

/// A page behind the login, logging in again when the site sends to its
/// login page.
fn read(proving_ground: &ProvingGround, task: &Value, session: &mut String, path: &str) -> String {
    let url = proving_ground.url(HOST, path);
    let mut page = proving_ground.get_with_cookie(&url, Some(session));
    if page.status == 303 {
        *session = log_in(proving_ground, task);
        page = proving_ground.get_with_cookie(&url, Some(session));
    }
    assert_eq!(page.status, 200, "{path}: {}", page.body);
    page.body
}

fn capture(text: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .unwrap_or_else(|| panic!("{pattern} in {text}"))[1]
        .to_owned()
}

fn order_record(page: &str) -> Value {
    json!({
        "id": capture(page, r#"<article class="order" data-id="([^"]+)""#),
        "item": capture(page, r#"<h1 class="item">([^<]+)</h1>"#),
        "total": capture(page, r#"data-total="([^"]+)""#),
    })
}

fn order_paths(account_page: &str) -> Vec<String> {
    Regex::new(r#"href="(/account/orders/[^"]+)""#)
        .expect("a valid pattern")
        .captures_iter(account_page)
        .map(|link| link[1].to_owned())
        .collect()
}

/// The Records of every order of the task's account.
fn read_orders(proving_ground: &ProvingGround, task: &Value) -> Vec<Value> {
    let mut session = log_in(proving_ground, task);
    let account_page = read(proving_ground, task, &mut session, "/account");
    order_paths(&account_page)
        .iter()
        .map(|path| order_record(&read(proving_ground, task, &mut session, path)))
        .collect()
}

/// Covers: story:2-31, story:2-33
#[rstest]
#[case::with_mfa("login/1")]
#[case::that_expires("login/2")]
fn a_correct_result_behind_a_login_is_verified(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let records = read_orders(&proving_ground, &proving_ground.task(id));

    let answer = proving_ground.judge(&submission(id, "success", &records));

    assert_eq!(answer["task"], id);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-32, story:2-33
#[test]
fn a_crawl_behind_a_login_missing_one_order_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = read_orders(&proving_ground, &proving_ground.task("login/1"));
    let dropped = records.remove(2);

    let answer = proving_ground.judge(&submission("login/1", "needs-review", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

/// Covers: story:2-4, story:2-33
#[test]
fn a_crawl_that_stops_when_its_login_expires_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = proving_ground.task("login/2");
    let session = log_in(&proving_ground, &task);
    let account_page = proving_ground
        .get_with_cookie(&proving_ground.url(HOST, "/account"), Some(&session))
        .body;
    let records: Vec<Value> = order_paths(&account_page)
        .iter()
        .map(|path| proving_ground.get_with_cookie(&proving_ground.url(HOST, path), Some(&session)))
        .take_while(|page| page.status == 200)
        .map(|page| order_record(&page.body))
        .collect();

    let answer = proving_ground.judge(&submission("login/2", "success", &records));

    assert_eq!(answer["task"], "login/2");
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!(["O-03", "O-04", "O-05", "O-06"]));
}

/// Covers: story:2-4, story:2-33
#[test]
fn a_success_that_read_the_login_page_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission("login/1", "success", &[]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"].as_array().unwrap().len(), 6, "{answer}");
}

/// Covers: story:2-33
#[test]
fn the_account_page_without_a_login_sends_to_the_login_page() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&proving_ground.url(HOST, "/account"));

    assert_eq!(page.status, 303, "{}", page.body);
    assert_eq!(page.header("location"), Some("/login"));
}

/// Covers: story:2-33
#[test]
fn the_login_page_has_a_form() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&proving_ground.url(HOST, "/login"));

    assert_eq!(page.status, 200, "{}", page.body);
    assert!(
        page.body
            .contains(r#"<form method="post" action="/login">"#),
        "{}",
        page.body
    );
}

/// Covers: story:2-33
#[test]
fn a_wrong_password_opens_no_session() {
    let proving_ground = ProvingGround::start(SEED);
    let (username, _, _) = account(&proving_ground.task("login/1"));

    let login = post_password(&proving_ground, &username, "not-the-password");

    assert_eq!(login.status, 401, "{}", login.body);
    assert_eq!(login.cookie("session"), None);
    assert_eq!(login.cookie("pending"), None);
}

/// Covers: story:2-33
#[test]
fn a_password_alone_opens_no_session_for_an_account_with_mfa() {
    let proving_ground = ProvingGround::start(SEED);
    let (username, password, _) = account(&proving_ground.task("login/1"));
    let login = post_password(&proving_ground, &username, &password);
    let pending = login.cookie("pending").expect("a pending login's cookie");

    let page =
        proving_ground.get_with_cookie(&proving_ground.url(HOST, "/account"), Some(&pending));

    assert_eq!(login.header("location"), Some("/mfa"));
    assert_eq!(login.cookie("session"), None);
    assert_eq!(page.status, 303, "{}", page.body);
}

/// Covers: story:2-33
#[test]
fn a_wrong_totp_code_opens_no_session() {
    let proving_ground = ProvingGround::start(SEED);
    let (username, password, secret) = account(&proving_ground.task("login/1"));
    let pending = post_password(&proving_ground, &username, &password)
        .cookie("pending")
        .expect("a pending login's cookie");
    // Every digit of the current code moved by five: never the code of this
    // step, and the code of a step next to it only by a one in 10^6 chance.
    let wrong: String = totp(&secret.unwrap(), now())
        .chars()
        .map(|digit| {
            let digit = u8::try_from(digit.to_digit(10).unwrap()).unwrap();
            char::from(b'0' + (digit + 5) % 10)
        })
        .collect();

    let mfa = proving_ground.post_form(
        &proving_ground.url(HOST, "/mfa"),
        &[("code", &wrong)],
        Some(&pending),
    );

    assert_eq!(mfa.status, 401, "{}", mfa.body);
    assert_eq!(mfa.cookie("session"), None);
}

/// Covers: story:2-33
#[test]
fn an_expiring_login_sends_its_fourth_page_to_the_login_page() {
    let proving_ground = ProvingGround::start(SEED);
    let session = log_in(&proving_ground, &proving_ground.task("login/2"));
    let url = proving_ground.url(HOST, "/account");
    let statuses: Vec<u16> = (0..3)
        .map(|_| proving_ground.get_with_cookie(&url, Some(&session)).status)
        .collect();

    let fourth = proving_ground.get_with_cookie(&url, Some(&session));

    assert_eq!(statuses, [200, 200, 200]);
    assert_eq!(fourth.status, 303, "{}", fourth.body);
    assert_eq!(fourth.header("location"), Some("/login"));
}

/// Covers: story:2-33
#[test]
fn each_seed_has_its_own_accounts() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);

    let accounts = (
        account(&first.task("login/1")),
        account(&second.task("login/1")),
    );

    assert_ne!(accounts.0.0, accounts.1.0);
    assert_ne!(accounts.0.1, accounts.1.1);
    assert_ne!(accounts.0.2, accounts.1.2);
}
