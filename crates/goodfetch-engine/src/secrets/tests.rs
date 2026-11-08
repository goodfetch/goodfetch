//! The Secret-provider driver through its interface (spec #564's `secrets`
//! seam), on the paused `Clock` and a real Audit log in the test's folder:
//! a value resolves only when used, through the `env:` provider or a
//! provider the test fills the primitives of, is held for the run and
//! recorded in the Audit log without its value; each failure is
//! `secret_unavailable`'s variant, naming the placeholder and the reference.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059) and read the Audit log's file"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_protocol::secrets::{SecretFailure, SecretInputRefusedReason};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use rusqlite::Connection;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{
    Carried, ConfigProblem, Keychain, ProviderFailure, Purpose, Resolving, RunSecrets, SecretError,
    SecretProvider, Secrets,
};
use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::home::Home;
use crate::tasks::{BlockingPolicy, TaskOwner};

const KEY: &str = "run_state.key_reference";

/// A provider that answers every reference with its own text, and keeps
/// each reference it was asked for.
#[derive(Default)]
struct Counting {
    asked: tokio::sync::Mutex<Vec<String>>,
}

impl SecretProvider for Counting {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            self.asked.lock().await.push(reference.to_string());
            Ok(SecretString::from(format!("value of {reference}")))
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// A provider that never answers.
struct Hanging;

impl SecretProvider for Hanging {
    fn resolve<'a>(&'a self, _: &'a SecretReference) -> Resolving<'a> {
        Box::pin(std::future::pending::<Result<SecretString, ProviderFailure>>())
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

mod files;
mod keychain;
mod racing;
mod reuse;
mod translations;

fn driver(env: &TestEnv, variables: &[(&str, &str)]) -> Secrets {
    driver_with(env, variables, Keychain::absent())
}

/// The driver over `variables` and `keychain`, with the test's home.
fn driver_with(env: &TestEnv, variables: &[(&str, &str)], keychain: Keychain) -> Secrets {
    let environment: BTreeMap<OsString, OsString> = variables
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();
    Secrets::new(
        Carried {
            environment,
            keychain,
            home: Home::new(env.home().to_path_buf()),
            entropy: crate::entropy::Entropy::seeded(env.seed()),
        },
        Arc::new(AuditLog::of(&Home::new(env.home().to_path_buf()), None)),
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        Clock::paused(env.start_time()),
    )
}

fn for_the_key<'a>(domain: Option<&'a str>, domains: &'a [String]) -> Purpose<'a> {
    Purpose {
        placeholder: KEY,
        domain,
        domains,
        layer: Some(SettingsLayer::UserConfiguration),
        run_id: None,
    }
}

fn reference(text: &str) -> SecretReference {
    SecretReference::parse(text).unwrap()
}

/// Resolves `text` for the run's key through `secrets`.
fn resolved(secrets: &RunSecrets, text: &str) -> Result<SecretString, SecretError> {
    goodfetch_test_support::runtime()
        .block_on(secrets.resolve(&reference(text), &for_the_key(None, &[])))
}

/// The entries of the Audit log in `env`'s home, as recorded.
fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let connection = Connection::open(env.home().join("audit-log.sqlite")).unwrap();
    let mut statement = connection
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| serde_json::from_str(&entry.unwrap()).unwrap())
        .collect()
}

/// Covers: story:4-49
#[test]
fn a_reference_the_run_never_uses_reaches_no_provider() {
    let env = TestEnv::new();
    let counting = Arc::new(Counting::default());
    let secrets = goodfetch_test_support::runtime().block_on(async {
        driver(&env, &[])
            .with(
                Provider::Caller,
                Arc::clone(&counting) as Arc<dyn SecretProvider>,
            )
            .for_run(Duration::from_secs(30))
    });
    resolved(&secrets, "caller:used").unwrap();

    assert_eq!(
        goodfetch_test_support::runtime().block_on(async { counting.asked.lock().await.clone() }),
        vec!["caller:used".to_owned()]
    );
}

/// Covers: story:4-49
#[test]
fn a_set_variable_resolves_through_env_and_is_held_for_the_run() {
    let env = TestEnv::new();
    let secrets = goodfetch_test_support::runtime().block_on(async {
        driver(&env, &[("SHOP_KEY", "s3cr3t-value")]).for_run(Duration::from_secs(30))
    });

    let value = resolved(&secrets, "env:SHOP_KEY").unwrap();

    let held = goodfetch_test_support::runtime().block_on(secrets.held());
    assert_eq!(
        (
            value.expose_secret().to_owned(),
            held.iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect::<Vec<_>>()
        ),
        (
            "s3cr3t-value".to_owned(),
            vec![(KEY.to_owned(), "s3cr3t-value".to_owned())]
        )
    );
}

/// A value resolved again for the same placeholder, as each Session's open
/// resolves the proxy password, is held once: 100 resolutions of one
/// value leave one held value, and a value resolved after it comes last.
/// Covers: story:4-49
#[test]
fn a_value_resolved_again_for_its_placeholder_is_held_once() {
    let env = TestEnv::new();
    let secrets = goodfetch_test_support::runtime().block_on(async {
        driver(
            &env,
            &[("SHOP_KEY", "s3cr3t-value"), ("OTHER_KEY", "other-value")],
        )
        .for_run(Duration::from_secs(30))
    });

    for _ in 0..100 {
        resolved(&secrets, "env:SHOP_KEY").unwrap();
    }
    resolved(&secrets, "env:OTHER_KEY").unwrap();
    resolved(&secrets, "env:SHOP_KEY").unwrap();

    let held = goodfetch_test_support::runtime().block_on(secrets.held());
    assert_eq!(
        held.iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect::<Vec<_>>(),
        vec![
            (KEY.to_owned(), "other-value".to_owned()),
            (KEY.to_owned(), "s3cr3t-value".to_owned())
        ]
    );
}

/// Covers: story:4-60, error:secret_unavailable
#[rstest]
#[case::an_unset_variable("env:SHOP_KEY", &[], SecretFailure::Missing)]
#[case::an_empty_variable("env:SHOP_KEY", &[("SHOP_KEY", "")], SecretFailure::Missing)]
#[case::a_provider_this_engine_lacks("op://shop/portal/password", &[], SecretFailure::NotConfigured)]
fn a_reference_that_gives_no_value_is_unavailable_naming_its_placeholder_and_failure(
    #[case] text: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: SecretFailure,
) {
    let env = TestEnv::new();
    let secrets = goodfetch_test_support::runtime()
        .block_on(async { driver(&env, variables).for_run(Duration::from_secs(30)) });

    let error = resolved(&secrets, text).unwrap_err();

    let SecretError::Unavailable {
        placeholder,
        reference,
        failure,
        ..
    } = error
    else {
        panic!("expected `secret_unavailable`, found {error:?}");
    };
    assert_eq!(
        (placeholder.as_str(), reference.as_str(), failure),
        (KEY, text, expected)
    );
}

/// Covers: story:4-60, error:secret_unavailable, setting:timeout.secret
#[test]
fn a_provider_that_hangs_ends_at_its_timeout_as_timed_out() {
    let env = TestEnv::new();
    let failure = goodfetch_test_support::runtime().block_on(async {
        let secrets = driver(&env, &[])
            .with(Provider::Vault, Arc::new(Hanging))
            .for_run(Duration::from_secs(30));
        let started = tokio::time::Instant::now();
        let error = secrets
            .resolve(
                &reference("vault:secret/shop#password"),
                &for_the_key(None, &[]),
            )
            .await
            .unwrap_err();
        let SecretError::Unavailable { failure, .. } = error else {
            panic!("expected `secret_unavailable`, found {error:?}");
        };
        (failure, started.elapsed())
    });

    assert_eq!(failure, (SecretFailure::TimedOut, Duration::from_secs(30)));
}

/// Covers: story:4-62
#[test]
fn each_resolution_is_recorded_with_its_placeholder_reference_and_domain_never_its_value() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let value = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let secrets = goodfetch_test_support::runtime().block_on(async {
        driver(&env, &[("SHOP_PASSWORD", &value)]).for_run(Duration::from_secs(30))
    });

