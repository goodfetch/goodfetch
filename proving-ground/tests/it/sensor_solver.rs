//! The Hyper Solutions stand-in (#1539, ADRs 0035 and 0076): its Akamai
//! sensor endpoint answers in the service's own shapes with sensor data the
//! Proving ground's sensor page accepts, as the account of the call's key
//! was told, and every call is recorded.
//!
//! Each test is a plain HTTP client of the service and of the page.
//! Covers: story:2-33

use std::time::Duration;

use rstest::rstest;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::proving_ground::{Answer, ProvingGround, submission};

const SEED: u64 = 1539;

/// The task whose page takes sensor data.
const TASK: &str = "protection/4";

const HOST: &str = "hyper-solutions.test";
const ENDPOINT: &str = "/v2/sensor";
const PRICE: &str = "0.0010";

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// One client of the page and of the service, with an account of its own.
struct Client<'a> {
    proving_ground: &'a ProvingGround,
    account: String,
    key: String,
    /// The page's URL.
    url: String,
    /// The script's URL and text.
    script_url: String,
    script: String,
    /// The `name=value` pairs of the cookies the page set.
    abck: String,
    bmsz: String,
}

fn value(pair: &str) -> &str {
    pair.split_once('=').expect("a cookie pair").1
}

impl<'a> Client<'a> {
    /// A client that asked for the page, whose account was told `told`.
    fn new(proving_ground: &'a ProvingGround, told: &Value) -> Self {
        let answer = proving_ground.post_control(
            "/solver-accounts",
            &json!({ "service": "hyper_solutions", "told": told }),
        );
        assert_eq!(answer.status, 200, "{}", answer.body);
        let account = answer.json();
        let text = |field: &str| account[field].as_str().expect(field).to_owned();
        let url = proving_ground.task(TASK)["start_url"]
            .as_str()
            .expect("a start URL")
            .to_owned();
        let page = proving_ground.get(&url);
        assert_eq!(page.status, 403, "{}", page.body);
        let path = page
            .body
            .split_once("<script src=\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .expect("the page's script")
            .0;
        let origin = url.strip_suffix("/sensor").expect("the page's path");
        let script_url = format!("{origin}{path}");
        Self {
            proving_ground,
            account: text("account"),
            key: text("key"),
            script: proving_ground.get(&script_url).body,
            script_url,
            abck: page.cookie("_abck").expect("an `_abck` cookie"),
            bmsz: page.cookie("bm_sz").expect("a `bm_sz` cookie"),
            url,
        }
    }

    /// The body of a first call for the page.
    fn body(&self) -> Value {
        json!({
            "abck": value(&self.abck),
            "bmsz": value(&self.bmsz),
            "version": "3",
            "pageUrl": self.url,
            "scriptUrl": self.script_url,
            "userAgent": USER_AGENT,
            "ip": "203.0.113.7",
            "acceptLanguage": "en-US,en;q=0.9",
            "script": self.script,
        })
    }

    /// The body of a first call with `fields` changed, a `null` one removed.
    fn changed(&self, fields: &Value) -> Value {
        let mut body = self.body();
        let kept = body.as_object_mut().expect("an object");
        for (name, value) in fields.as_object().expect("an object") {
            if value.is_null() {
                kept.remove(name);
            } else {
                kept.insert(name.clone(), value.clone());
            }
        }
        body
    }

    fn call(&self, path: &str, body: &Value) -> Answer {
        self.proving_ground.post_json_with_headers(
            &self.proving_ground.url(HOST, path),
            &[("x-api-key", &self.key)],
            body,
        )
    }

    /// The service's answer to a first call with `fields` changed.
    fn sensor(&self, fields: &Value) -> Answer {
        self.call(ENDPOINT, &self.changed(fields))
    }

    /// Posts `sensor_data` to the page's script, as its browser would.
    fn post(&self, sensor_data: &Value, user_agent: &str) -> Answer {
        let cookies = format!("{}; {}", self.abck, self.bmsz);
        self.proving_ground.post_json_with_headers(
            &self.script_url,
            &[("cookie", &cookies), ("user-agent", user_agent)],
            &json!({ "sensor_data": sensor_data }),
        )
    }

    /// The account's calls, as the stand-in recorded them.
    fn recorded(&self) -> Vec<Value> {
        calls(self.proving_ground)
            .into_iter()
            .filter(|call| call["account"] == self.account.as_str())
            .collect()
    }
}

/// Every call the stand-ins recorded.
fn calls(proving_ground: &ProvingGround) -> Vec<Value> {
    proving_ground.control("/solver-calls").json()["calls"]
        .as_array()
        .expect("`calls` is an array")
        .clone()
}

fn hyper_error(code: &str, text: &str) -> Value {
    json!({
        "error": text,
        "code": code,
        "docs": format!("https://docs.hypersolutions.co/api-reference/errors#{code}"),
    })
}

#[test]
fn the_page_accepts_the_sensor_data_the_service_answers() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));

    let answer = client.sensor(&json!({}));
    let solved = answer.json();
    let accepted = client.post(&solved["payload"], USER_AGENT);
    let abck = accepted.cookie("_abck").expect("an `_abck` cookie");
    let page = proving_ground.get_with_cookie(&client.url, Some(&abck));
    let between = |before: &str, after: &str| {
        let rest = page.body.split_once(before).expect(before).1;
        rest.split_once(after).expect(after).0.to_owned()
    };
    let record = json!({
        "id": between("data-id=\"", "\""),
        "title": between("<h1 class=\"title\">", "</h1>"),
    });
    let judged = proving_ground.judge(&submission(TASK, "success", &[record]));

    assert_eq!(answer.status, 200, "{solved}");
    assert!(
        solved["headers"]["sec-ch-ua"].is_string() && solved["context"].is_string(),
        "{solved}"
    );
    assert_eq!(accepted.status, 201);
    assert_eq!(accepted.json(), json!({ "success": true }));
    assert!(abck.contains("~0~"), "{abck}");
    assert_eq!(judged["outcome"], "verified", "{judged}");
}

