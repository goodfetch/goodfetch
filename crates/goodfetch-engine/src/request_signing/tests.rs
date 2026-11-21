//! A Session's signer (#1551): what it signs verifies with Cloudflare's
//! `web-bot-auth` against the key directory its `Signature-Agent` names,
//! bound to the request's authority, and a key it cannot sign with is
//! refused without quoting it.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Entropy (ADR 0059)"
)]

use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD};
use ed25519_dalek::SigningKey;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;
use url::Url;
use web_bot_auth::components::{CoveredComponent, DerivedComponent, HTTPField};
use web_bot_auth::keyring::{JSONWebKeySet, KeyRing, Thumbprintable};
use web_bot_auth::message_signatures::{MessageVerifier, SignedMessage};
use web_bot_auth::{SignatureAgentLink, WebBotAuthVerifier};

use super::{
    Directory, DirectoryError, KeyError, RequestSigner, SignedHeaders, directory, new_key,
    public_key,
};
use crate::entropy::Entropy;

/// RFC 9421, Appendix B.1.4's Ed25519 test key.
const RFC_KEY: [u8; 32] = [
    0x9f, 0x83, 0x62, 0xf8, 0x7a, 0x48, 0x4a, 0x95, 0x4e, 0x6e, 0x74, 0x0c, 0x5b, 0x4c, 0x0e, 0x84,
    0x22, 0x91, 0x39, 0xa2, 0x0a, 0xa8, 0xab, 0x56, 0xff, 0x66, 0x58, 0x6f, 0x6a, 0x7d, 0x29, 0xc5,
];

fn public_x(private: &[u8; 32]) -> String {
    BASE64URL_NOPAD.encode(SigningKey::from_bytes(private).verifying_key().as_bytes())
}

/// `private` as the private JSON Web Key a Secret holds.
fn private_jwk(private: &[u8; 32]) -> SecretString {
    SecretString::from(
        json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "d": BASE64URL_NOPAD.encode(private),
            "x": public_x(private),
        })
        .to_string(),
    )
}

/// An inline key directory publishing `private`'s public key under its
/// thumbprint.
fn inline_directory(private: &[u8; 32]) -> String {
    let key = Thumbprintable::OKP {
        crv: "Ed25519".to_owned(),
        x: public_x(private),
    };
    let mut published = serde_json::to_value(&key).expect("a JWK");
    published["kid"] = json!(key.b64_thumbprint());
    let directory = json!({ "keys": [published] }).to_string();
    format!(
        "data:application/http-message-signatures-directory;base64,{}",
        BASE64.encode(directory.as_bytes())
    )
}

fn url(url: &str) -> Url {
    Url::parse(url).expect("a URL")
}

fn signer(private: &[u8; 32], agent: &str) -> RequestSigner {
    RequestSigner::new(&private_jwk(private), agent, Entropy::seeded(1551)).expect("the key signs")
}

/// A request to `authority` as a site receives it, with `headers`.
struct Received {
    authority: String,
    headers: SignedHeaders,
}

impl SignedMessage for Received {
    fn lookup_component(&self, name: &CoveredComponent) -> Vec<String> {
        match name {
            CoveredComponent::Derived(DerivedComponent::Authority { .. }) => {
                vec![self.authority.clone()]
            }
            CoveredComponent::HTTP(HTTPField { name, .. }) => match name.as_str() {
                "signature-agent" => vec![self.headers.signature_agent.clone()],
                "signature-input" => vec![self.headers.signature_input.clone()],
                "signature" => vec![self.headers.signature.clone()],
                _ => vec![],
            },
            CoveredComponent::Derived(_) => vec![],
        }
    }
}

/// Whether a site at `authority` verifies `headers` against the directory
/// the `Signature-Agent` names inline, and the key ID it read.
fn verify(authority: &str, headers: SignedHeaders) -> (bool, Option<String>) {
    let received = Received {
        authority: authority.to_owned(),
        headers,
    };
    let verifier = WebBotAuthVerifier::parse(&received).expect("a Web Bot Auth message");
    let key_id = verifier
        .get_parsed_label()
        .base
        .parameters
        .details
        .keyid
        .clone();
    let [SignatureAgentLink::Inline(directory)] = verifier.get_signature_agents().as_slice() else {
        panic!("one inline directory");
    };
    let mut keys = KeyRing::default();
    keys.import_jwks(directory.clone());
    (verifier.verify(&keys, None).is_ok(), key_id)
}