    goodfetch_test_support::runtime()
        .block_on(secrets.resolve(
            &reference("env:SHOP_PASSWORD"),
            &for_the_key(Some("shop.example"), &[]),
        ))
        .unwrap();

    let entries = audit_entries(&env);
    assert_eq!(
        (
            entries
                .iter()
                .map(|entry| (entry["kind"].clone(), entry["details"].clone()))
                .collect::<Vec<_>>(),
            canaries.leaks(&[Output::Path(env.home())]).unwrap()
        ),
        (
            vec![(
                json!("secret_reference_used"),
                json!({
                    "placeholder": KEY,
                    "reference": "env:SHOP_PASSWORD",
                    "domain": "shop.example",
                })
            )],
            vec![]
        )
    );
}

/// Covers: error:secret_domain_not_allowed
#[rstest]
#[case::a_bound_domain("shop.example", None)]
#[case::a_subdomain_of_a_bound_domain("login.Shop.example", None)]
#[case::another_domain("evil.example", Some("evil.example"))]
#[case::a_domain_that_only_ends_alike("evilshop.example", Some("evilshop.example"))]
fn a_value_goes_only_to_a_domain_its_binding_allows(
    #[case] domain: &str,
    #[case] refused: Option<&str>,
) {
    let env = TestEnv::new();
    let secrets = goodfetch_test_support::runtime().block_on(async {
        driver(&env, &[("SHOP_PASSWORD", "s3cr3t-value")]).for_run(Duration::from_secs(30))
    });
    let domains = vec!["shop.example".to_owned()];

    let answer = goodfetch_test_support::runtime().block_on(secrets.resolve(
        &reference("env:SHOP_PASSWORD"),
        &for_the_key(Some(domain), &domains),
    ));

    let refused_for = match answer {
        Ok(_) => None,
        Err(SecretError::DomainNotAllowed { domain, .. }) => Some(domain),
        Err(other) => panic!("expected a value or `secret_domain_not_allowed`: {other:?}"),
    };
    assert_eq!(refused_for.as_deref(), refused);
}

