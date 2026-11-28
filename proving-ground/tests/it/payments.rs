//! Black-box tests for pay-per-crawl and x402 pages settled on the Proving
//! ground's local chain (#1537, ADRs 0045 and 0076): each test drives `serve`
//! with a plain HTTP client, never an engine crate.
//!
//! Covers: story:2-33
#![expect(
    clippy::disallowed_types,
    reason = "the test serves an agent's Web Bot Auth key directory on its own loopback socket"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test serves a key directory on a thread of its own and signs Web Bot Auth requests with wall-clock expiry"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD, HEXLOWER};
use ed25519_dalek::SigningKey;
use goodfetch_test_support::ROUTES;
use hmac::{Hmac, Mac as _};
use indexmap::IndexMap;
use serde_json::{Value, json};
use sha2::Sha256;
use web_bot_auth::components::{
    CoveredComponent, DerivedComponent, HTTPField, HTTPFieldParametersSet,
};
use web_bot_auth::keyring::{Algorithm, Thumbprintable};
use web_bot_auth::message_signatures::{MessageSigner, UnsignedMessage};

use crate::proving_ground::ProvingGround;

const SEED: u64 = 1537;

/// Deterministic Ed25519 test key for Web Bot Auth (RFC 9421 Appendix B.1.4).
const AGENT_KEY: [u8; 32] = [
    0x9f, 0x83, 0x62, 0xf8, 0x7a, 0x48, 0x4a, 0x95, 0x4e, 0x6e, 0x74, 0x0c, 0x5b, 0x4c, 0x0e, 0x84,
    0x22, 0x91, 0x39, 0xa2, 0x0a, 0xa8, 0xab, 0x56, 0xff, 0x66, 0x58, 0x6f, 0x6a, 0x7d, 0x29, 0xc5,
];

fn jwk(private: &[u8; 32]) -> (Thumbprintable, String) {
    let public = SigningKey::from_bytes(private).verifying_key().to_bytes();
    let key = Thumbprintable::OKP {
        crv: "Ed25519".to_owned(),
        x: BASE64URL_NOPAD.encode(&public),
    };
    let thumbprint = key.b64_thumbprint();
    (key, thumbprint)
}

struct Covered {
    components: IndexMap<CoveredComponent, String>,
    label: &'static str,
    input: String,
    signature: String,
}

impl UnsignedMessage for Covered {
    fn fetch_components_to_cover(&self) -> IndexMap<CoveredComponent, String> {
        self.components.clone()
    }

    fn register_header_contents(&mut self, signature_input: String, signature_header: String) {
        self.input = format!("{}={signature_input}", self.label);
        self.signature = format!("{}={signature_header}", self.label);
    }
}

fn sign_wba(
    key: &[u8; 32],
    tag: &str,
    label: &'static str,
    nonce_byte: u8,
    components: IndexMap<CoveredComponent, String>,
) -> (String, String) {
    let mut covered = Covered {
        components,
        label,
        input: String::new(),
        signature: String::new(),
    };
    MessageSigner {
        keyid: jwk(key).1,
        nonce: BASE64URL_NOPAD.encode(&[nonce_byte; 32]),
        tag: tag.to_owned(),
    }
    .generate_signature_headers_content(
        &mut covered,
        Duration::from_secs(60),
        Algorithm::Ed25519,
        key,
    )
    .expect("the Web Bot Auth message signs");
    (covered.input, covered.signature)
}

fn authority_component(value: &str, request: bool) -> (CoveredComponent, String) {
    (
        CoveredComponent::Derived(DerivedComponent::Authority { req: request }),
        value.to_owned(),
    )
}

fn field_component(name: &str, value: &str) -> (CoveredComponent, String) {
    (
        CoveredComponent::HTTP(HTTPField {
            name: name.to_owned(),
            parameters: HTTPFieldParametersSet(vec![]),
        }),
        value.to_owned(),
    )
}