#[test]
fn a_later_call_takes_the_context_in_place_of_the_script() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));
    let first = client.sensor(&json!({})).json();

    let later = client.sensor(&json!({ "script": null, "context": first["context"] }));
    let payload = later.json()["payload"].clone();

    assert_eq!(later.status, 200, "{}", later.body);
    assert_ne!(payload, first["payload"]);
    let accepted = client.post(&payload, USER_AGENT);
    assert_eq!(accepted.json(), json!({ "success": true }));
}

#[test]
fn a_hash_of_the_pages_script_stands_for_it() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));
    let hash = data_encoding::HEXLOWER.encode(&Sha256::digest(&client.script));

    let answer = client.sensor(&json!({ "script": null, "scriptHash": hash }));
    let accepted = client.post(&answer.json()["payload"], USER_AGENT);

    assert_eq!(accepted.json(), json!({ "success": true }));
}

#[rstest]
#[case::another_page(json!({ "pageUrl": "http://elsewhere.test/sensor" }))]
#[case::another_user_agent(json!({ "userAgent": "curl/8.0" }))]
#[case::another_abck(json!({ "abck": "0~-1~YAAQ~-1~-1~-1" }))]
#[case::another_bmsz(json!({ "bmsz": "0~YAAQ" }))]
#[case::another_script(json!({ "script": "var bmak = {};" }))]
fn the_page_refuses_sensor_data_made_for_something_else(#[case] fields: Value) {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));

    let payload = client.sensor(&fields).json()["payload"].clone();
    let refused = client.post(&payload, USER_AGENT);
    let abck = refused.cookie("_abck").expect("an `_abck` cookie");
    let page = proving_ground.get_with_cookie(&client.url, Some(&abck));

    assert!(payload.is_string(), "{payload}");
    assert_eq!(refused.status, 201);
    assert_eq!(refused.json(), json!({ "success": false }));
    assert_eq!(page.status, 403);
}