/// Where the host has an OS keychain, the engine makes its own key there,
/// as `keychain:goodfetch/run-state-key`, once: 32 random bytes in Base64,
/// which resolve through `keychain:`; made again, the first key stays, and
/// no encrypted local file is made. keyring-core's mock store stands in
/// for the keychain, as its reads and writes are the real ones'.
/// Covers: story:4-64, setting:run_state.key_reference
#[test]
fn with_an_os_keychain_the_engines_own_key_is_made_once_there() {
    let env = TestEnv::new();
    let keychain = Keychain::of(keyring_core::mock::Store::new().unwrap());
    let key = reference("keychain:goodfetch/run-state-key");
    let runtime = goodfetch_test_support::runtime();
    let secrets = runtime.block_on(async { driver_with(&env, &[], keychain) });
    let read = || {
        let resolving = secrets.for_run(Duration::from_secs(30));
        runtime
            .block_on(resolving.resolve(&key, &for_the_key(None, &[])))
            .unwrap()
            .expose_secret()
            .to_owned()
    };

    runtime.block_on(secrets.make_key(&key)).unwrap();
    let first = read();
    runtime.block_on(secrets.make_key(&key)).unwrap();
    let second = read();

    assert_eq!(
        (
            data_encoding::BASE64
                .decode(first.as_bytes())
                .map(|bytes| bytes.len()),
            first == second,
            env.home().join("secrets").exists()
        ),
        (Ok(32), true, false)
    );
}

/// keyring-core's mock store, counting each item a caller reaches through
/// it: a keychain that shows whether anything asked it.
struct Reaching {
    store: Arc<keyring_core::mock::Store>,
    #[expect(
        clippy::disallowed_types,
        reason = "a test's counter, which no loom model runs (ADR 0060)"
    )]
    reached: std::sync::atomic::AtomicUsize,
}

impl keyring_core::api::CredentialStoreApi for Reaching {
    fn vendor(&self) -> String {
        self.store.vendor()
    }

    fn id(&self) -> String {
        self.store.id()
    }