#[test]
fn a_signed_request_verifies_against_the_directory_its_agent_names() {
    let signer = signer(&RFC_KEY, &inline_directory(&RFC_KEY));

    let headers = signer
        .sign(&url("https://identity.test:8443/observations"))
        .expect("the request signs");

    assert_eq!(
        verify("identity.test:8443", headers),
        (
            true,
            Some(public_key(&private_jwk(&RFC_KEY)).expect("a key").kid)
        )
    );
}

#[test]
fn a_signature_is_bound_to_its_authority() {
    let signer = signer(&RFC_KEY, &inline_directory(&RFC_KEY));

    let headers = signer
        .sign(&url("https://identity.test/"))
        .expect("the request signs");

    assert!(!verify("elsewhere.test", headers).0);
}

#[rstest]
#[case::the_scheme_s_own_port("https://Identity.TEST:443/a", "identity.test")]
#[case::another_port("http://identity.test:8080/a", "identity.test:8080")]
#[case::an_ip_v6_address("http://[::1]:8080/", "[::1]:8080")]
fn the_authority_is_the_host_and_a_port_not_the_scheme_s_own(
    #[case] request: &str,
    #[case] authority: &str,
) {
    let signer = signer(&RFC_KEY, &inline_directory(&RFC_KEY));

    let headers = signer.sign(&url(request)).expect("the request signs");

    assert!(verify(authority, headers).0);
}

#[rstest]
#[case::data("data:text/plain,a")]
#[case::a_blob("blob:https://identity.test/1")]
#[case::a_file("file:///tmp/a")]
fn a_request_that_carries_no_headers_is_not_signed(#[case] request: &str) {
    let signer = signer(&RFC_KEY, "https://bot.example");

    assert_eq!(signer.sign(&url(request)), None);
}

#[test]
fn the_agent_is_sent_as_a_string_and_each_signature_has_a_fresh_nonce() {
    let signer = signer(&RFC_KEY, "https://bot.example");

    let first = signer
        .sign(&url("https://identity.test/"))
        .expect("the request signs");
    let second = signer
        .sign(&url("https://identity.test/"))
        .expect("the request signs");

    assert_eq!(first.signature_agent, "\"https://bot.example\"");
    let nonce = |headers: &SignedHeaders| {
        headers
            .signature_input
            .split(';')
            .find_map(|parameter| parameter.strip_prefix("nonce="))
            .map(str::to_owned)
    };
    assert!(nonce(&first).is_some());
    assert_ne!(nonce(&first), nonce(&second));
}

#[rstest]
#[case::not_json("not a key", KeyError::NotJwk)]
#[case::a_public_key(&json!({"kty": "OKP", "crv": "Ed25519", "x": public_x(&RFC_KEY)}).to_string(), KeyError::NotJwk)]
#[case::another_curve(&json!({"kty": "OKP", "crv": "X25519", "d": BASE64URL_NOPAD.encode(&RFC_KEY), "x": public_x(&RFC_KEY)}).to_string(), KeyError::NotEd25519)]
#[case::a_short_private_key(&json!({"kty": "OKP", "crv": "Ed25519", "d": BASE64URL_NOPAD.encode(&RFC_KEY[..31]), "x": public_x(&RFC_KEY)}).to_string(), KeyError::BadPrivateKey)]
#[case::another_public_key(&json!({"kty": "OKP", "crv": "Ed25519", "d": BASE64URL_NOPAD.encode(&RFC_KEY), "x": public_x(&[7; 32])}).to_string(), KeyError::MismatchedPublicKey)]
fn a_key_that_cannot_sign_is_refused_without_quoting_it(
    #[case] key: &str,
    #[case] refused: KeyError,
) {
    let error = RequestSigner::new(
        &SecretString::from(key.to_owned()),
        "https://bot.example",
        Entropy::seeded(1551),
    )
    .expect_err("the key cannot sign");

    assert_eq!(error, refused);
    assert!(
        !error
            .to_string()
            .contains(&BASE64URL_NOPAD.encode(&RFC_KEY))
    );
}

