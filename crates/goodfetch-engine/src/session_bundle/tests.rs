//! Session export bundles through their interface: a Session sealed to a
//! public key or a passphrase shows none of its values and opens under its
//! key alone; each failure to open is `session_export_open_failed`'s, naming
//! the reference and never the value; a document of a version this engine
//! does not read is `format_version_unsupported`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use age::x25519;
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::refusals::RefusalCase;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::session_export::{
    ExportedCookie, ExportedOrigin, SameSite, SessionExport, SessionValue, StorageItem,
};
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{Key, Limits, OpenError, PublicKey, To, document, open, seal, seal_document};

/// Little scrypt work, so a test's passphrase opens at once.
const WORK: u8 = 2;

/// Limits a test's bundle meets or passes at once.
const LIMITS: Limits = Limits {
    work_factor: WORK,
    document_bytes: 1_024,
};

fn reference(text: &str) -> SecretReference {
    SecretReference::parse(text).unwrap()
}

/// A Session with `cookie` as its one cookie's value and storage in each
/// of its two kinds.
fn session(cookie: &str) -> SessionExport {
    SessionExport {
        format_version: ProtocolVersion { major: 0, minor: 1 },
        cookies: vec![ExportedCookie {
            name: "sid".to_owned(),
            value: SessionValue::new(cookie),
            domain: ".shop.example".to_owned(),
            path: "/".to_owned(),
            expires: Some(1_798_761_600),
            http_only: true,
            secure: true,
            same_site: Some(SameSite::Lax),
        }],
        origins: vec![ExportedOrigin {
            origin: "https://shop.example".to_owned(),
            local_storage: vec![StorageItem {
                name: "cart".to_owned(),
                value: SessionValue::new("3 items"),
            }],
            session_storage: vec![StorageItem {
                name: "step".to_owned(),
                value: SessionValue::new("payment"),
            }],
        }],
    }
}

fn passphrase(value: &str) -> To {
    To::Passphrase {
        passphrase: SecretString::from(value.to_owned()),
        work_factor: WORK,
    }
}

/// Opens `bundle` under `value`, given as the reference `env:LOGIN_KEY`.
fn opened(bundle: &[u8], value: &SecretString) -> Result<SessionExport, OpenError> {
    open(
        bundle,
        &Key {
            reference: &reference("env:LOGIN_KEY"),
            value,
        },
        LIMITS,
    )
}

/// The first half of `bundle`.
fn cut_short(mut bundle: Vec<u8>) -> Vec<u8> {
    bundle.truncate(bundle.len() / 2);
    bundle
}

/// How a bundle is sealed.
enum Sealing {
    PublicKey,
    Passphrase,
}

/// A key of `sealing` to seal to, and its value, which opens it.
fn sealed_and_key(sealing: &Sealing) -> (To, SecretString) {
    match sealing {
        Sealing::PublicKey => {
            let identity = x25519::Identity::generate();
            let to = To::PublicKey(identity.to_public().to_string().parse().unwrap());
            // An identity file's text, comment lines and all, as `age-keygen`
            // writes it.
            let value = format!(
                "# created: 2026-01-05T09:00:00Z\n# public key: {}\n{}\n",
                identity.to_public(),
                identity.to_string().expose_secret()
            );
            (to, SecretString::from(value))
        }
        Sealing::Passphrase => (
            passphrase("correct horse battery staple"),
            SecretString::from("correct horse battery staple"),
        ),
    }
}

/// `error`'s code and details, as a call answers it.
fn answered(error: &OpenError) -> (&'static str, Value) {
    let answered = error.to_call_error();
    (
        answered.code(),
        Value::Object(answered.details().unwrap().clone()),
    )
}

/// Covers: story:4-68, format:session-export
#[rstest]
#[case::to_a_public_key(&Sealing::PublicKey)]
#[case::to_a_passphrase(&Sealing::Passphrase)]
fn a_bundle_shows_no_cookie_and_opens_under_its_key(#[case] sealing: &Sealing) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let cookie = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let (to, value) = sealed_and_key(sealing);

    let bundle = seal(&session(&cookie), &to);

    assert_eq!(
        (
            canaries
                .leaks(&[Output::Bytes {
                    name: "bundle",
                    bytes: &bundle,
                }])
                .unwrap(),
            bundle.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"),
            opened(&bundle, &value).unwrap(),
        ),
        (vec![], true, session(&cookie))
    );
}

/// A wrong passphrase names the reference it came from, never itself.
/// Covers: story:4-68, error:session_export_open_failed
#[test]
fn a_wrong_passphrase_names_its_reference_and_never_its_value() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let wrong = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let bundle = seal(&session("abc"), &passphrase("correct horse battery staple"));

    let refused = opened(&bundle, &SecretString::from(wrong)).unwrap_err();

    let (code, details) = answered(&refused);
    assert_eq!(
        (
            code,
            details["reference"].clone(),
            details["failure"].clone(),
            canaries
                .leaks(&[
                    Output::Bytes {
                        name: "message",
                        bytes: refused.to_call_error().message().as_bytes(),
                    },
                    Output::Bytes {
                        name: "details",
                        bytes: details.to_string().as_bytes(),
                    },
                ])
                .unwrap(),
        ),
        (
            "session_export_open_failed",
            json!("env:LOGIN_KEY"),
            json!("wrong_key"),
            vec![]
        )
    );
}

