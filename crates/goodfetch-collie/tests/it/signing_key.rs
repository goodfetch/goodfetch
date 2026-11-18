//! `signing-key create` and `signing-key directory` through Collie's
//! executable (ADRs 0045 and 0056; #1551): the key `create` makes in an
//! encrypted local file signs an `identified` Session's requests, and the
//! directory `directory` exports, served as it is from a plain HTTP origin
//! the Session's `Signature-Agent` names, is the one the Proving ground
//! fetches as each request arrives: it checks each key's ID is its
//! thumbprint and each key's proof of possession, then the request's
//! signature against it. A second `create` keeps the key it finds, and
//! both operations refuse a reference that cannot hold or give a key,
//! never quoting what the item holds.
#![expect(
    clippy::disallowed_methods,
    reason = "the agent's origin stand-in serves the exported directory on a thread of its own"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the agent's origin stand-in listens outside the engine, as the user's own server would (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, hello};
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, cli};
use serde_json::{Map, Value, json};

use crate::page::{opened, started_with};
use crate::secrets::{encrypt, identity};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's own seed for these tests.
const SEED: u64 = 1551;

/// What `signing-key <words…> --json` printed, as JSON.
fn printed(env: &TestEnv, words: &[&str]) -> Value {
    let output = cli(env, COLLIE)
        .args(["signing-key"])
        .args(words)
        .arg("--json")
        .run()
        .unwrap();
    serde_json::from_str(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
}

/// Serves `directory`, as `signing-key directory` exported it, at every
/// path of the plain HTTP origin `listener` listens on.
fn serve(listener: TcpListener, directory: &Value) {
    let body = directory["body"].as_str().unwrap().to_owned();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nSignature-Input: {}\r\nSignature: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        directory["content_type"].as_str().unwrap(),
        directory["signature_input"].as_str().unwrap(),
        directory["signature"].as_str().unwrap(),
        body.len(),
    );
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let mut stream = reader.into_inner();
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
}

/// Each request `identity.test` received, as its target, whether its
/// signature verified, the key that signed it and why it did not verify.
fn checked(proving_ground: &ProvingGround) -> Vec<(String, Value, Value, Value)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == "identity.test")
        .map(|request| {
            let verification = request
                .message_signature
                .as_ref()
                .map_or(&Value::Null, |signature| &signature["verification"]);
            (
                request.target.clone(),
                verification["verified"].clone(),
                verification["key_id"].clone(),
                verification["error"].clone(),
            )
        })
        .collect()
}

/// Covers: story:8-3, command:signing-key.create, command:signing-key.directory
/// Covers: operation:signing_key.create, operation:signing_key.directory
/// Covers: setting:identified.signing_key, setting:identified.signature_agent
#[test]
fn a_session_signing_with_a_created_key_verifies_against_its_exported_directory() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let retiring = printed(
        &env,
        &["create", "--signing-key", "secretfile:retiring-key"],
    );
    let current = printed(&env, &["create", "--signing-key", "secretfile:current-key"]);
    let again = printed(&env, &["create", "--signing-key", "secretfile:current-key"]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let authority = listener.local_addr().unwrap().to_string();
    let directory = printed(
        &env,
        &[
            "directory",
            "--signing-keys",
            "secretfile:retiring-key",
            "--signing-keys",
            "secretfile:current-key",
            "--authority",
            &authority,
        ],
    );
    serve(listener, &directory);
    let settings = json!({
        "browser": { "access_mode": "identified" },
        "identified": {
            "signing_key": "secretfile:current-key",
            "signature_agent": format!("http://{authority}"),
        },
    });
    let Value::Object(settings) = settings else {
        unreachable!("settings are an object")
    };
    let mut engine = started_with(&env, &cache, &proving_ground, "exported", &settings, |_| {});

    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "exported" })));
    let status = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("identity.test", "/catalog/1") }),
    )["result"]["response"]["status"]
        .clone();
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "exported" }));
    let _ = engine.finish();

    let key_id = current["key_id"].clone();
    assert_eq!(
        (
            (&current["created"], &again["created"], &again["key_id"]),
            (&current["public_key"]["kid"], &current["public_key"]["kty"]),
            current.as_object().map(Map::len),
            (&directory["path"], &directory["key_ids"]),
            status,
            checked(&proving_ground),
        ),
        (
            (&json!(true), &json!(false), &key_id),
            (&key_id, &json!("OKP")),
            Some(4),
            (
                &json!("/.well-known/http-message-signatures-directory"),
                &json!([retiring["key_id"], key_id]),
            ),
            json!(200),
            vec![(
                "/catalog/1".to_owned(),
                json!(true),
                key_id.clone(),
                Value::Null
            )],
        ),
        "{directory}"
    );
}

/// What `method` answered, called with `params` on an Attached engine.
fn answered(env: &TestEnv, method: &str, params: &Value) -> Value {
    let mut engine = Attached::start(env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let answer = engine.ask(1, method, params);
    let _ = engine.finish();
    answer
}

/// Covers: operation:signing_key.create, operation:signing_key.directory
#[test]
fn a_reference_that_cannot_hold_or_give_a_key_is_refused_without_quoting_its_item() {
    let env = TestEnv::new();
    let held = "not-a-signing-key-but-a-password";
    encrypt(&env, "password", held, &identity(&env));

    let refusals = [
        answered(
            &env,
            "signing_key.create",
            &json!({ "signing_key": "env:BOT_KEY" }),
        ),
        answered(
            &env,
            "signing_key.create",
            &json!({ "signing_key": "secretfile:password" }),
        ),
        answered(
            &env,
            "signing_key.directory",
            &json!({ "signing_keys": ["secretfile:password"], "authority": "bot.example" }),
        ),
        answered(
            &env,
            "signing_key.directory",
            &json!({ "signing_keys": [], "authority": "bot.example" }),
        ),
    ];

    let codes: Vec<&Value> = refusals
        .iter()
        .map(|refusal| &refusal["error"]["code"])
        .collect();
    assert_eq!(
        (
            codes,
            refusals
                .iter()
                .any(|refusal| refusal.to_string().contains(held)),
        ),
        (vec![&json!(-32602); 4], false),
        "{refusals:?}"
    );
}
