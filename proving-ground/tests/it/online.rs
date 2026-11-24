//! The online copy (#168): `serve --copy online` reads its vendors from
//! `--online-config` and their secrets from the environment, carries each
//! configured vendor's challenge on `challenges.test`, and judges each on
//! the server side, recording the vendor's own verdict in the observations
//! the oracle reads: passed, failed, or inconclusive when the vendor cannot
//! say, never a pass. A token vendor's endpoint is a local stand-in; the
//! tests solve ALTCHA's challenges, sign Anubis's cookies and send the
//! edge's headers as those vendors would.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD, HEXLOWER};
use goodfetch_test_support::{LOOPBACK_PORT_0, TestEnv, UNSHAPED, serve_arguments};
use hmac::{Hmac, Mac as _};
use rstest::rstest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256, Sha512};

use crate::proving_ground::ProvingGround;

const SEED: u64 = 168;

const HOST: &str = "challenges.test";

/// What a stand-in received of one request: its `Content-Type` and body.
struct Received {
    content_type: String,
    body: String,
}

/// A stand-in for a vendor's verification endpoint: answers one request
/// with `status` and `body`, and hands over what it received.
struct StandIn {
    address: SocketAddr,
    received: Receiver<Received>,
}

impl StandIn {
    fn answering(status: u16, body: &str) -> Self {
        #[expect(
            clippy::disallowed_types,
            reason = "the stand-in is a vendor's endpoint the Proving ground calls"
        )]
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let address = listener.local_addr().expect("a bound address");
        let (sender, received) = mpsc::channel();
        let answer = format!(
            "HTTP/1.1 {status} Stand-in\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the Proving ground connects");
            let mut reader = BufReader::new(stream);
            let (mut content_type, mut length) = (String::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("a request head");
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    match name.to_ascii_lowercase().as_str() {
                        "content-type" => value.trim().clone_into(&mut content_type),
                        "content-length" => length = value.trim().parse().expect("a length"),
                        _ => {}
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("a request body");
            reader
                .get_mut()
                .write_all(answer.as_bytes())
                .expect("the answer is written");
            let body = String::from_utf8(body).expect("a UTF-8 body");
            let _ = sender.send(Received { content_type, body });
        });
        Self { address, received }
    }

    /// The endpoint's URL.
    fn url(&self) -> String {
        format!("http://{}/siteverify", self.address)
    }

    /// What the stand-in received.
    fn received(&self) -> Received {
        self.received
            .recv_timeout(Duration::from_secs(10))
            .expect("the Proving ground called the stand-in")
    }
}

/// A URL nothing listens on: a port bound, then released.
fn unreachable() -> String {
    #[expect(
        clippy::disallowed_types,
        reason = "the test finds a port nothing listens on"
    )]
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("a bound address");
    drop(listener);
    format!("http://{address}/siteverify")
}

/// The online copy's arguments: `serve`'s with `--copy online` and the
/// configuration `config` written in `env`'s folder.
fn online_arguments(env: &TestEnv, config: &str) -> Vec<String> {
    let path = env.folder().join("online.toml");
    std::fs::write(&path, config).expect("the configuration is written");
    let mut arguments = serve_arguments(SEED, LOOPBACK_PORT_0, UNSHAPED).to_vec();
    "online".clone_into(&mut arguments[4]);
    arguments.extend(["--online-config".to_owned(), path.display().to_string()]);
    arguments
}

/// The online copy, with one vendor's section and its secret.
fn online(env: &TestEnv, config: &str, secret: (&str, &str)) -> ProvingGround {
    ProvingGround::start_with_env(&online_arguments(env, config), &[secret])
}