    fn build(
        &self,
        service: &str,
        user: &str,
        modifiers: Option<&std::collections::HashMap<&str, &str>>,
    ) -> keyring_core::Result<keyring_core::Entry> {
        self.reached
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.store.build(service, user, modifiers)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl std::fmt::Debug for Reaching {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Reaching").finish_non_exhaustive()
    }
}

/// Building the driver over the OS keychain, as the engine does at its
/// start, and handing a Run what it resolves through reach none of the
/// keychain's items: the keychain is asked only once a reference needs it
/// (#2204 moves its probe to the first data key made or unwrapped).
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "a test's counter, which no loom model runs (ADR 0060)"
)]
fn the_driver_asks_the_keychain_nothing_until_a_reference_needs_it() {
    let env = TestEnv::new();
    let store = Arc::new(Reaching {
        store: keyring_core::mock::Store::new().unwrap(),
        reached: std::sync::atomic::AtomicUsize::new(0),
    });
    let keychain = Keychain::of(Arc::clone(&store) as Arc<keyring_core::CredentialStore>);
    let runtime = goodfetch_test_support::runtime();
    let secrets = runtime.block_on(async { driver_with(&env, &[], keychain) });
    let resolving = secrets.for_run(Duration::from_secs(30));
    let built = store.reached.load(std::sync::atomic::Ordering::SeqCst);

    let _ = runtime.block_on(resolving.resolve(
        &reference("keychain:goodfetch-test/portal"),
        &for_the_key(None, &[]),
    ));

    assert_eq!(
        (
            built,
            store.reached.load(std::sync::atomic::Ordering::SeqCst) > 0
        ),
        (0, true)
    );
}

/// A host the Run reached through an exception is found whatever the
/// spelling a later caller gives it, so no client certificate or Secret
/// goes to it: its IDNA form, its case and its trailing dot, as TLS keys
/// it when it records the host.
#[rstest]
#[case::as_recorded("xn--bcher-kva.test")]
#[case::unicode("bücher.test")]
#[case::upper_case_fully_qualified("BÜCHER.Test.")]
fn a_host_reached_through_an_exception_is_found_whatever_its_spelling(#[case] asked: &str) {
    let reached = super::ExceptionHosts::default();
    let key = crate::known_certificates::KeyHash::of_public_key(b"key");

    let found = goodfetch_test_support::runtime().block_on(async {
        reached
            .add(
                "xn--bcher-kva.test",
                key,
                crate::known_certificates::ExceptionKind::TrustOnFirstUse,
            )
            .await;
        reached.reached(asked).await
    });

    assert_eq!(found, Some(key));
}

/// A proxy password goes to its proxy's host, and asks nothing else before it
/// resolves: this refusal alone keeps it from a host the Run reached through
/// an exception (ADR 0042). A host the Run reached otherwise still gets it.
#[test]
fn no_secret_resolves_for_a_host_the_run_reached_through_an_exception() {
    let env = TestEnv::new();
    let password = reference("env:PROXY_PASSWORD");
    let purpose = |domain| Purpose {
        placeholder: "proxy.password",
        domain: Some(domain),
        domains: &[],
        layer: Some(SettingsLayer::UserConfiguration),
        run_id: None,
    };

    let (to_excepted, to_verified) = goodfetch_test_support::runtime().block_on(async {
        let secrets =
            driver(&env, &[("PROXY_PASSWORD", "s3cr3t-value")]).for_run(Duration::from_secs(30));
        secrets
            .exception_hosts()
            .add(
                "expired.test",
                crate::known_certificates::KeyHash::of_public_key(b"key"),
                crate::known_certificates::ExceptionKind::TrustOnFirstUse,
            )
            .await;
        (
            secrets.resolve(&password, &purpose("expired.test")).await,
            secrets.resolve(&password, &purpose("verified.test")).await,
        )
    });

    let Err(SecretError::InputRefused {
        placeholder,
        domain,
        reason: SecretInputRefusedReason::InvalidCertificateException,
        ..
    }) = to_excepted
    else {
        panic!("no Secret resolves for a host reached through an exception: {to_excepted:?}");
    };
    assert_eq!(
        (placeholder.as_str(), domain.as_str()),
        ("proxy.password", "expired.test")
    );
    assert_eq!(to_verified.unwrap().expose_secret(), "s3cr3t-value");
}