#[rstest]
#[case::another_user_agent(None, "curl/8.0")]
#[case::made_up(Some("3;0;1;0;1;made-up"), USER_AGENT)]
fn the_page_refuses_sensor_data_its_browser_did_not_send(
    #[case] made_up: Option<&str>,
    #[case] user_agent: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));
    let payload = client.sensor(&json!({})).json()["payload"].clone();
    let sent = made_up.map_or(payload, Value::from);

    let refused = client.post(&sent, user_agent);

    assert_eq!(refused.json(), json!({ "success": false }));
}

#[test]
fn the_pages_refusal_committed_as_a_record_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let record = json!({ "id": "T-0008", "title": "Access Denied" });

    let judged = proving_ground.judge(&submission(TASK, "success", &[record]));

    assert_eq!(judged["outcome"], "false_success", "{judged}");
}

#[test]
fn a_product_the_service_does_not_support_is_refused_with_no_price() {
    let proving_ground = ProvingGround::start(SEED);
    let solve = Client::new(&proving_ground, &json!("solve"));
    let told = Client::new(&proving_ground, &json!("unsupported"));
    let expected = hyper_error(
        "product_not_enabled",
        "The account has no access to this product.",
    );

    let unsupported = solve.call("/kasada/payload", &json!({}));
    let told_unsupported = told.sensor(&json!({}));

    for answer in [unsupported, told_unsupported] {
        assert_eq!((answer.status, answer.json()), (403, expected.clone()));
    }
    for (client, kind) in [(&solve, "kasada/payload"), (&told, "v2/sensor")] {
        let recorded = client.recorded();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(
            (&recorded[0]["kind"], &recorded[0]["price"]),
            (&json!(kind), &Value::Null)
        );
    }
}

/// Every consumer variant (#1086) that an answer tells, with its status.
#[rstest]
#[case::no_balance("no_plan_or_balance", 402)]
#[case::exhausted("balance_exhausted", 402)]
#[case::bad_key("invalid_api_key", 401)]
#[case::disabled_key("api_key_disabled", 403)]
#[case::refused_domain("domain_not_whitelisted", 403)]
#[case::refused("access_denied", 403)]
#[case::unknown("some_new_failure", 500)]
fn a_service_told_to_fail_a_call_answers_the_code_in_its_shape(
    #[case] code: &str,
    #[case] status: u16,
) {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!({ "create_error": code }));

    let answer = client.sensor(&json!({}));
    let recorded = client.recorded();

    let body = answer.json();
    assert_eq!((answer.status, &body["code"]), (status, &json!(code)));
    assert_eq!(
        body["docs"],
        format!("https://docs.hypersolutions.co/api-reference/errors#{code}")
    );
    assert!(body["error"].is_string(), "{body}");
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["error"], body);
    assert_eq!(recorded[0]["price"], Value::Null);
}

#[test]
fn a_call_with_no_accounts_key_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));
    let url = proving_ground.url(HOST, ENDPOINT);
    let mut unknown = client.key.clone();
    let last = unknown.pop().expect("a key");
    unknown.push(if last == '0' { '1' } else { '0' });

    let answers = [
        vec![],
        vec![("x-api-key", "not-a-uuid")],
        vec![("x-api-key", unknown.as_str())],
    ]
    .map(|headers| {
        let answer = proving_ground.post_json_with_headers(&url, &headers, &client.body());
        (answer.status, answer.json()["code"].clone())
    });

    assert_eq!(
        answers,
        ["missing_api_key", "malformed_api_key", "invalid_api_key"].map(|code| (401, json!(code)))
    );
    let accounts: Vec<Value> = calls(&proving_ground)
        .into_iter()
        .map(|mut call| call["account"].take())
        .collect();
    assert_eq!(accounts, [const { Value::Null }; 3]);
}

