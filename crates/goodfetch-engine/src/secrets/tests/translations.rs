//! The OS keychain's and the encrypted local file's translation tables
//! (ADR 0127): one named case per failure each party is known to answer
//! and one for a failure it is not, and a party's own text, planted as a
//! Secret canary, in no log line and no formatted failure. Then the
//! keychain provider through the driver over keyring-core's mock store,
//! and on a host with no keychain.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use age::DecryptError;
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, logged};
use keyring_core::{CredentialStore, Error};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{for_the_key, reference};
use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::home::Home;
use crate::secrets::{
    Carried, Keychain, ProviderFailure, SecretError, Secrets, keychain, secretfile,
};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Which [`ProviderFailure`] a failure is, without its text.
#[derive(Debug, PartialEq, Eq)]
enum Meaning {
    Missing,
    NotConfigured,
    Refused,
    Unrecognized,
}

fn meaning(failure: &ProviderFailure) -> Meaning {
    match failure {
        ProviderFailure::Missing => Meaning::Missing,
        ProviderFailure::NotConfigured(_) => Meaning::NotConfigured,
        ProviderFailure::Refused { .. } => Meaning::Refused,
        ProviderFailure::Unrecognized(_) => Meaning::Unrecognized,
        ProviderFailure::ResolverFailed => panic!("no provider here answers for the caller"),
        ProviderFailure::Busy { .. } => panic!("no OS provider here answers that it is busy"),
    }
}

fn platform(text: &str) -> keyring_core::error::PlatformError {
    Box::new(io::Error::other(text.to_owned()))
}

/// Covers: story:4-50
#[rstest]
#[case::no_entry(Error::NoEntry, Meaning::Missing)]
#[case::no_storage_access(Error::NoStorageAccess(platform("locked")), Meaning::Refused)]
#[case::bad_encoding(Error::BadEncoding(vec![0x80]), Meaning::Refused)]
#[case::ambiguous(Error::Ambiguous(Vec::new()), Meaning::Refused)]
#[case::too_long(Error::TooLong("service".to_owned(), 255), Meaning::Refused)]
#[case::invalid(Error::Invalid("service".to_owned(), "empty".to_owned()), Meaning::Refused)]
#[case::an_unknown_failure(Error::PlatformFailure(platform("-25308")), Meaning::Unrecognized)]
fn the_keychains_failures_translate_each_to_its_meaning(
    #[case] error: Error,
    #[case] expected: Meaning,
) {
    assert_eq!(meaning(&keychain::translate(error)), expected);
}

/// Covers: story:4-50
#[cfg(target_os = "linux")]
#[rstest]
#[case::service_unknown(
    Some("org.freedesktop.DBus.Error.ServiceUnknown"),
    false,
    Meaning::NotConfigured
)]
#[case::name_has_no_owner(
    Some("org.freedesktop.DBus.Error.NameHasNoOwner"),
    false,
    Meaning::NotConfigured
)]
#[case::bus_unreachable(None, true, Meaning::NotConfigured)]
#[case::is_locked(Some("org.freedesktop.Secret.Error.IsLocked"), false, Meaning::Refused)]
#[case::access_denied(
    Some("org.freedesktop.DBus.Error.AccessDenied"),
    false,
    Meaning::Refused
)]
#[case::no_such_object(
    Some("org.freedesktop.Secret.Error.NoSuchObject"),
    false,
    Meaning::Missing
)]
#[case::unknown_object(
    Some("org.freedesktop.DBus.Error.UnknownObject"),
    false,
    Meaning::Missing
)]
#[case::item_gone(
    Some("org.freedesktop.DBus.Error.UnknownMethod"),
    false,
    Meaning::Missing
)]
#[case::an_unknown_named_failure(
    Some("org.freedesktop.Secret.Error.NoSession"),
    false,
    Meaning::Unrecognized
)]
#[case::an_unknown_unnamed_failure(None, false, Meaning::Unrecognized)]
fn the_secret_services_failures_translate_each_to_its_meaning(
    #[case] name: Option<&str>,
    #[case] unreachable: bool,
    #[case] expected: Meaning,
) {
    let failure = keychain::secret_service::translate_named(name, unreachable, String::new);

    assert_eq!(meaning(&failure), expected);
}

