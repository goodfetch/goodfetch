//! Standard Webhooks signatures through the module's interface (ADR 0017;
//! #666): the shared vectors, which the TypeScript and Python packages
//! pass too, the header checks of ADR 0067's hostile-input table, and the
//! signing key's canary.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{
    Canaries, HeaderSource, HostileTables, Output, SecretKind, SignedVector, VerificationCase,
    VerificationOutcome, webhook_signatures,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::{
    DeliveryRefused, EndpointId, KeyError, Message, Received, Signer, SigningKey, Unverified,
    Verifier, VerifyingKey, body,
};

/// A resolved key, as its Secret reference gives it.
fn secret(key: &str) -> SecretString {
    SecretString::from(key.to_owned())
}

/// The endpoint ID a case names.
fn endpoint(id: Option<&String>) -> Option<EndpointId> {
    id.map(|id| EndpointId::new(id.as_str()).unwrap())
}

/// The signer of a signed vector.
fn signer(vector: &SignedVector) -> Signer {
    let keys = vector
        .keys
        .iter()
        .map(|key| SigningKey::from_secret(&secret(key)).unwrap())
        .collect();
    Signer::new(keys, endpoint(vector.endpoint.as_ref())).unwrap()
}

/// What the verifier answers for `case`, as the vectors name it.
fn outcome(case: &VerificationCase) -> (VerificationOutcome, Option<u64>) {
    let keys = case
        .trusted
        .iter()
        .map(|key| VerifyingKey::from_secret(&secret(key)).unwrap())
        .collect();
    let verifier = Verifier::new(keys, endpoint(case.endpoint.as_ref())).unwrap();
    let received = Received {
        id: case.id.as_deref(),
        timestamp: case.timestamp.as_deref(),
        signature: case.signature.as_deref(),
        body: case.body.as_bytes(),
    };
    let answer = if case.run_epoch.is_some() {
        verifier
            .verify(&received, case.now)
            .map(|verified| Some(verified.run_epoch))
    } else {
        verifier
            .verify_signature(&received, case.now)
            .map(|()| None)
    };
    match answer {
        Ok(run_epoch) => (VerificationOutcome::Verified, run_epoch),
        Err(unverified) => (
            match unverified {
                Unverified::MissingHeader(_) => VerificationOutcome::MissingHeader,
                Unverified::TimestampInvalid => VerificationOutcome::TimestampInvalid,
                Unverified::TimestampTooOld => VerificationOutcome::TimestampTooOld,
                Unverified::TimestampTooNew => VerificationOutcome::TimestampTooNew,
                Unverified::NoMatchingSignature => VerificationOutcome::NoMatchingSignature,
                Unverified::NotADelivery => panic!("{}: not a delivery", case.name),
            },
            None,
        ),
    }
}

/// Every signed vector of the shared file, the specification's published
/// `v1` and `v2a` ones among them, is what the signer sends, and each of
/// its keys verifies it on its own, a rotation's old key as its new one.
///
/// Covers: story:4-88
#[test]
fn every_signed_vector_is_what_the_signer_sends_and_each_key_verifies() {
    let vectors = webhook_signatures();

    let misses: Vec<String> = vectors
        .signed
        .iter()
        .filter_map(|vector| {
            let message = Message {
                id: &vector.id,
                timestamp: vector.timestamp,
                body: vector.body.as_bytes(),
            };
            let sent = signer(vector).signature(&message);
            let timestamp = vector.timestamp.to_string();
            let unverified: Vec<&String> = vector
                .keys
                .iter()
                .filter(|key| {
                    let key = SigningKey::from_secret(&secret(key)).unwrap();
                    let verifier = Verifier::new(
                        vec![key.verifying_key()],
                        endpoint(vector.endpoint.as_ref()),
                    )
                    .unwrap();
                    let received = Received {
                        id: Some(&vector.id),
                        timestamp: Some(&timestamp),
                        signature: Some(&sent),
                        body: vector.body.as_bytes(),
                    };
                    verifier
                        .verify_signature(&received, vector.timestamp)
                        .is_err()
                })
                .collect();
            (sent != vector.signature || !unverified.is_empty())
                .then(|| format!("{}: sent {sent}, unverified {unverified:?}", vector.name))
        })
        .collect();

    assert_eq!(misses, Vec::<String>::new());
}

/// Every verification case of the shared file gets its outcome: either
/// key of a rotation verifies and a third does not, a body whose run epoch
/// changed after signing does not, a timestamp more than five minutes in
/// the past or the future is refused, and a verified delivery gives its
/// run epoch.
///
/// Covers: story:4-88
#[test]
fn every_verification_case_gets_its_outcome() {
    let vectors = webhook_signatures();

    let misses: Vec<String> = vectors
        .verifications
        .iter()
        .filter_map(|case| {
            let got = outcome(case);
            (got != (case.outcome, case.run_epoch)).then(|| format!("{}: {got:?}", case.name))
        })
        .collect();

    assert_eq!(misses, Vec::<String>::new());
}

/// A delivery's body puts the run epoch beside its data, byte for byte as
/// the shared rotation vector signs it.
///
/// Covers: story:4-88
#[test]
fn a_delivery_body_carries_the_run_epoch_beside_its_data() {
    let vectors = webhook_signatures();
    let rotation = vectors
        .signed
        .iter()
        .find(|vector| vector.name == "rotation_two_v1_keys")
        .unwrap();

    let sent = body(
        7,
        &json!({ "event": "run.finished", "run_id": "01JDQ8X0000000000000000000" }),
    )
    .unwrap();

    assert_eq!(String::from_utf8(sent).unwrap(), rotation.body);
}