/// The verdicts the observations record.
fn verdicts(proving_ground: &ProvingGround) -> Value {
    proving_ground.observations()["verdicts"].clone()
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::turnstile("turnstile", "cf-turnstile-response")]
#[case::recaptcha("recaptcha", "g-recaptcha-response")]
#[case::hcaptcha("hcaptcha", "h-captcha-response")]
fn a_token_the_vendor_accepts_is_a_passed_verdict(#[case] vendor: &str, #[case] field: &str) {
    let env = TestEnv::new();
    let stand_in = StandIn::answering(200, r#"{"success":true,"error-codes":[]}"#);
    let config = format!(
        "[{vendor}]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        stand_in.url()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    let page = proving_ground.get(&proving_ground.url(HOST, &format!("/{vendor}")));
    let answer = proving_ground.post_form(
        &proving_ground.url(HOST, &format!("/{vendor}/verify")),
        &[(field, "the-token")],
        None,
    );

    let received = stand_in.received();
    let form: Vec<(String, String)> = form_urlencoded::parse(received.body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(page.status, 200);
    assert!(page.body.contains("site-key"), "{}", page.body);
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(received.content_type, "application/x-www-form-urlencoded");
    assert!(
        form.contains(&("secret".to_owned(), "the-secret".to_owned())),
        "{form:?}"
    );
    assert!(
        form.contains(&("response".to_owned(), "the-token".to_owned())),
        "{form:?}"
    );
    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": vendor, "outcome": "passed" }])
    );
}

/// Covers: story:2-38, story:2-49
#[test]
fn hcaptcha_is_also_sent_its_site_key() {
    let env = TestEnv::new();
    let stand_in = StandIn::answering(200, r#"{"success":true}"#);
    let config = format!(
        "[hcaptcha]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        stand_in.url()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    proving_ground.post_form(
        &proving_ground.url(HOST, "/hcaptcha/verify"),
        &[("h-captcha-response", "the-token")],
        None,
    );

    let body = stand_in.received().body;
    assert!(body.contains("sitekey=site-key"), "{body}");
}

/// Covers: story:2-38, story:2-49
#[test]
fn cap_is_verified_with_a_json_request() {
    let env = TestEnv::new();
    let stand_in = StandIn::answering(200, r#"{"success":true}"#);
    let config = format!(
        "[cap]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        stand_in.url()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    proving_ground.post_form(
        &proving_ground.url(HOST, "/cap/verify"),
        &[("cap-token", "the-token")],
        None,
    );

    let received = stand_in.received();
    assert_eq!(received.content_type, "application/json");
    assert_eq!(
        serde_json::from_str::<Value>(&received.body).expect("a JSON body"),
        json!({ "secret": "the-secret", "response": "the-token" })
    );
    assert_eq!(verdicts(&proving_ground)[0]["outcome"], "passed");
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::refused_token(
    200,
    r#"{"success":false,"error-codes":["invalid-input-response"]}"#,
    "failed",
    "invalid-input-response"
)]
#[case::spent_token(
    200,
    r#"{"success":false,"error-codes":["timeout-or-duplicate"]}"#,
    "failed",
    "timeout-or-duplicate"
)]
#[case::wrong_secret(
    200,
    r#"{"success":false,"error-codes":["invalid-input-secret"]}"#,
    "inconclusive",
    "invalid-input-secret"
)]
#[case::vendor_error(
    200,
    r#"{"success":false,"error-codes":["internal-error"]}"#,
    "inconclusive",
    "internal-error"
)]
#[case::out_of_quota(429, "{}", "inconclusive", "out_of_quota")]
#[case::server_error(503, "{}", "inconclusive", "status_503")]
#[case::not_json(200, "<html>", "inconclusive", "malformed_answer")]
fn a_refusal_fails_and_what_the_vendor_cannot_say_is_inconclusive(
    #[case] status: u16,
    #[case] body: &str,
    #[case] outcome: &str,
    #[case] reason: &str,
) {
    let env = TestEnv::new();
    let stand_in = StandIn::answering(status, body);
    let config = format!(
        "[turnstile]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        stand_in.url()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    proving_ground.post_form(
        &proving_ground.url(HOST, "/turnstile/verify"),
        &[("cf-turnstile-response", "the-token")],
        None,
    );

    stand_in.received();
    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": "turnstile", "outcome": outcome, "reason": reason }])
    );
}