/// Covers: story:4-50
#[rstest]
#[case::no_matching_keys(DecryptError::NoMatchingKeys, Meaning::Refused)]
#[case::unknown_format(DecryptError::UnknownFormat, Meaning::Refused)]
#[case::invalid_header(DecryptError::InvalidHeader, Meaning::Refused)]
#[case::invalid_mac(DecryptError::InvalidMac, Meaning::Refused)]
#[case::decryption_failed(DecryptError::DecryptionFailed, Meaning::Refused)]
#[case::key_decryption_failed(DecryptError::KeyDecryptionFailed, Meaning::Refused)]
#[case::excessive_work(DecryptError::ExcessiveWork { required: 22, target: 18 }, Meaning::Refused)]
#[case::cut_short(DecryptError::Io(io::ErrorKind::UnexpectedEof.into()), Meaning::Refused)]
#[case::permission_denied(DecryptError::Io(io::ErrorKind::PermissionDenied.into()), Meaning::Refused)]
#[case::an_unknown_failure(
    DecryptError::Io(io::Error::other("disk on fire")),
    Meaning::Unrecognized
)]
fn the_encrypted_local_files_failures_translate_each_to_its_meaning(
    #[case] error: DecryptError,
    #[case] expected: Meaning,
) {
    let failure = secretfile::translate(
        error,
        Path::new("shop-key.age"),
        Path::new("identity.txt"),
        "age1recipient",
    );

    assert_eq!(meaning(&failure), expected);
}

/// Covers: story:4-50
#[test]
fn a_partys_own_text_is_in_no_log_line_and_no_formatted_failure() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let texts: Vec<String> = (0..4)
        .map(|_| {
            canaries
                .plant(SecretKind::SinkCredential)
                .value()
                .to_owned()
        })
        .collect();

    let (failures, logs) = logged(|| {
        vec![
            keychain::translate(Error::PlatformFailure(platform(&texts[0]))),
            keychain::translate(Error::BadEncoding(texts[1].clone().into_bytes())),
            secretfile::translate(
                DecryptError::Io(io::Error::other(texts[2].clone())),
                Path::new("shop-key.age"),
                Path::new("identity.txt"),
                "age1recipient",
            ),
            secretfile::translate_io(&io::Error::other(texts[3].clone()), Path::new("x.age")),
            #[cfg(target_os = "linux")]
            keychain::secret_service::translate(zbus::Error::Failure(texts[3].clone())),
        ]
    });

    let formatted = format!("{failures:?}");
    assert_eq!(
        (
            failures.iter().map(meaning).collect::<Vec<_>>(),
            canaries
                .leaks(&[
                    Output::Bytes {
                        name: "logs",
                        bytes: &logs,
                    },
                    Output::Bytes {
                        name: "formatted",
                        bytes: formatted.as_bytes(),
                    },
                ])
                .unwrap()
        ),
        (
            vec![
                Meaning::Unrecognized,
                Meaning::Refused,
                Meaning::Unrecognized,
                Meaning::Unrecognized,
                #[cfg(target_os = "linux")]
                Meaning::Unrecognized,
            ],
            vec![]
        )
    );
}

/// The driver over `keychain`, with the test's home.
fn over(env: &TestEnv, keychain: Keychain) -> crate::secrets::RunSecrets {
    let home = Home::new(env.home().to_path_buf());
    goodfetch_test_support::runtime().block_on(async {
        Secrets::new(
            Carried {
                environment: std::collections::BTreeMap::new(),
                keychain,
                home: home.clone(),
                entropy: crate::entropy::Entropy::seeded(0),
            },
            Arc::new(AuditLog::of(&home, None)),
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
            Clock::paused(env.start_time()),
        )
        .for_run(Duration::from_secs(30))
    })
}

fn through(
    secrets: &crate::secrets::RunSecrets,
    text: &str,
) -> Result<secrecy::SecretString, SecretError> {
    goodfetch_test_support::runtime()
        .block_on(secrets.resolve(&reference(text), &for_the_key(None, &[])))
}

/// Covers: story:4-50
#[test]
fn a_keychain_item_resolves_through_keychain_and_a_missing_one_is_missing() {
    let env = TestEnv::new();
    let store: Arc<CredentialStore> = keyring_core::mock::Store::new().unwrap();
    store
        .build("shop", "portal", None)
        .unwrap()
        .set_password("s3cr3t-value")
        .unwrap();
    let secrets = over(&env, Keychain::of(store));

    let found =
        through(&secrets, "keychain:shop/portal").map(|value| value.expose_secret().to_owned());
    let missing = match through(&secrets, "keychain:shop/admin") {
        Err(SecretError::Unavailable { failure, .. }) => Some(failure),
        _ => None,
    };

    assert_eq!(
        (found.ok(), missing),
        (
            Some("s3cr3t-value".to_owned()),
            Some(SecretFailure::Missing)
        )
    );
}

/// Covers: story:4-50, error:secret_unavailable
#[test]
fn a_keychain_reference_on_a_host_with_no_keychain_is_not_configured_naming_secretfile() {
    let env = TestEnv::new();
    let secrets = over(&env, Keychain::absent());

    let (failure, fix) = match through(&secrets, "keychain:shop/portal") {
        Err(SecretError::Unavailable { failure, fix, .. }) => (failure, fix),
        other => panic!("expected `secret_unavailable`, found {:?}", other.err()),
    };

    assert_eq!(
        (failure, fix.contains("secretfile:")),
        (SecretFailure::NotConfigured, true),
        "{fix}"
    );
}