/// The signer for the shared rotation's two `v1` keys.
fn rotation_signer() -> Signer {
    let vectors = webhook_signatures();
    signer(
        vectors
            .signed
            .iter()
            .find(|vector| vector.name == "rotation_two_v1_keys")
            .unwrap(),
    )
}

/// Every webhook header case of the hostile-input table is refused before
/// a delivery is signed, naming the header and never its value; a plain
/// header goes first, before the three Standard Webhooks headers.
///
/// Covers: story:4-88, error:header_value_invalid
#[test]
fn a_webhook_header_holding_a_cr_an_lf_or_a_nul_is_refused() {
    let tables = HostileTables::committed().unwrap();
    let signer = rotation_signer();
    let message = Message {
        id: "01JDQ8X2Y5S6T7V8W9XAYBZC0D",
        timestamp: 1_764_000_000,
        body: b"{}",
    };

    let answers: Vec<(String, Option<&'static str>)> = tables
        .header_values
        .cases
        .iter()
        .filter(|case| case.input.source == HeaderSource::Webhook)
        .map(|case| {
            let refused = signer
                .headers(
                    &message,
                    &[(case.input.name.as_str(), case.input.value.as_str())],
                )
                .err();
            let code = match &refused {
                Some(DeliveryRefused::Header(invalid)) => Some(invalid.error().code()),
                _ => None,
            };
            (case.name.clone(), code)
        })
        .collect();
    let sent = signer.headers(&message, &[("X-Note", "plain")]).unwrap();

    assert_eq!(answers.len(), 4);
    assert!(
        answers
            .iter()
            .all(|(_, code)| *code == Some("header_value_invalid")),
        "{answers:?}"
    );
    let names: Vec<&str> = sent.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "X-Note",
            "webhook-id",
            "webhook-timestamp",
            "webhook-signature"
        ]
    );
}

/// A delivery ID that is empty, holds a `.` or a line break is refused:
/// its signed text would be ambiguous, or its header would end early.
#[rstest]
#[case::empty("", false)]
#[case::dot("01JDQ8X2.Y5S6", false)]
#[case::line_break("01JDQ8X2\r\nX-Injected: yes", true)]
fn a_delivery_id_that_would_blur_the_signed_text_is_refused(
    #[case] id: &str,
    #[case] header: bool,
) {
    let message = Message {
        id,
        timestamp: 1_764_000_000,
        body: b"{}",
    };

    let refused = rotation_signer().headers(&message, &[]).unwrap_err();

    assert_eq!(matches!(refused, DeliveryRefused::Header(_)), header);
}

/// A key the module cannot use is refused for a reason that never quotes
/// it.
#[rstest]
#[case::no_prefix("MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw", KeyError::Prefix)]
#[case::not_base64("whsec_not base64!", KeyError::Encoding)]
#[case::too_short("whsec_AQIDBAUGBwgJCgsMDQ4PEBESExQVFhc=", KeyError::Length(23))]
#[case::seed_too_short("whsk_AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcY", KeyError::Length(24))]
#[case::public_key_to_sign(
    "whpk_uWKK13PfwqyOgrqT0OrWKcS2uVD7e89+oUKnkoPjx7w=",
    KeyError::PublicKeyCannotSign
)]
fn a_key_that_cannot_sign_is_refused(#[case] key: &str, #[case] expected: KeyError) {
    let refused = SigningKey::from_secret(&secret(key)).unwrap_err();

    assert_eq!(refused, expected);
}

/// A signer or verifier takes at least one key, and a `v2a` key takes the
/// endpoint ID it signs for.
#[test]
fn a_signer_takes_a_key_and_a_v2a_key_its_endpoint() {
    let v2a = SigningKey::from_secret(&secret("whsk_4uKPjji/dhB8w6GW3YYDnIMYvXcdHrvH6lidTr4Z+EY="))
        .unwrap();
    let public = v2a.verifying_key();

    assert_eq!(
        (
            Signer::new(Vec::new(), None).unwrap_err(),
            Verifier::new(vec![public], None).unwrap_err(),
            Signer::new(vec![v2a], None).unwrap_err(),
            EndpointId::new("ep.1").is_err(),
        ),
        (
            KeyError::NoKey,
            KeyError::NoEndpoint,
            KeyError::NoEndpoint,
            true
        )
    );
}

/// A webhook signing secret planted as a canary is in no `Debug` form of
/// the keys, the signer or the verifier, nor in a refusal's text: its
/// value, the key's bytes, shows only in the signature it makes.
///
/// Covers: story:4-88
#[test]
fn a_webhook_signing_secret_shows_in_no_debug_form_or_refusal() {
    let mut canaries = Canaries::new(666);
    let value = canaries
        .plant(SecretKind::WebhookSigningSecret)
        .value()
        .to_owned();
    let encoded = data_encoding::BASE64.encode(value.as_bytes());
    let key = SigningKey::from_secret(&secret(&format!("whsec_{encoded}"))).unwrap();
    let verifying = key.verifying_key();
    let signer = Signer::new(vec![key], None).unwrap();
    let verifier = Verifier::new(vec![verifying.clone()], None).unwrap();
    let refusals = [
        SigningKey::from_secret(&secret(&format!("whsec_{}", &encoded[..20]))).unwrap_err(),
        SigningKey::from_secret(&secret(&format!("{encoded}!"))).unwrap_err(),
        SigningKey::from_secret(&secret(&format!("whsec_{value}"))).unwrap_err(),
    ];

    let shown = format!(
        "{signer:?} {verifier:?} {verifying:?} {}",
        refusals
            .iter()
            .map(|refusal| format!("{refusal} {refusal:?}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let leaks: Vec<String> = canaries
        .leaks(&[Output::Bytes {
            name: "shown",
            bytes: shown.as_bytes(),
        }])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{hit:?}"))
        .collect();

    assert_eq!(leaks, Vec::<String>::new());
}
