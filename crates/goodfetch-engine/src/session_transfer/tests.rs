//! A Session exported and imported through the Run's Secrets, with a real
//! Audit log in the test's folder: what each records, without a value, and
//! what a move that fails leaves.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059) and read the Audit log's file"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use age::x25519;
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::session_export::{
    ExportedCookie, ExportedOrigin, SessionExport, SessionValue, StorageItem,
};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use rusqlite::Connection;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::{Recipient, export, import};
use crate::audit::{AuditLog, Origin};
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::home::Home;
use crate::mask::Redaction;
use crate::pinned_download::Digest;
use crate::secrets::{Carried, Keychain, RunSecrets, Secrets};
use crate::session_bundle::{Limits, PublicKey};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Little scrypt work, so a test's passphrase opens at once.
const WORK: u8 = 2;

/// Limits every bundle of these tests meets.
const LIMITS: Limits = Limits {
    work_factor: WORK,
    document_bytes: 65_536,
};

fn reference(text: &str) -> SecretReference {
    SecretReference::parse(text).unwrap()
}

/// A Run's Secrets over `variables`, recorded in the Audit log of `env`'s
/// home.
fn secrets(env: &TestEnv, variables: &[(&str, &str)]) -> RunSecrets {
    let home = Home::new(env.home().to_path_buf());
    Secrets::new(
        Carried {
            environment: variables
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect(),
            keychain: Keychain::absent(),
            home: home.clone(),
            entropy: Entropy::seeded(env.seed()),
        },
        Arc::new(AuditLog::of(
            &home,
            None,
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        )),
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        Clock::paused(env.start_time()),
    )
    .for_run_with(Duration::from_secs(30), Redaction::On)
}

/// Ada, moving her login.
fn ada() -> Origin {
    Origin {
        actor: Actor::Person {
            name: "ada".to_owned(),
        },
        surface: None,
        layer: Some(SettingsLayer::UserConfiguration),
        run_id: None,
    }
}

/// A Session with `cookie` as its one cookie's value and storage of one
/// origin.
fn session(cookie: &str) -> SessionExport {
    SessionExport {
        format_version: ProtocolVersion { major: 0, minor: 1 },
        cookies: vec![ExportedCookie {
            name: "sid".to_owned(),
            value: SessionValue::new(cookie),
            domain: ".shop.example".to_owned(),
            path: "/".to_owned(),
            expires: None,
            http_only: true,
            secure: true,
            same_site: None,
        }],
        origins: vec![ExportedOrigin {
            origin: "https://shop.example".to_owned(),
            local_storage: vec![StorageItem {
                name: "cart".to_owned(),
                value: SessionValue::new("3 items"),
            }],
            session_storage: vec![],
        }],
    }
}

fn to_passphrase(reference_text: &str) -> Recipient {
    Recipient::Passphrase {
        reference: reference(reference_text),
        work_factor: WORK,
    }
}

/// The kind, actor and details of each Audit entry in `env`'s home, as
/// recorded.
fn recorded(env: &TestEnv) -> Vec<Value> {
    let connection = Connection::open(env.home().join("audit-log.sqlite")).unwrap();
    let mut statement = connection
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| {
            let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
            json!([entry["kind"], entry["actor"], entry["details"]])
        })
        .collect()
}

/// The entry of `reference`'s use for `placeholder`.
fn used(reference: &str, placeholder: &str) -> Value {
    json!([
        "secret_reference_used",
        { "kind": "engine" },
        { "placeholder": placeholder, "reference": reference, "domain": null }
    ])
}

/// The entry of `kind`, Ada's, for a `bundle` of [`session`] under `key`.
fn moved(kind: &str, key: &Value, bundle: &[u8]) -> Value {
    json!([
        kind,
        { "kind": "person", "name": "ada" },
        {
            "key": key,
            "bundle_sha256": Digest::of(bundle).hex(),
            "cookies": 1,
            "origins": 1
        }
    ])
}

/// A Session exported to a passphrase imports with it on the engine it
/// moved to; each engine's Audit log holds its half, and no value.
/// Covers: story:4-68
#[test]
fn an_export_to_a_passphrase_and_its_import_are_each_recorded() {
    let from = TestEnv::new();
    let to = TestEnv::new();
    let mut canaries = Canaries::new(from.seed());
    let cookie = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let passphrase = canaries.plant(SecretKind::TypedPassword).value().to_owned();

    let (bundle, imported) = goodfetch_test_support::runtime().block_on(async {
        let exporting = secrets(&from, &[("EXPORT_PASSPHRASE", &passphrase)]);
        let importing = secrets(&to, &[("CI_PASSPHRASE", &passphrase)]);
        let bundle = export(
            session(&cookie),
            to_passphrase("env:EXPORT_PASSPHRASE"),
            &ada(),
            &exporting,
        )
        .await
        .unwrap();
        let imported = import(
            bundle.clone(),
            &reference("env:CI_PASSPHRASE"),
            LIMITS,
            &ada(),
            &importing,
        )
        .await
        .unwrap();
        (bundle, imported)
    });

    let (exported, received) = (recorded(&from), recorded(&to));
    let leaks = canaries
        .leaks(&[
            Output::Bytes {
                name: "bundle",
                bytes: &bundle,
            },
            Output::Bytes {
                name: "export's Audit log",
                bytes: json!(exported).to_string().as_bytes(),
            },
            Output::Bytes {
                name: "import's Audit log",
                bytes: json!(received).to_string().as_bytes(),
            },
        ])
        .unwrap();
    assert_eq!(
        (imported, exported, received, leaks),
        (
            session(&cookie),
            vec![
                used("env:EXPORT_PASSPHRASE", "session_export.passphrase"),
                moved(
                    "session_exported",
                    &json!({ "reference": "env:EXPORT_PASSPHRASE" }),
                    &bundle
                ),
            ],
            vec![
                used("env:CI_PASSPHRASE", "session.source"),
                moved(
                    "session_imported",
                    &json!({ "reference": "env:CI_PASSPHRASE" }),
                    &bundle
                ),
            ],
            vec![],
        )
    );
}