/// Serves a loopback Web Bot Auth key directory for `AGENT_KEY` and returns its origin.
fn start_key_directory() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind key directory");
    let origin = format!("http://{}", listener.local_addr().expect("dir addr"));
    let host = origin.trim_start_matches("http://").to_owned();
    let (key_jwk, thumbprint) = jwk(&AGENT_KEY);
    let mut key_value = serde_json::to_value(key_jwk).expect("jwk json");
    key_value["kid"] = json!(thumbprint);
    let body = json!({ "keys": [key_value] }).to_string();
    let (input, signature) = sign_wba(
        &AGENT_KEY,
        "http-message-signatures-directory",
        "sig1",
        1,
        IndexMap::from([authority_component(&host, true)]),
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/http-message-signatures-directory+json\r\n\
         Signature-Input: {input}\r\nSignature: {signature}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    origin
}

struct Accounts {
    chain: String,
    network: String,
    sender: String,
    sender_key: String,
    recipient: String,
    token: String,
}

fn read_accounts(ground: &ProvingGround) -> Accounts {
    let first = ground.line_json();
    let accounts = &first["chain_accounts"];
    Accounts {
        chain: first["chain"].as_str().expect("chain url").to_owned(),
        network: accounts["network"].as_str().expect("network").to_owned(),
        sender: accounts["payer_address"]
            .as_str()
            .expect("payer")
            .to_owned(),
        sender_key: accounts["payer_key"]
            .as_str()
            .expect("payer key")
            .to_owned(),
        recipient: accounts["payee_address"]
            .as_str()
            .expect("payee")
            .to_owned(),
        token: accounts["token"].as_str().expect("token").to_owned(),
    }
}

fn sign_x402_payload(
    accounts: &Accounts,
    scheme: &str,
    value: u128,
    valid_before: u64,
    nonce: &str,
) -> String {
    let key_bytes = HEXLOWER
        .decode(accounts.sender_key.trim_start_matches("0x").as_bytes())
        .expect("hex payer key");
    let mut mac = Hmac::<Sha256>::new_from_slice(&key_bytes).expect("hmac key");
    let message = format!(
        "{scheme}|{}|{}|{}|{value}|{valid_before}|{nonce}",
        accounts.network,
        accounts.sender.to_ascii_lowercase(),
        accounts.recipient.to_ascii_lowercase()
    );
    mac.update(message.as_bytes());
    let sig = format!("0x{}", HEXLOWER.encode(&mac.finalize().into_bytes()));
    let payload = json!({
        "x402Version": 2,
        "scheme": scheme,
        "network": accounts.network,
        "payload": {
            "from": accounts.sender,
            "to": accounts.recipient,
            "value": value.to_string(),
            "validBefore": valid_before,
            "nonce": nonce,
            "signature": sig,
        }
    });
    BASE64.encode(&serde_json::to_vec(&payload).expect("serialize payload"))
}

fn rpc_call(url: &str, method: &str, params: &Value) -> Value {
    let resp = ureq::post(url)
        .send_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .expect("rpc call succeeds");
    resp.into_body()
        .read_json::<Value>()
        .expect("rpc json response")
}

fn nonce_hex(n: u64) -> String {
    format!("0x{n:064x}")
}

fn auth_state_calldata(sender: &str, nonce: &str) -> String {
    let sender_hex = sender.trim_start_matches("0x").to_ascii_lowercase();
    let nonce_clean = nonce.trim_start_matches("0x").to_ascii_lowercase();
    format!("0xe94a0102{sender_hex:0>64}{nonce_clean:0>64}")
}

