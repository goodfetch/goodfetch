//! The identity site (story 40): a request's Web Bot Auth signature is
//! checked as it arrives with Cloudflare's `web-bot-auth`, against the key
//! directory its `Signature-Agent` names, and the check is recorded with the
//! request. The lab's identity cases (`identity-signature.test.ts`) are
//! carried over: a request signed with RFC 9421's Ed25519 test key verifies,
//! bound to its authority; a replay on another authority, a key the
//! directory no longer publishes and a directory whose key IDs are not their
//! thumbprints are refused; and the oracle judges a task's signatures from
//! those records.
#![expect(
    clippy::disallowed_types,
    reason = "the test serves the agent's key directory on its own socket"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the agent's key directory is served on a thread of the test's own"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::SigningKey;
use indexmap::IndexMap;
use serde_json::{Value, json};
use web_bot_auth::components::{
    CoveredComponent, DerivedComponent, HTTPField, HTTPFieldParametersSet,
};
use web_bot_auth::keyring::{Algorithm, Thumbprintable};
use web_bot_auth::message_signatures::{MessageSigner, UnsignedMessage};

use crate::proving_ground::ProvingGround;
use crate::raw_clients;

const SEED: u64 = 162;

/// RFC 9421, Appendix B.1.4's Ed25519 test key, as the lab's test used it.
const RFC_KEY: [u8; 32] = [
    0x9f, 0x83, 0x62, 0xf8, 0x7a, 0x48, 0x4a, 0x95, 0x4e, 0x6e, 0x74, 0x0c, 0x5b, 0x4c, 0x0e, 0x84,
    0x22, 0x91, 0x39, 0xa2, 0x0a, 0xa8, 0xab, 0x56, 0xff, 0x66, 0x58, 0x6f, 0x6a, 0x7d, 0x29, 0xc5,
];

/// Another agent key, which the directory does not always publish.
const OTHER_KEY: [u8; 32] = [7; 32];

/// A key's public half as a JWK, and its RFC 7638 thumbprint.
fn jwk(private: &[u8; 32]) -> (Thumbprintable, String) {
    let public = SigningKey::from_bytes(private).verifying_key().to_bytes();
    let key = Thumbprintable::OKP {
        crv: "Ed25519".to_owned(),
        x: BASE64URL_NOPAD.encode(&public),
    };
    let thumbprint = key.b64_thumbprint();
    (key, thumbprint)
}

/// The components a signature covers, with what it registers.
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

/// `Signature-Input` and `Signature` of `components`, signed by `key` with
/// `tag`.
fn sign(
    key: &[u8; 32],
    tag: &str,
    label: &'static str,
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
        nonce: BASE64URL_NOPAD.encode(&[u8::try_from(label.len()).expect("a short label"); 32]),
        tag: tag.to_owned(),
    }
    .generate_signature_headers_content(
        &mut covered,
        Duration::from_secs(60),
        Algorithm::Ed25519,
        key,
    )
    .expect("the message signs");
    (covered.input, covered.signature)
}

fn authority(value: &str, request: bool) -> (CoveredComponent, String) {
    (
        CoveredComponent::Derived(DerivedComponent::Authority { req: request }),
        value.to_owned(),
    )
}

fn field(name: &str, value: &str) -> (CoveredComponent, String) {
    (
        CoveredComponent::HTTP(HTTPField {
            name: name.to_owned(),
            parameters: HTTPFieldParametersSet(vec![]),
        }),
        value.to_owned(),
    )
}