/// An export to a public key resolves no Secret, records the key, and
/// imports with the key's identity.
/// Covers: story:4-68
#[test]
fn an_export_to_a_public_key_is_recorded_with_the_key() {
    let env = TestEnv::new();
    let identity = x25519::Identity::generate();
    let public_key = identity.to_public().to_string();

    let (bundle, imported) = goodfetch_test_support::runtime().block_on(async {
        let secrets = secrets(&env, &[("IDENTITY", identity.to_string().expose_secret())]);
        let bundle = export(
            session("abc"),
            Recipient::PublicKey(public_key.parse::<PublicKey>().unwrap()),
            &ada(),
            &secrets,
        )
        .await
        .unwrap();
        let imported = import(
            bundle.clone(),
            &reference("env:IDENTITY"),
            LIMITS,
            &ada(),
            &secrets,
        )
        .await
        .unwrap();
        (bundle, imported)
    });

    assert_eq!(
        (imported, recorded(&env)),
        (
            session("abc"),
            vec![
                moved(
                    "session_exported",
                    &json!({ "public_key": public_key }),
                    &bundle
                ),
                used("env:IDENTITY", "session.source"),
                moved(
                    "session_imported",
                    &json!({ "reference": "env:IDENTITY" }),
                    &bundle
                ),
            ],
        )
    );
}

/// An import whose key opens nothing or gives no value imports nothing: the
/// Audit log holds the reference's use alone, if it gave a value.
/// Covers: story:4-68, error:session_export_open_failed, error:secret_unavailable
#[rstest]
#[case::a_wrong_passphrase(
    "env:WRONG_PASSPHRASE",
    "session_export_open_failed",
    vec![used("env:WRONG_PASSPHRASE", "session.source")]
)]
#[case::no_value("env:UNSET_PASSPHRASE", "secret_unavailable", vec![])]
fn an_import_that_fails_names_its_reference_and_records_no_import(
    #[case] key: &str,
    #[case] code: &str,
    #[case] entries: Vec<Value>,
) {
    let from = TestEnv::new();
    let to = TestEnv::new();

    let refused = goodfetch_test_support::runtime().block_on(async {
        let exporting = secrets(&from, &[("EXPORT_PASSPHRASE", "correct horse")]);
        let importing = secrets(&to, &[("WRONG_PASSPHRASE", "incorrect horse")]);
        let bundle = export(
            session("abc"),
            to_passphrase("env:EXPORT_PASSPHRASE"),
            &ada(),
            &exporting,
        )
        .await
        .unwrap();
        import(bundle, &reference(key), LIMITS, &ada(), &importing)
            .await
            .unwrap_err()
            .to_call_error()
    });

    let recorded = if entries.is_empty() {
        // No entry was ever written, so there may be no store to read.
        assert!(!to.home().join("audit-log.sqlite").exists());
        vec![]
    } else {
        recorded(&to)
    };
    assert_eq!(
        (
            refused.code(),
            refused.details().unwrap()["reference"].clone(),
            recorded
        ),
        (code, json!(key), entries)
    );
}

/// A move the Audit log refuses to record gives its caller nothing: no
/// bundle of an export, no Session of an import.
/// Covers: story:4-68
#[rstest]
#[case::export("session_exported", "the session export cannot be recorded")]
#[case::import("session_imported", "the session import cannot be recorded")]
fn a_move_that_cannot_be_recorded_gives_nothing(#[case] kind: &str, #[case] message: &str) {
    let env = TestEnv::new();

    let refused = goodfetch_test_support::runtime().block_on(async {
        let secrets = secrets(&env, &[("PASSPHRASE", "correct horse")]);
        let bundle = export(
            session("abc"),
            to_passphrase("env:PASSPHRASE"),
            &ada(),
            &secrets,
        )
        .await
        .unwrap();
        Connection::open(env.home().join("audit-log.sqlite"))
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER refused BEFORE INSERT ON entries
                 WHEN json_extract(NEW.entry, '$.kind') = '{kind}'
                 BEGIN SELECT RAISE(ABORT, 'refused'); END"
            ))
            .unwrap();
        if kind == "session_exported" {
            export(
                session("abc"),
                to_passphrase("env:PASSPHRASE"),
                &ada(),
                &secrets,
            )
            .await
            .unwrap_err()
        } else {
            import(
                bundle,
                &reference("env:PASSPHRASE"),
                LIMITS,
                &ada(),
                &secrets,
            )
            .await
            .unwrap_err()
        }
    });

    let answered = refused.to_call_error();
    assert!(
        answered.message().starts_with(message),
        "{}",
        answered.message()
    );
}