fn oracle_judge(ground: &ProvingGround, body: &Value) -> Value {
    let answer = ground.post_control(ROUTES.oracle, body);
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

#[test]
fn serve_reports_local_chain_and_facilitator_with_no_external_network() {
    let ground = ProvingGround::start(SEED);
    let first = ground.line_json();
    let chain = first["chain"]
        .as_str()
        .expect("chain address in first line");
    let facilitator = first["facilitator"]
        .as_str()
        .expect("facilitator address in first line");
    assert!(chain.starts_with("http://127.0.0.1:"), "chain={chain}");
    assert!(
        facilitator.starts_with("http://127.0.0.1:"),
        "facilitator={facilitator}"
    );
    let accounts = &first["chain_accounts"];
    assert_eq!(accounts["chain_id"], 31_337);
    assert_eq!(accounts["network"], "eip155:31337");
    assert!(
        accounts["payer_address"]
            .as_str()
            .unwrap()
            .starts_with("0x")
    );
    assert!(
        accounts["payee_address"]
            .as_str()
            .unwrap()
            .starts_with("0x")
    );

    let chain_id = rpc_call(chain, "eth_chainId", &json!([]));
    assert_eq!(chain_id["result"], "0x7a69");
    let balance = rpc_call(chain, "eth_getBalance", &json!([accounts["payer_address"]]));
    assert_ne!(balance["result"], "0x0");

    let supported: Value = ureq::get(&format!("{facilitator}/supported"))
        .call()
        .expect("GET /supported")
        .into_body()
        .read_json()
        .expect("supported json");
    let schemes: Vec<&str> = supported["kinds"]
        .as_array()
        .expect("kinds array")
        .iter()
        .filter_map(|k| k["scheme"].as_str())
        .collect();
    assert!(schemes.contains(&"exact"));
    assert!(schemes.contains(&"upto"));
    assert!(schemes.contains(&"batch-settlement"));
}

#[test]
fn pay_per_crawl_enforces_price_signature_and_replay_and_oracle_counts() {
    let ground = ProvingGround::start(SEED);
    let dir_origin = start_key_directory();
    let host = "1.paid.test";
    let authority = format!("{host}:{}", ground.port());
    let url = ground.url(host, "/page");

    let unpaid = ground.get(&url);
    assert_eq!(unpaid.status, 402);
    assert_eq!(unpaid.header("crawler-price"), Some("USD 0.01"));

    let unsigned = ground.get_with_headers(&url, &[("crawler-exact-price", "USD 0.01")]);
    assert_eq!(unsigned.status, 402);
    assert_eq!(unsigned.header("x-payment-error"), Some("unsigned"));

    let sig_agent = format!("\"{dir_origin}\"");
    let (wrong_in, wrong_sig) = sign_wba(
        &AGENT_KEY,
        "web-bot-auth",
        "sig1",
        10,
        IndexMap::from([
            authority_component(&authority, false),
            field_component("signature-agent", &sig_agent),
            field_component("crawler-exact-price", "USD 0.005"),
        ]),
    );
    let wrong = ground.get_with_headers(
        &url,
        &[
            ("crawler-exact-price", "USD 0.005"),
            ("signature-agent", &sig_agent),
            ("signature-input", &wrong_in),
            ("signature", &wrong_sig),
        ],
    );
    assert_eq!(wrong.status, 402);
    assert_eq!(wrong.header("x-payment-error"), Some("wrong_amount"));

    let (ok_in, ok_sig) = sign_wba(
        &AGENT_KEY,
        "web-bot-auth",
        "sig1",
        11,
        IndexMap::from([
            authority_component(&authority, false),
            field_component("signature-agent", &sig_agent),
            field_component("crawler-exact-price", "USD 0.01"),
        ]),
    );
    let ok_headers = [
        ("crawler-exact-price", "USD 0.01"),
        ("signature-agent", sig_agent.as_str()),
        ("signature-input", ok_in.as_str()),
        ("signature", ok_sig.as_str()),
    ];
    let paid = ground.get_with_headers(&url, &ok_headers);
    assert_eq!(paid.status, 200);
    assert_eq!(paid.header("crawler-charged"), Some("USD 0.01"));
    assert!(paid.body.contains("Paid content delivered."));

    let replay = ground.get_with_headers(&url, &ok_headers);
    assert_eq!(replay.status, 402);
    assert_eq!(replay.header("x-payment-error"), Some("replay"));

    let verdict = oracle_judge(
        &ground,
        &json!({ "task": "paid/1", "status": "success", "records": [] }),
    );
    assert_eq!(verdict["payments"]["paid_count"], 1);
    assert_eq!(verdict["payments"]["total_paid_atomic"], 10_000);
    assert_eq!(verdict["payments"]["fetched_count"], 1);
    assert_eq!(verdict["payments"]["refused_count"], 4);
}

#[test]
fn x402_exact_and_upto_settle_on_local_chain() {
    let ground = ProvingGround::start(SEED);
    let accounts = read_accounts(&ground);
    let valid_before: u64 = 1_800_000_000;

    let url2 = ground.url("2.paid.test", "/page");
    let unpaid = ground.get(&url2);
    assert_eq!(unpaid.status, 402);
    assert!(unpaid.header("payment-required").is_some());
    assert_eq!(unpaid.json()["accepts"][0]["scheme"], "exact");

    let wrong_exact = sign_x402_payload(&accounts, "exact", 5_000, valid_before, &nonce_hex(20));
    let resp = ground.get_with_headers(&url2, &[("payment-signature", &wrong_exact)]);
    assert_eq!(resp.status, 402);
    assert_eq!(resp.header("x-payment-error"), Some("wrong_amount"));

    let unsigned_payload = BASE64.encode(
        serde_json::to_vec(&json!({
            "x402Version": 2,
            "scheme": "exact",
            "network": accounts.network,
            "payload": {
                "from": accounts.sender,
                "to": accounts.recipient,
                "value": "10000",
                "validBefore": valid_before,
                "nonce": nonce_hex(21),
                "signature": "",
            }
        }))
        .expect("serialize unsigned payload")
        .as_slice(),
    );
    let resp = ground.get_with_headers(&url2, &[("payment-signature", &unsigned_payload)]);
    assert_eq!(resp.status, 402);
    assert_eq!(resp.header("x-payment-error"), Some("unsigned"));

    let exact_nonce = nonce_hex(22);
    let ok_exact = sign_x402_payload(&accounts, "exact", 10_000, valid_before, &exact_nonce);
    let resp = ground.get_with_headers(&url2, &[("payment-signature", &ok_exact)]);
    assert_eq!(resp.status, 200);
    assert!(resp.header("payment-response").is_some());

    let replay = ground.get_with_headers(&url2, &[("payment-signature", &ok_exact)]);
    assert_eq!(replay.status, 402);
    assert_eq!(replay.header("x-payment-error"), Some("replay"));

    let auth_state = rpc_call(
        &accounts.chain,
        "eth_call",
        &json!([{"to": accounts.token, "data": auth_state_calldata(&accounts.sender, &exact_nonce)}]),
    );
    assert_eq!(
        auth_state["result"],
        "0x0000000000000000000000000000000000000000000000000000000000000001"
    );

    let url3 = ground.url("3.paid.test", "/page");
    let ok_upto = sign_x402_payload(&accounts, "upto", 20_000, valid_before, &nonce_hex(30));
    let resp = ground.get_with_headers(&url3, &[("payment-signature", &ok_upto)]);
    assert_eq!(resp.status, 200);
    let upto_verdict = oracle_judge(
        &ground,
        &json!({ "task": "paid/3", "status": "success", "records": [] }),
    );
    assert_eq!(upto_verdict["payments"]["paid_count"], 1);
    assert_eq!(upto_verdict["payments"]["total_paid_atomic"], 10_000);
    assert_eq!(upto_verdict["payments"]["fetched_count"], 1);
}

#[test]
fn x402_batch_settlement_requires_web_bot_auth() {
    let ground = ProvingGround::start(SEED);
    let dir_origin = start_key_directory();
    let accounts = read_accounts(&ground);
    let url4 = ground.url("4.paid.test", "/page");
    let authority4 = format!("4.paid.test:{}", ground.port());
    let batch_payload = sign_x402_payload(
        &accounts,
        "batch-settlement",
        10_000,
        1_800_000_000,
        &nonce_hex(40),
    );

    let unsigned_batch = ground.get_with_headers(&url4, &[("payment-signature", &batch_payload)]);
    assert_eq!(unsigned_batch.status, 402);
    assert_eq!(unsigned_batch.header("x-payment-error"), Some("unsigned"));

    let sig_agent = format!("\"{dir_origin}\"");
    let (batch_in, batch_sig) = sign_wba(
        &AGENT_KEY,
        "web-bot-auth",
        "sig1",
        40,
        IndexMap::from([
            authority_component(&authority4, false),
            field_component("signature-agent", &sig_agent),
            field_component("payment-signature", &batch_payload),
        ]),
    );
    let ok_batch = ground.get_with_headers(
        &url4,
        &[
            ("payment-signature", &batch_payload),
            ("signature-agent", &sig_agent),
            ("signature-input", &batch_in),
            ("signature", &batch_sig),
        ],
    );
    assert_eq!(ok_batch.status, 200);
    assert!(ok_batch.header("payment-response").is_some());
}

#[test]
fn refusal_and_interrupted_payments_are_reported_by_chain_and_oracle() {
    let ground = ProvingGround::start(SEED);
    let accounts = read_accounts(&ground);
    let valid_before: u64 = 1_767_226_000;

    let refused = ground.get(&ground.url("5.paid.test", "/page"));
    assert_eq!(refused.status, 402);
    assert_eq!(refused.header("crawler-price"), Some("USD 0.50"));
    let refusal_verdict = oracle_judge(
        &ground,
        &json!({
            "task": "paid/5",
            "status": "failure",
            "error": { "code": "payment_required" },
        }),
    );
    assert_eq!(refusal_verdict["outcome"], "verified");
    assert_eq!(refusal_verdict["payments"]["paid_count"], 0);

    let n6 = nonce_hex(60);
    let p6 = sign_x402_payload(&accounts, "exact", 10_000, valid_before, &n6);
    let r6 = ground.get_with_headers(
        &ground.url("6.paid.test", "/page"),
        &[("payment-signature", &p6)],
    );
    assert_eq!(r6.status, 503);
    assert_eq!(r6.header("x-payment-interrupted"), Some("settled"));
    let state6 = rpc_call(
        &accounts.chain,
        "eth_call",
        &json!([{"to": accounts.token, "data": auth_state_calldata(&accounts.sender, &n6)}]),
    );
    assert_eq!(
        state6["result"],
        "0x0000000000000000000000000000000000000000000000000000000000000001"
    );
    let v6 = oracle_judge(&ground, &json!({"task": "paid/6", "status": "failure"}));
    assert_eq!(v6["payments"]["interrupted"], "settled");
    assert_eq!(v6["payments"]["paid_count"], 1);
    assert_eq!(v6["payments"]["fetched_count"], 0);

    let n7 = nonce_hex(70);
    let p7 = sign_x402_payload(&accounts, "exact", 10_000, valid_before, &n7);
    let r7 = ground.get_with_headers(
        &ground.url("7.paid.test", "/page"),
        &[("payment-signature", &p7)],
    );
    assert_eq!(r7.status, 503);
    assert_eq!(
        r7.header("x-payment-interrupted"),
        Some("still_able_to_settle")
    );
    let state7 = rpc_call(
        &accounts.chain,
        "eth_call",
        &json!([{"to": accounts.token, "data": auth_state_calldata(&accounts.sender, &n7)}]),
    );
    assert_eq!(
        state7["result"],
        "0x0000000000000000000000000000000000000000000000000000000000000000"
    );
    let v7 = oracle_judge(&ground, &json!({"task": "paid/7", "status": "failure"}));
    assert_eq!(v7["payments"]["interrupted"], "still_able_to_settle");
    assert_eq!(v7["payments"]["paid_count"], 0);

    let n8 = nonce_hex(80);
    let p8 = sign_x402_payload(&accounts, "exact", 10_000, valid_before, &n8);
    let r8 = ground.get_with_headers(
        &ground.url("8.paid.test", "/page"),
        &[("payment-signature", &p8)],
    );
    assert_eq!(r8.status, 503);
    assert_eq!(r8.header("x-payment-interrupted"), Some("unable_to_settle"));
    let block = rpc_call(
        &accounts.chain,
        "eth_getBlockByNumber",
        &json!(["finalized", false]),
    );
    let ts_hex = block["result"]["timestamp"]
        .as_str()
        .expect("block timestamp");
    let ts = u64::from_str_radix(ts_hex.trim_start_matches("0x"), 16).expect("hex timestamp");
    assert!(
        ts > valid_before,
        "chain timestamp {ts} must exceed validBefore {valid_before}"
    );
    let v8 = oracle_judge(&ground, &json!({"task": "paid/8", "status": "failure"}));
    assert_eq!(v8["payments"]["interrupted"], "unable_to_settle");
    assert_eq!(v8["payments"]["paid_count"], 0);
}