#[rstest]
#[case::a_quote("https://bot.example/\"")]
#[case::a_backslash("https://bot.example/\\")]
#[case::a_line_break("https://bot.example/\n")]
fn an_agent_a_header_string_cannot_carry_is_refused(#[case] agent: &str) {
    let error = RequestSigner::new(&private_jwk(&RFC_KEY), agent, Entropy::seeded(1551))
        .expect_err("the agent cannot be sent");

    assert_eq!(error, KeyError::BadAgent);
}

/// The key directory response `directory` as a site that fetched it from
/// `authority` receives it.
fn fetched(authority: &str, directory: &Directory) -> Received {
    Received {
        authority: authority.to_owned(),
        headers: SignedHeaders {
            signature_agent: String::new(),
            signature_input: directory.signature_input.clone(),
            signature: directory.signature.clone(),
        },
    }
}

/// Whether `label`'s signature in `received` verifies against `keys`, and
/// the key ID and tag it names.
fn proof(
    received: &Received,
    label: &str,
    keys: &KeyRing,
) -> (bool, Option<String>, Option<String>) {
    let verifier = MessageVerifier::parse(received, |(name, _)| name.as_str() == label)
        .expect("a labelled signature");
    let details = verifier.parsed.base.parameters.details.clone();
    (
        verifier.verify(keys, None).is_ok(),
        details.keyid,
        details.tag,
    )
}

#[test]
fn a_directory_publishes_each_key_by_its_thumbprint_and_proves_it_holds_each() {
    let other = [7; 32];
    let keys = [private_jwk(&RFC_KEY), private_jwk(&other)];

    let directory =
        directory(&keys, "Bot.Example:8443", Duration::from_secs(60)).expect("the directory");

    let published: JSONWebKeySet = serde_json::from_str(&directory.body).expect("a key set");
    let mut ring = KeyRing::default();
    ring.import_jwks(published.clone());
    let thumbprints: Vec<String> = published
        .keys
        .iter()
        .map(Thumbprintable::b64_thumbprint)
        .collect();
    let received = fetched("bot.example:8443", &directory);
    let tag = Some("http-message-signatures-directory".to_owned());
    assert_eq!(
        (
            &directory.key_ids,
            &thumbprints,
            proof(&received, "sig1", &ring),
            proof(&received, "sig2", &ring),
        ),
        (
            &thumbprints,
            &vec![
                public_key(&keys[0]).expect("a key").kid,
                public_key(&keys[1]).expect("a key").kid,
            ],
            (true, Some(thumbprints[0].clone()), tag.clone()),
            (true, Some(thumbprints[1].clone()), tag),
        )
    );
    assert!(
        !proof(
            &fetched("elsewhere.example:8443", &directory),
            "sig1",
            &ring
        )
        .0
    );
    assert!(directory.expires > 0);
}

#[rstest]
#[case::empty("")]
#[case::a_url("https://bot.example")]
#[case::a_path("bot.example/keys")]
#[case::user_info("user@bot.example")]
fn an_authority_that_is_not_a_host_and_a_port_is_refused(#[case] authority: &str) {
    let error = directory(&[private_jwk(&RFC_KEY)], authority, Duration::from_secs(60))
        .expect_err("no authority");

    assert!(matches!(error, DirectoryError::BadAuthority(_)), "{error}");
}

#[test]
fn a_directory_of_no_keys_or_a_key_that_cannot_sign_is_refused_without_quoting_it() {
    let held = "not-a-signing-key";

    let none = directory(&[], "bot.example", Duration::from_secs(60)).expect_err("no keys");
    let bad = directory(
        &[private_jwk(&RFC_KEY), SecretString::from(held.to_owned())],
        "bot.example",
        Duration::from_secs(60),
    )
    .expect_err("a bad key");

    assert!(matches!(none, DirectoryError::NoKeys));
    assert!(matches!(
        bad,
        DirectoryError::Key {
            index: 1,
            error: KeyError::NotJwk
        }
    ));
    assert!(!bad.to_string().contains(held));
}

#[test]
fn a_new_key_signs_and_differs_from_the_last() {
    let first = new_key().expect("a key");
    let second = new_key().expect("a key");

    let public = public_key(&first).expect("the new key signs");

    assert_eq!(
        (public.kty, public.crv, public.kid.len()),
        ("OKP", "Ed25519", 43)
    );
    assert_ne!(public, public_key(&second).expect("the new key signs"));
}