/// Serves a key directory on loopback for the rest of the test: `published`
/// keys, each named `kid` by its thumbprint (or `wrong-kid`), each proving
/// possession by signing the response. Its origin.
fn directory(published: &[[u8; 32]], thumbprint_kids: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a directory port");
    let origin = format!("http://{}", listener.local_addr().expect("a bound address"));
    let host = origin.trim_start_matches("http://").to_owned();
    let keys: Vec<Value> = published
        .iter()
        .map(|key| {
            let (jwk, thumbprint) = jwk(key);
            let mut value = serde_json::to_value(jwk).expect("a JWK");
            value["kid"] = json!(if thumbprint_kids {
                thumbprint
            } else {
                "wrong-kid".to_owned()
            });
            value
        })
        .collect();
    let body = json!({ "keys": keys }).to_string();
    let mut inputs = Vec::new();
    let mut signatures = Vec::new();
    for (index, key) in published.iter().enumerate() {
        let label: &'static str = ["sig1", "sig2", "sig3"][index];
        let (input, signature) = sign(
            key,
            "http-message-signatures-directory",
            label,
            IndexMap::from([authority(&host, true)]),
        );
        inputs.push(input);
        signatures.push(signature);
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/http-message-signatures-directory+json\r\nSignature-Input: {}\r\nSignature: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        inputs.join(", "),
        signatures.join(", "),
        body.len()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    origin
}

/// The headers of a request to `host`'s `path` signed by `key` for
/// `signed_authority`, naming the directory at `origin`.
fn signed_headers(
    key: &[u8; 32],
    origin: &str,
    signed_authority: &str,
) -> Vec<(&'static str, String)> {
    let agent = format!("\"{origin}\"");
    let (input, signature) = sign(
        key,
        "web-bot-auth",
        "sig1",
        IndexMap::from([
            authority(signed_authority, false),
            field("signature-agent", &agent),
        ]),
    );
    vec![
        ("Signature-Agent", agent),
        ("Signature-Input", input),
        ("Signature", signature),
    ]
}

/// The host name of task `id`.
fn task_host(proving_ground: &ProvingGround, id: &str) -> String {
    let url = proving_ground.task(id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    url.trim_start_matches("https://")
        .split(':')
        .next()
        .expect("a host")
        .to_owned()
}

/// Sends `path` on `host` with `headers`, and returns its recorded check.
fn checked(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
    headers: &[(&'static str, String)],
) -> Value {
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let arrived = raw_clients::get(proving_ground, host, path, &headers);
    assert_eq!(arrived.status(), Some(200));
    let observations = proving_ground.observations();
    observations["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .rev()
        .find(|request| request["host"] == host && request["target"] == path)
        .map(|request| request["message_signature"]["verification"].clone())
        .expect("the request is recorded")
}

/// Covers: story:2-40
#[test]
fn a_signed_request_verifies_against_its_agents_key_directory() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = directory(&[RFC_KEY], true);
    let host = task_host(&proving_ground, "identity/1");
    let own = format!("{host}:{}", proving_ground.port());

    let check = checked(
        &proving_ground,
        &host,
        "/catalog/1",
        &signed_headers(&RFC_KEY, &origin, &own),
    );

    assert_eq!(
        check,
        json!({ "verified": true, "key_id": jwk(&RFC_KEY).1, "error": null })
    );
}

#[test]
fn a_signature_replayed_on_another_authority_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = directory(&[RFC_KEY], true);
    let host = task_host(&proving_ground, "identity/1");
    let other = format!("2.identity.test:{}", proving_ground.port());

    let check = checked(
        &proving_ground,
        &host,
        "/catalog/1",
        &signed_headers(&RFC_KEY, &origin, &other),
    );

    assert_eq!(check["verified"], false, "{check}");
}

#[test]
fn a_key_the_directory_no_longer_publishes_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = directory(&[RFC_KEY], true);
    let host = task_host(&proving_ground, "identity/1");
    let own = format!("{host}:{}", proving_ground.port());

    let check = checked(
        &proving_ground,
        &host,
        "/catalog/1",
        &signed_headers(&OTHER_KEY, &origin, &own),
    );

    assert_eq!(check["verified"], false, "{check}");
}

#[test]
fn a_directory_whose_key_ids_are_not_their_thumbprints_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = directory(&[RFC_KEY], false);
    let host = task_host(&proving_ground, "identity/1");
    let own = format!("{host}:{}", proving_ground.port());

    let check = checked(
        &proving_ground,
        &host,
        "/catalog/1",
        &signed_headers(&RFC_KEY, &origin, &own),
    );

    assert_eq!(check["verified"], false, "{check}");
    assert!(
        check["error"]
            .as_str()
            .is_some_and(|error| error.contains("thumbprint")),
        "{check}"
    );
}

#[test]
fn a_rotated_directory_publishing_both_keys_verifies_each() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = directory(&[RFC_KEY, OTHER_KEY], true);
    let host = task_host(&proving_ground, "identity/1");
    let own = format!("{host}:{}", proving_ground.port());

    let before = checked(
        &proving_ground,
        &host,
        "/catalog/1",
        &signed_headers(&RFC_KEY, &origin, &own),
    );
    let after = checked(
        &proving_ground,
        &host,
        "/catalog/2",
        &signed_headers(&OTHER_KEY, &origin, &own),
    );

    assert_eq!(before["key_id"], jwk(&RFC_KEY).1);
    assert_eq!(after["key_id"], jwk(&OTHER_KEY).1);
}

/// Covers: story:2-40
#[test]
fn the_oracle_judges_a_tasks_signatures_from_the_records() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = directory(&[RFC_KEY], true);
    let identified = task_host(&proving_ground, "identity/1");
    let standard = task_host(&proving_ground, "identity/2");
    let own = format!("{identified}:{}", proving_ground.port());
    let mut records = Vec::new();
    for page in 1..=3 {
        let path = format!("/catalog/{page}");
        let headers = signed_headers(&RFC_KEY, &origin, &own);
        let headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let arrived = raw_clients::get(&proving_ground, &identified, &path, &headers);
        records.extend(records_on(&String::from_utf8_lossy(arrived.body())));
    }
    let submission = |task: &str, records: &[Value]| json!({ "task": task, "status": "success", "records": records });

    let signed = proving_ground.judge(&submission("identity/1", &records));
    raw_clients::get(&proving_ground, &identified, "/catalog/1", &[]);
    let one_unsigned = proving_ground.judge(&submission("identity/1", &records));
    let headers = signed_headers(
        &RFC_KEY,
        &origin,
        &format!("{standard}:{}", proving_ground.port()),
    );
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let mut standard_records = Vec::new();
    for page in 1..=3 {
        let arrived = raw_clients::get(
            &proving_ground,
            &standard,
            &format!("/catalog/{page}"),
            &headers,
        );
        standard_records.extend(records_on(&String::from_utf8_lossy(arrived.body())));
    }
    let standard_signed = proving_ground.judge(&submission("identity/2", &standard_records));

    assert_eq!(signed["outcome"], "verified", "{signed}");
    assert_eq!(one_unsigned["outcome"], "false_success", "{one_unsigned}");
    assert_eq!(
        one_unsigned["signature_faults"],
        json!([{ "target": "/catalog/1", "fault": "unsigned" }])
    );
    assert_eq!(
        standard_signed["outcome"], "false_success",
        "{standard_signed}"
    );
    assert_eq!(
        standard_signed["signature_faults"].as_array().map(Vec::len),
        Some(3)
    );
}

/// The `id` and `price` of every Record on a shelf page's HTML.
fn records_on(html: &str) -> Vec<Value> {
    html.split("<article")
        .skip(1)
        .filter_map(|article| {
            let field = |class: &str| {
                article
                    .split(&format!("class=\"{class}\">"))
                    .nth(1)
                    .and_then(|rest| rest.split('<').next())
                    .map(str::to_owned)
            };
            Some(json!({ "id": field("id")?, "price": field("price")? }))
        })
        .collect()
}
