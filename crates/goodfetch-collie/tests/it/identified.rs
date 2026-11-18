//! An `identified` Session over stdio (ADRs 0021 and 0045; #1551): each
//! request its browser sends, at the interception, and each `page.request`
//! carries a Web Bot Auth signature that the Proving ground verifies as the
//! request arrives, against the inline key directory its `Signature-Agent`
//! names. The key is a Secret reference to an encrypted local file, read
//! when the Session opens: a key rotated in the file signs the next
//! Session of a Run that resolves it at each use, and a Session reopened
//! on a resume. A Secret canary stands for the key's private bytes and is
//! searched in every reply and every file the test's folder holds.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use data_encoding::{BASE64, BASE64URL_NOPAD};
use ed25519_dalek::SigningKey;
use goodfetch_protocol::page::SessionId;
use goodfetch_test_client::Attached;
use goodfetch_test_support::{
    Canaries, Output, ProvingGround, SecretKind, TestEnv, browser_cache, chrome_for_testing_files,
};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::canaries::paths_but;
use crate::page::{opened, resumed_with, started_with};
use crate::secrets::{encrypt, identity};
use crate::sessions::{page_in, session_of};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 1551;

/// The encrypted local file the Session's key is read from.
const KEY_FILE: &str = "signing-key";

/// The Secret reference of [`KEY_FILE`].
const KEY_REFERENCE: &str = "secretfile:signing-key";

/// An Ed25519 key whose 32 private bytes are a planted canary's.
struct Key {
    private: [u8; 32],
}

impl Key {
    /// A key whose private bytes are a fresh 32-byte key canary of
    /// `canaries`.
    fn planted(canaries: &mut Canaries) -> Self {
        let canary = canaries.plant(SecretKind::RunDataKey);
        Self {
            private: canary
                .value()
                .as_bytes()
                .try_into()
                .expect("a key canary is 32 bytes"),
        }
    }

    /// The public key, in unpadded Base64url.
    fn x(&self) -> String {
        BASE64URL_NOPAD.encode(
            SigningKey::from_bytes(&self.private)
                .verifying_key()
                .as_bytes(),
        )
    }

    /// The private JSON Web Key the Secret holds.
    fn jwk(&self) -> String {
        json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "d": BASE64URL_NOPAD.encode(&self.private),
            "x": self.x(),
        })
        .to_string()
    }

    /// The key's ID, its RFC 7638 thumbprint.
    fn id(&self) -> String {
        let members = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, self.x());
        BASE64URL_NOPAD.encode(&Sha256::digest(members.as_bytes()))
    }
}

/// An inline key directory publishing `keys`, each under its thumbprint.
fn directory(keys: &[&Key]) -> String {
    let keys: Vec<Value> = keys
        .iter()
        .map(|key| json!({ "kty": "OKP", "crv": "Ed25519", "x": key.x(), "kid": key.id() }))
        .collect();
    format!(
        "data:application/http-message-signatures-directory;base64,{}",
        BASE64.encode(json!({ "keys": keys }).to_string().as_bytes())
    )
}

/// The settings of an `identified` Session signing with the key at
/// [`KEY_REFERENCE`], its directory naming `keys`.
fn identified(keys: &[&Key]) -> Value {
    json!({
        "browser": { "access_mode": "identified" },
        "identified": {
            "signing_key": KEY_REFERENCE,
            "signature_agent": directory(keys),
        },
    })
}

/// The settings `settings` by area, as a Run starts with them.
fn by_area(settings: Value) -> Map<String, Value> {
    match settings {
        Value::Object(areas) => areas,
        other => panic!("settings by area, found {other}"),
    }
}

/// Takes a new page of the Session `session` to the shelf's page `page` on
/// `identity.test`, and answers its status.
fn shelf_page(
    engine: &mut Attached,
    id: i64,
    proving_ground: &ProvingGround,
    session: &SessionId,
    page: u32,
) -> Value {
    let opened = page_in(engine, id, session);
    engine.ask(
        id + 1,
        "page.goto",
        &json!({
            "page_id": opened.page_id,
            "url": proving_ground.url("identity.test", &format!("/catalog/{page}")),
        }),
    )["result"]["response"]["status"]
        .clone()
}

/// Each request `identity.test` received, as its target, whether its
/// signature verified and the key that signed it.
fn checked(proving_ground: &ProvingGround) -> Vec<(String, Value, Value)> {
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
            )
        })
        .collect()
}

/// The checks of the requests for `target`, each as whether it verified
/// and its key.
fn checks_of(checked: &[(String, Value, Value)], target: &str) -> Vec<(Value, Value)> {
    checked
        .iter()
        .filter(|(seen, _, _)| seen == target)
        .map(|(_, verified, key)| (verified.clone(), key.clone()))
        .collect()
}

/// The leaks of `canaries` in `replies` and every file of `env`'s folder,
/// but the Chrome for Testing build in the Browser cache at `cache`, which
/// no engine writes.
fn leaks(canaries: &Canaries, env: &TestEnv, cache: &Path, replies: &[Value]) -> Vec<String> {
    let replies = serde_json::to_vec(replies).unwrap();
    let searched = paths_but(env.folder(), &chrome_for_testing_files(cache));
    let mut outputs = vec![Output::Bytes {
        name: "replies",
        bytes: &replies,
    }];
    outputs.extend(searched.iter().map(|path| Output::Path(path)));
    canaries
        .leaks(&outputs)
        .unwrap()
        .into_iter()
        .map(|hit| format!("{} in {}", hit.canary, hit.output))
        .collect()
}