#[rstest]
#[case::no_abck(json!({ "abck": null }), 400)]
#[case::no_page(json!({ "pageUrl": "/sensor" }), 400)]
#[case::no_user_agent(json!({ "userAgent": null }), 400)]
#[case::version(json!({ "version": "1" }), 400)]
#[case::script_and_hash(json!({ "scriptHash": "0".repeat(64) }), 400)]
#[case::hash_and_context(json!({ "script": null, "scriptHash": "0".repeat(64), "context": "c" }), 400)]
#[case::hash_not_hex(json!({ "script": null, "scriptHash": "X".repeat(64) }), 400)]
#[case::made_up_context(json!({ "script": null, "context": "c" }), 400)]
#[case::no_script(json!({ "script": null }), 422)]
#[case::unknown_hash(json!({ "script": null, "scriptHash": "0".repeat(64) }), 422)]
fn a_call_that_misses_what_the_sensor_needs_is_refused_with_no_price(
    #[case] fields: Value,
    #[case] status: u16,
) {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));

    let answer = client.sensor(&fields);
    let recorded = client.recorded();

    let body = answer.json();
    assert_eq!(answer.status, status, "{body}");
    if status == 422 {
        assert_eq!(body, json!({ "error": "script_required" }));
    } else {
        assert!(body["error"].is_string(), "{body}");
        assert_eq!(body.as_object().map(serde_json::Map::len), Some(1));
    }
    assert_eq!(recorded[0]["price"], Value::Null);
}

#[test]
fn a_service_told_to_answer_malformed_json_does_so_and_charges() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("malformed"));

    let answer = client.sensor(&json!({}));

    assert_eq!(answer.status, 200);
    let read: Result<Value, _> = serde_json::from_str(&answer.body);
    assert!(read.is_err(), "{}", answer.body);
    assert_eq!(client.recorded()[0]["price"], PRICE);
}

#[test]
fn a_service_told_to_stall_never_answers() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("stall"));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_recv_response(Some(Duration::from_secs(1)))
        .build()
        .into();

    let answer = agent
        .post(format!("http://{}{ENDPOINT}", proving_ground.address()))
        .header("host", HOST)
        .header("x-api-key", &client.key)
        .send_json(client.body());
    let recorded = client.recorded();

    assert!(matches!(answer, Err(ureq::Error::Timeout(_))), "{answer:?}");
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(
        (&recorded[0]["status"], &recorded[0]["price"]),
        (&Value::Null, &Value::Null)
    );
}

#[test]
fn every_call_is_recorded_with_the_price_on_the_one_that_spends_it() {
    let proving_ground = ProvingGround::start(SEED);
    let client = Client::new(&proving_ground, &json!("solve"));

    client.sensor(&json!({}));
    client.sensor(&json!({ "script": null }));
    let recorded = client.recorded();

    let shown: Vec<Value> = recorded
        .iter()
        .map(|call| {
            json!([
                call["service"],
                call["call"],
                call["kind"],
                call["task"],
                call["status"],
                call["error"],
                call["price"]
            ])
        })
        .collect();
    let refused = json!({ "error": "script_required" });
    assert_eq!(
        shown,
        [
            json!([
                "hyper_solutions",
                "generate",
                "v2/sensor",
                null,
                200,
                null,
                PRICE
            ]),
            json!([
                "hyper_solutions",
                "generate",
                "v2/sensor",
                null,
                422,
                refused,
                null
            ]),
        ]
    );
    let document = proving_ground.control("/solver-calls").body;
    assert!(!document.contains(&client.key), "{document}");
}

#[test]
fn a_result_error_is_refused_for_a_service_that_has_one_call() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.post_control(
        "/solver-accounts",
        &json!({
            "service": "hyper_solutions",
            "told": { "result_error": "access_denied" },
        }),
    );

    assert_eq!(
        (answer.status, answer.json()["error"].clone()),
        (400, json!("malformed_solver_account"))
    );
}