/// What a key that opens nothing is told.
/// Covers: error:session_export_open_failed
#[rstest]
#[case::another_identity(&Sealing::PublicKey, "wrong_key")]
#[case::a_passphrase_for_a_public_key(&Sealing::Passphrase, "not_an_identity")]
fn a_bundle_to_a_public_key_opens_under_its_identity_alone(
    #[case] other: &Sealing,
    #[case] failure: &str,
) {
    let bundle = seal(&session("abc"), &sealed_and_key(&Sealing::PublicKey).0);
    let (_, value) = sealed_and_key(other);

    let refused = opened(&bundle, &value).unwrap_err();

    assert_eq!(answered(&refused).1["failure"], json!(failure), "{refused}");
}

/// A bundle cut short, bytes that are no age file, and an age file that
/// holds no Session are no bundle.
/// Covers: error:session_export_open_failed
#[rstest]
#[case::cut_short(cut_short(seal(&session("abc"), &passphrase("p"))))]
#[case::no_age_file(b"{\"cookies\": []}".to_vec())]
#[case::no_session_inside(seal_document(b"not json", &passphrase("p")))]
#[case::a_field_no_version_has(seal_document(
    br#"{"format_version":"0.1","cookies":[],"origins":[],"extra":1}"#,
    &passphrase("p")
))]
fn bytes_of_no_bundle_are_not_a_bundle(#[case] bundle: Vec<u8>) {
    let refused = opened(&bundle, &SecretString::from("p")).unwrap_err();

    assert_eq!(
        answered(&refused).1["failure"],
        json!("not_a_bundle"),
        "{refused}"
    );
}

/// A passphrase bundle that asks more scrypt work than the import allows
/// is refused before the work, and one past the size limit after it.
/// Covers: error:session_export_open_failed
#[rstest]
#[case::more_work(
    seal(
        &session("abc"),
        &To::Passphrase {
            passphrase: SecretString::from("p"),
            work_factor: WORK + 1,
        }
    ),
    "excessive_work"
)]
#[case::too_large(
    seal_document(&vec![b' '; 1_025], &passphrase("p")),
    "too_large"
)]
fn a_bundle_past_the_import_limits_is_refused(#[case] bundle: Vec<u8>, #[case] failure: &str) {
    let refused = opened(&bundle, &SecretString::from("p")).unwrap_err();

    assert_eq!(answered(&refused).1["failure"], json!(failure), "{refused}");
}

/// Each generated refusal case of the format, fabricated inside a bundle
/// with a field this engine does not know: `format_version_unsupported`,
/// with the rule's verdict (ADR 0066).
/// Covers: story:4-68, format:session-export, error:format_version_unsupported
#[rstest]
fn each_generated_refusal_of_a_bundle_gets_its_verdict(
    #[files("../goodfetch-protocol/generated/refusals/session-export/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();
    let document = json!({
        "format_version": case.found,
        "cookies": [],
        "origins": [],
        "identity": {},
    });
    let bundle = seal_document(document.to_string().as_bytes(), &passphrase("p"));

    let refused = opened(&bundle, &SecretString::from("p")).unwrap_err();

    let OpenError::Version {
        found,
        compatibility,
    } = &refused
    else {
        panic!("{refused}");
    };
    let (code, details) = answered(&refused);
    assert_eq!(
        (
            *found,
            *compatibility,
            code,
            details["format"].clone(),
            details["found"].clone()
        ),
        (
            case.found,
            case.compatibility,
            "format_version_unsupported",
            json!("session-export"),
            json!(case.found)
        )
    );
}

/// The corpus's document, sealed in a bundle, opens to the Session it holds
/// (ADR 0066).
/// Covers: format:session-export
#[test]
fn the_corpus_document_opens_whole() {
    const HEAD: &str = include_str!("../../../../compat/session-export/head/session.json");
    let bundle = seal_document(HEAD.as_bytes(), &passphrase("p"));

    let session = opened(&bundle, &SecretString::from("p")).unwrap();

    assert_eq!(str::from_utf8(&document(&session)).unwrap(), HEAD);
}

/// A public key parses from the text `age-keygen` prints, and other text
/// does not.
#[test]
fn only_an_age_public_key_parses_as_one() {
    let identity = x25519::Identity::generate();

    assert_eq!(
        (
            identity
                .to_public()
                .to_string()
                .parse::<PublicKey>()
                .is_ok(),
            "ssh-ed25519 AAAA".parse::<PublicKey>().is_ok()
        ),
        (true, false)
    );
}

mod session_bundle_file {
    goodfetch_test_support::fuzz_replay!(
        crate::session_bundle::fuzz_session_bundle,
        "fuzz/session_bundle"
    );
}