/// Covers: story:8-3, operation:run.new_page, operation:page.goto, operation:page.request
/// Covers: setting:identified.signing_key, setting:identified.signature_agent
#[test]
fn every_request_of_an_identified_session_is_signed_and_verified_and_no_output_holds_its_key() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let key = Key::planted(&mut canaries);
    encrypt(&env, KEY_FILE, &key.jwk(), &identity(&env));
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "signing",
        &by_area(identified(&[&key])),
        |_| {},
    );

    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "signing" })));
    let mut replies = vec![
        engine.ask(
            3,
            "page.goto",
            &json!({ "page_id": page.page_id, "url": proving_ground.url("identity.test", "/catalog/1") }),
        ),
        engine.ask(
            4,
            "page.request",
            &json!({ "page_id": page.page_id, "url": proving_ground.url("identity.test", "/catalog/2") }),
        ),
        engine.ask(5, "run.finish", &json!({ "run_id": "signing" })),
    ];
    let (rest, _) = engine.finish();
    replies.extend(rest);
    let checked = checked(&proving_ground);

    let signed = (json!(true), json!(key.id()));
    assert_eq!(
        (
            (
                &replies[0]["result"]["response"]["status"],
                &replies[1]["result"]["status"],
            ),
            checks_of(&checked, "/catalog/1"),
            checks_of(&checked, "/catalog/2"),
            checked
                .iter()
                .all(|(_, verified, _)| verified == &json!(true)),
            leaks(&canaries, &env, &cache, &replies),
        ),
        (
            (&json!(200), &json!(200)),
            vec![signed.clone()],
            vec![signed],
            true,
            Vec::<String>::new(),
        ),
        "{checked:?}"
    );
}

/// Covers: story:8-3, operation:run.new_session, setting:identified.signing_key
/// Covers: setting:secrets.resolve_each_use
#[test]
fn a_key_rotated_in_its_secret_signs_the_runs_next_session() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let (first, second) = (Key::planted(&mut canaries), Key::planted(&mut canaries));
    let files = identity(&env);
    encrypt(&env, KEY_FILE, &first.jwk(), &files);
    let mut settings = by_area(identified(&[&first, &second]));
    settings.insert(
        "secrets".to_owned(),
        json!({ "resolve_each_use": [KEY_REFERENCE] }),
    );
    let mut engine = started_with(&env, &cache, &proving_ground, "rotating", &settings, |_| {});

    let before = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "rotating" })));
    let before_status = shelf_page(&mut engine, 3, &proving_ground, &before, 1);
    encrypt(&env, KEY_FILE, &second.jwk(), &files);
    let after = session_of(&engine.ask(5, "run.new_session", &json!({ "run_id": "rotating" })));
    let after_status = shelf_page(&mut engine, 6, &proving_ground, &after, 2);
    let again_status = shelf_page(&mut engine, 8, &proving_ground, &before, 3);
    let _ = engine.ask(10, "run.finish", &json!({ "run_id": "rotating" }));
    let _ = engine.finish();
    let checked = checked(&proving_ground);

    assert_eq!(
        (
            (before_status, after_status, again_status),
            checks_of(&checked, "/catalog/1"),
            checks_of(&checked, "/catalog/2"),
            checks_of(&checked, "/catalog/3"),
        ),
        (
            (json!(200), json!(200), json!(200)),
            vec![(json!(true), json!(first.id()))],
            vec![(json!(true), json!(second.id()))],
            vec![(json!(true), json!(first.id()))],
        ),
        "{checked:?}"
    );
}

/// Covers: story:8-3, operation:run.resume, setting:identified.signing_key
#[test]
fn a_session_reopened_on_a_resume_signs_with_its_key_as_it_reads_it_then() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let (first, second) = (Key::planted(&mut canaries), Key::planted(&mut canaries));
    let files = identity(&env);
    encrypt(&env, KEY_FILE, &first.jwk(), &files);
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "reopened",
        &Map::new(),
        |_| {},
    );
    let session = session_of(&engine.ask(
        2,
        "run.new_session",
        &json!({ "run_id": "reopened", "settings": identified(&[&first, &second]) }),
    ));
    let before_status = shelf_page(&mut engine, 3, &proving_ground, &session, 1);
    engine.kill();
    encrypt(&env, KEY_FILE, &second.jwk(), &files);

    let mut resumed = resumed_with(
        &env,
        &cache,
        &proving_ground,
        "reopened",
        &Map::new(),
        |_| {},
    );
    let after_status = shelf_page(&mut resumed, 2, &proving_ground, &session, 2);
    let _ = resumed.ask(4, "run.finish", &json!({ "run_id": "reopened" }));
    let _ = resumed.finish();
    let checked = checked(&proving_ground);

    assert_eq!(
        (
            (before_status, after_status),
            checks_of(&checked, "/catalog/1"),
            checks_of(&checked, "/catalog/2"),
        ),
        (
            (json!(200), json!(200)),
            vec![(json!(true), json!(first.id()))],
            vec![(json!(true), json!(second.id()))],
        ),
        "{checked:?}"
    );
}