/// Covers: story:2-38, story:2-49
#[test]
fn an_unreachable_vendor_is_inconclusive() {
    let env = TestEnv::new();
    let config = format!(
        "[turnstile]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        unreachable()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    proving_ground.post_form(
        &proving_ground.url(HOST, "/turnstile/verify"),
        &[("cf-turnstile-response", "the-token")],
        None,
    );

    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": "turnstile", "outcome": "inconclusive", "reason": "unreachable" }])
    );
}

/// Covers: story:2-38, story:2-49
#[test]
fn a_missing_token_fails_without_asking_the_vendor() {
    let env = TestEnv::new();
    let config = format!(
        "[turnstile]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        unreachable()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    proving_ground.post_form(&proving_ground.url(HOST, "/turnstile/verify"), &[], None);

    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": "turnstile", "outcome": "failed", "reason": "missing_token" }])
    );
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::low_score(r#"{"success":true,"score":0.3}"#, json!({ "task": null, "vendor": "recaptcha", "outcome": "failed", "score": 0.3, "reason": "score_below_threshold" }))]
#[case::high_score(r#"{"success":true,"score":0.9}"#, json!({ "task": null, "vendor": "recaptcha", "outcome": "passed", "score": 0.9 }))]
fn a_recaptcha_score_below_the_threshold_fails(#[case] body: &str, #[case] verdict: Value) {
    let env = TestEnv::new();
    let stand_in = StandIn::answering(200, body);
    let config = format!(
        "[recaptcha]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\nmin_score = 0.5\n",
        stand_in.url()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    proving_ground.post_form(
        &proving_ground.url(HOST, "/recaptcha/verify"),
        &[("g-recaptcha-response", "the-token")],
        None,
    );

    stand_in.received();
    assert_eq!(verdicts(&proving_ground), json!([verdict]));
}

#[test]
fn a_vendor_not_configured_has_no_page() {
    let env = TestEnv::new();
    let config = format!(
        "[turnstile]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\nsiteverify = \"{}\"\n",
        unreachable()
    );
    let proving_ground = online(&env, &config, ("VENDOR_SECRET", "the-secret"));

    let page = proving_ground.get(&proving_ground.url(HOST, "/hcaptcha"));

    assert_eq!(page.status, 404);
}

#[test]
fn the_local_copy_carries_no_challenge() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&proving_ground.url(HOST, "/turnstile"));

    assert_eq!((page.status, verdicts(&proving_ground)), (404, json!([])));
}

/// What `serve` printed to stderr when it exited at once with `arguments`
/// in an environment of `variables` alone, failing.
fn refused(arguments: &[String], variables: &[(&str, &str)]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
        .args(arguments)
        .env_clear()
        .envs(variables.iter().copied())
        .stdin(Stdio::null())
        .output()
        .expect("the Proving ground runs");
    assert!(!output.status.success(), "serve started");
    assert!(output.stdout.is_empty(), "serve printed its line");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_configured_vendor_without_its_secret_is_refused_at_the_start() {
    let env = TestEnv::new();
    let arguments = online_arguments(
        &env,
        "[turnstile]\nsite_key = \"site-key\"\nsecret_env = \"VENDOR_SECRET\"\n",
    );

    let stderr = refused(&arguments, &[]);

    assert!(stderr.contains("VENDOR_SECRET"), "{stderr}");
}

#[test]
fn the_online_copy_needs_its_configuration_and_the_local_copy_takes_none() {
    let env = TestEnv::new();
    let mut online = serve_arguments(SEED, LOOPBACK_PORT_0, UNSHAPED).to_vec();
    "online".clone_into(&mut online[4]);
    let mut local = online_arguments(&env, "");
    "local".clone_into(&mut local[4]);

    let without = refused(&online, &[]);
    let with = refused(&local, &[]);

    assert!(without.contains("--online-config"), "{without}");
    assert!(with.contains("--online-config"), "{with}");
}

/// ALTCHA's configuration: a small `max_number`, so a test solves fast.
const ALTCHA: &str = "[altcha]\nsecret_env = \"ALTCHA_KEY\"\nmax_number = 1000\n";

fn hmac_sha256_hex(key: &str, text: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("a key of any length");
    mac.update(text.as_bytes());
    HEXLOWER.encode(&mac.finalize().into_bytes())
}

fn sha256_hex(text: &str) -> String {
    HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
}

/// The `altcha` field the widget posts: its solution as base64 JSON.
fn altcha_payload(challenge: &Value, number: u64) -> String {
    let solution = json!({
        "algorithm": challenge["algorithm"],
        "challenge": challenge["challenge"],
        "number": number,
        "salt": challenge["salt"],
        "signature": challenge["signature"],
    });
    BASE64.encode(solution.to_string().as_bytes())
}

/// Solves an ALTCHA challenge as its widget does: the number whose hash
/// with the salt is the challenge.
fn solve(challenge: &Value) -> u64 {
    let salt = challenge["salt"].as_str().expect("a salt");
    let target = challenge["challenge"].as_str().expect("a challenge");
    let most = challenge["maxnumber"].as_u64().expect("a maximum");
    (0..=most)
        .find(|number| sha256_hex(&format!("{salt}{number}")) == target)
        .expect("the challenge has a solution")
}

/// A challenge the test signs itself with ALTCHA's key, never issued.
fn forged_challenge(salt: &str, number: u64) -> Value {
    let challenge = sha256_hex(&format!("{salt}{number}"));
    json!({
        "algorithm": "SHA-256",
        "signature": hmac_sha256_hex("the-key", &challenge),
        "challenge": challenge,
        "salt": salt,
        "maxnumber": 1000,
    })
}

fn post_altcha(proving_ground: &ProvingGround, payload: &str) -> u16 {
    proving_ground
        .post_form(
            &proving_ground.url(HOST, "/altcha/verify"),
            &[("altcha", payload)],
            None,
        )
        .status
}

/// Covers: story:2-38, story:2-49
#[test]
fn an_altcha_challenge_the_copy_issued_and_the_tool_solved_passes_once() {
    let env = TestEnv::new();
    let proving_ground = online(&env, ALTCHA, ("ALTCHA_KEY", "the-key"));

    let page = proving_ground.get(&proving_ground.url(HOST, "/altcha"));
    let challenge = proving_ground
        .get(&proving_ground.url(HOST, "/altcha/challenge"))
        .json();
    let payload = altcha_payload(&challenge, solve(&challenge));
    let first = post_altcha(&proving_ground, &payload);
    let replayed = post_altcha(&proving_ground, &payload);

    let signed = challenge["challenge"].as_str().expect("a challenge");
    assert!(
        page.body.contains(r#"challengeurl="/altcha/challenge""#),
        "{}",
        page.body
    );
    assert_eq!(challenge["algorithm"], "SHA-256");
    assert_eq!(challenge["maxnumber"], 1000);
    assert_eq!(challenge["signature"], hmac_sha256_hex("the-key", signed));
    assert_eq!((first, replayed), (200, 200));
    assert_eq!(
        verdicts(&proving_ground),
        json!([
            { "task": null, "vendor": "altcha", "outcome": "passed" },
            { "task": null, "vendor": "altcha", "outcome": "failed", "reason": "unknown_challenge" },
        ])
    );
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::wrong_number("wrong_solution")]
#[case::another_key("bad_signature")]
#[case::expired("expired")]
#[case::never_issued("unknown_challenge")]
#[case::not_base64("malformed_solution")]
fn an_altcha_solution_that_does_not_hold_fails(#[case] reason: &str) {
    let env = TestEnv::new();
    let proving_ground = online(&env, ALTCHA, ("ALTCHA_KEY", "the-key"));
    let issued = proving_ground
        .get(&proving_ground.url(HOST, "/altcha/challenge"))
        .json();
    let number = solve(&issued);
    let payload = match reason {
        "wrong_solution" => altcha_payload(&issued, number + 1),
        "bad_signature" => {
            let mut other = issued.clone();
            other["signature"] = json!(hmac_sha256_hex("another-key", &sha256_hex("x")));
            altcha_payload(&other, number)
        }
        "expired" => altcha_payload(&forged_challenge("0a1b2c?expires=1", 7), 7),
        "unknown_challenge" => {
            let salt = format!("0a1b2c?expires={}", unix_seconds() + 600);
            altcha_payload(&forged_challenge(&salt, 7), 7)
        }
        _ => "not base64!".to_owned(),
    };

    post_altcha(&proving_ground, &payload);

    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": "altcha", "outcome": "failed", "reason": reason }])
    );
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// An HS512 JSON Web Token of `claims`, as Anubis signs its cookie, with
/// `key` and the header's algorithm `algorithm`.
fn jwt(key: &str, algorithm: &str, claims: &Value) -> String {
    let header = BASE64URL_NOPAD.encode(
        json!({ "alg": algorithm, "typ": "JWT" })
            .to_string()
            .as_bytes(),
    );
    let payload = BASE64URL_NOPAD.encode(claims.to_string().as_bytes());
    let mut mac = Hmac::<Sha512>::new_from_slice(key.as_bytes()).expect("a key of any length");
    mac.update(format!("{header}.{payload}").as_bytes());
    let signature = BASE64URL_NOPAD.encode(&mac.finalize().into_bytes());
    format!("{header}.{payload}.{signature}")
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::through_anubis("the-key", "HS512", 3600, json!({ "vendor": "anubis", "outcome": "passed" }))]
#[case::forged("another-key", "HS512", 3600, json!({ "vendor": "anubis", "outcome": "failed", "reason": "invalid_cookie" }))]
#[case::unsigned("the-key", "none", 3600, json!({ "vendor": "anubis", "outcome": "failed", "reason": "invalid_cookie" }))]
#[case::expired("the-key", "HS512", -60, json!({ "vendor": "anubis", "outcome": "failed", "reason": "expired_cookie" }))]
fn anubis_cookie_is_verified_with_its_key_and_expiry(
    #[case] key: &str,
    #[case] algorithm: &str,
    #[case] lifetime: i64,
    #[case] verdict: Value,
) {
    let env = TestEnv::new();
    let proving_ground = online(
        &env,
        "[anubis]\nsecret_env = \"ANUBIS_KEY\"\n",
        ("ANUBIS_KEY", "the-key"),
    );
    let now = i64::try_from(unix_seconds()).expect("a time in range");
    let cookie = jwt(
        key,
        algorithm,
        &json!({ "challenge": "c", "iat": now, "nbf": now - 60, "exp": now + lifetime }),
    );

    let page = proving_ground.get_with_cookie(
        &proving_ground.url(HOST, "/anubis"),
        Some(&format!("techaro.lol-anubis-auth={cookie}")),
    );

    let mut expected = json!({ "task": null });
    expected
        .as_object_mut()
        .expect("an object")
        .extend(verdict.as_object().expect("an object").clone());
    assert_eq!(page.status, 200);
    assert_eq!(verdicts(&proving_ground), json!([expected]));
}

/// Covers: story:2-38, story:2-49
#[test]
fn a_request_that_did_not_come_through_anubis_is_inconclusive() {
    let env = TestEnv::new();
    let proving_ground = online(
        &env,
        "[anubis]\nsecret_env = \"ANUBIS_KEY\"\n",
        ("ANUBIS_KEY", "the-key"),
    );

    proving_ground.get(&proving_ground.url(HOST, "/anubis"));

    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": "anubis", "outcome": "inconclusive", "reason": "not_through_anubis" }])
    );
}

/// The edge vendors, behind the edge whose secret is `the-edge-secret`.
const EDGE: &str =
    "[edge]\nsecret_env = \"EDGE_SECRET\"\n\n[cloudflare_bot_score]\n\n[aws_waf]\n\n[botid]\n";

const EDGE_HEADER: &str = "x-proving-ground-edge";

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::human_score("cloudflare_bot_score", "x-proving-ground-cf-bot-score", "85", json!({ "outcome": "passed", "score": 85.0 }))]
#[case::bot_score("cloudflare_bot_score", "x-proving-ground-cf-bot-score", "2", json!({ "outcome": "failed", "score": 2.0, "reason": "score_below_threshold" }))]
#[case::not_a_score("cloudflare_bot_score", "x-proving-ground-cf-bot-score", "high", json!({ "outcome": "inconclusive", "reason": "malformed_edge_verdict" }))]
#[case::no_bot_label("aws_waf", "x-amzn-waf-bot-control", "none", json!({ "outcome": "passed" }))]
#[case::bot_labels("aws_waf", "x-amzn-waf-bot-control", "awswaf:managed:aws:bot-control:signal:automated_browser", json!({ "outcome": "failed", "reason": "awswaf:managed:aws:bot-control:signal:automated_browser" }))]
#[case::human("botid", "x-proving-ground-botid", "human", json!({ "outcome": "passed" }))]
#[case::bot("botid", "x-proving-ground-botid", "bot", json!({ "outcome": "failed", "reason": "bot" }))]
#[case::bypassed("botid", "x-proving-ground-botid", "bypassed", json!({ "outcome": "inconclusive", "reason": "bypassed" }))]
fn an_edge_vendors_verdict_is_read_from_the_edges_header(
    #[case] vendor: &str,
    #[case] header: &str,
    #[case] value: &str,
    #[case] verdict: Value,
) {
    let env = TestEnv::new();
    let proving_ground = online(&env, EDGE, ("EDGE_SECRET", "the-edge-secret"));

    let page = proving_ground.get(&proving_ground.url(HOST, &format!("/{vendor}")));
    let answer = proving_ground.post_form_with_headers(
        &proving_ground.url(HOST, &format!("/{vendor}/verify")),
        &[],
        &[(EDGE_HEADER, "the-edge-secret"), (header, value)],
    );

    let mut expected = json!({ "task": null, "vendor": vendor });
    expected
        .as_object_mut()
        .expect("an object")
        .extend(verdict.as_object().expect("an object").clone());
    assert_eq!((page.status, answer.status), (200, 200));
    assert_eq!(verdicts(&proving_ground), json!([expected]));
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::no_edge_secret(None, Some("85"), "not_from_edge")]
#[case::wrong_edge_secret(Some("a-guess"), Some("85"), "not_from_edge")]
#[case::no_verdict(Some("the-edge-secret"), None, "no_edge_verdict")]
fn an_edge_verdict_the_edge_did_not_send_is_inconclusive(
    #[case] secret: Option<&str>,
    #[case] score: Option<&str>,
    #[case] reason: &str,
) {
    let env = TestEnv::new();
    let proving_ground = online(&env, EDGE, ("EDGE_SECRET", "the-edge-secret"));
    let headers: Vec<(&str, &str)> = secret
        .map(|secret| (EDGE_HEADER, secret))
        .into_iter()
        .chain(score.map(|score| ("x-proving-ground-cf-bot-score", score)))
        .collect();

    proving_ground.post_form_with_headers(
        &proving_ground.url(HOST, "/cloudflare_bot_score/verify"),
        &[],
        &headers,
    );

    assert_eq!(
        verdicts(&proving_ground),
        json!([{ "task": null, "vendor": "cloudflare_bot_score", "outcome": "inconclusive", "reason": reason }])
    );
}

#[rstest]
#[case::edge_vendor_without_edge("[botid]\n", "[edge]")]
#[case::score_outside_cloudflare(
    "[edge]\nsecret_env = \"EDGE_SECRET\"\n\n[aws_waf]\nmin_score = 30\n",
    "min_score"
)]
fn an_edge_configuration_that_cannot_hold_is_refused_at_the_start(
    #[case] config: &str,
    #[case] named: &str,
) {
    let env = TestEnv::new();
    let arguments = online_arguments(&env, config);

    let stderr = refused(&arguments, &[("EDGE_SECRET", "the-edge-secret")]);

    assert!(stderr.contains(named), "{stderr}");
}
