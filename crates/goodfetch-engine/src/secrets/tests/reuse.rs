//! Reuse and retry within a Run (ADR 0014's note of 2026-10-28, accepted by
//! the owner): a value resolved for a reference and placeholder is reused
//! for the rest of the Run, its domain binding checked again at each use;
//! concurrent resolutions of one reference make one call; a provider that
//! is briefly busy is asked again with exponential backoff and full jitter,
//! after its `Retry-After` when it gave one, within `timeout.secret`; and a
//! reference listed in `secrets.resolve_each_use` is resolved at each use.
//! On the paused `Clock`.

use std::time::Duration;

use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_test_support::TestEnv;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::{Counting, driver, for_the_key, reference};
use crate::secrets::{
    ConfigProblem, ProviderFailure, Resolving, RunSecrets, SecretError, SecretProvider,
};
use crate::settings::{Layers, Values, resolve};

const VAULT: &str = "vault:secret/shop#password";

/// A provider that answers `times` times that it is busy, after
/// `retry_after` when given, then the reference's own text, and keeps
/// each moment it was asked at.
struct Busy {
    times: u32,
    retry_after: Option<Duration>,
    asked: tokio::sync::Mutex<Vec<tokio::time::Instant>>,
}

impl Busy {
    fn new(times: u32, retry_after: Option<Duration>) -> Self {
        Self {
            times,
            retry_after,
            asked: tokio::sync::Mutex::default(),
        }
    }
}

impl SecretProvider for Busy {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let mut asked = self.asked.lock().await;
            #[expect(
                clippy::disallowed_methods,
                reason = "the test reads the paused runtime's clock, which the driver's Clock follows (ADR 0059)"
            )]
            asked.push(tokio::time::Instant::now());
            if asked.len() <= self.times as usize {
                return Err(ProviderFailure::Busy {
                    fix: "The server's rate limit refused the read; try again later.".to_owned(),
                    retry_after: self.retry_after,
                });
            }
            Ok(SecretString::from(format!("value of {reference}")))
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// A provider that answers after a second, counting its calls.
#[derive(Default)]
struct Slow(Counting);

impl SecretProvider for Slow {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            self.0.resolve(reference).await
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// `secrets` resolving `text` for the run's key, bound to `domains`, as
/// asked from `domain`.
async fn for_domain(
    secrets: &RunSecrets,
    text: &str,
    domain: &str,
    domains: &[String],
) -> Result<SecretString, SecretError> {
    secrets
        .resolve(&reference(text), &for_the_key(Some(domain), domains))
        .await
}

/// A value resolved once is reused at the reference's next uses in the
/// Run, and each use still checks the domain binding: the provider is asked
/// once, and a domain the binding does not allow is refused.
/// Covers: story:4-49, error:secret_domain_not_allowed
#[test]
fn a_value_resolved_once_is_reused_and_its_domain_binding_checked_at_each_use() {
    let env = TestEnv::new();
    let counting = std::sync::Arc::new(Counting::default());
    let domains = ["shop.example".to_owned()];

    let (answers, asked) = goodfetch_test_support::runtime().block_on(async {
        let secrets = driver(&env, &[])
            .with(
                Provider::Vault,
                std::sync::Arc::<Counting>::clone(&counting),
            )
            .for_run(Duration::from_secs(30));
        let mut answers = Vec::new();
        for domain in ["shop.example", "shop.example", "elsewhere.example"] {
            answers.push(
                for_domain(&secrets, VAULT, domain, &domains)
                    .await
                    .map(|value| value.expose_secret().to_owned())
                    .map_err(|error| matches!(error, SecretError::DomainNotAllowed { .. })),
            );
        }
        (answers, counting.asked.lock().await.len())
    });

    assert_eq!(
        (answers, asked),
        (
            vec![
                Ok(format!("value of {VAULT}")),
                Ok(format!("value of {VAULT}")),
                Err(true)
            ],
            1
        )
    );
}

/// Two resolutions of one reference at once make one call to its provider.
/// Covers: story:4-49
#[test]
fn concurrent_resolutions_of_one_reference_make_one_call() {
    let env = TestEnv::new();
    let slow = std::sync::Arc::new(Slow::default());

    let asked = goodfetch_test_support::runtime().block_on(async {
        let secrets = driver(&env, &[])
            .with(Provider::Vault, std::sync::Arc::<Slow>::clone(&slow))
            .for_run(Duration::from_secs(30));
        let purpose = for_the_key(None, &[]);
        let reference = reference(VAULT);
        let (first, second) = tokio::join!(
            secrets.resolve(&reference, &purpose),
            secrets.resolve(&reference, &purpose)
        );
        first.unwrap();
        second.unwrap();
        slow.0.asked.lock().await.len()
    });

    assert_eq!(asked, 1);
}

/// What resolving through a provider busy `times` times answers, and how
/// long each later ask came after the one before it.
fn through_busy(
    times: u32,
    retry_after: Option<Duration>,
) -> (Result<String, SecretFailure>, Vec<Duration>) {
    let env = TestEnv::new();
    let provider = std::sync::Arc::new(Busy::new(times, retry_after));
    goodfetch_test_support::runtime().block_on(async {
        let secrets = driver(&env, &[])
            .with(Provider::Vault, std::sync::Arc::<Busy>::clone(&provider))
            .for_run(Duration::from_secs(30));
        let answer = secrets
            .resolve(&reference(VAULT), &for_the_key(None, &[]))
            .await
            .map(|value| value.expose_secret().to_owned())
            .map_err(|error| match error {
                SecretError::Unavailable { failure, .. } => failure,
                other => panic!("expected `secret_unavailable`, found {other:?}"),
            });
        let asked = provider.asked.lock().await;
        let gaps = asked.windows(2).map(|pair| pair[1] - pair[0]).collect();
        (answer, gaps)
    })
}

/// A provider briefly busy, as a rate limit's 429 or a 503, is asked again
/// within `timeout.secret`, each wait drawn with full jitter under a cap
/// that doubles from 200 ms: never more than the cap so far.
/// Covers: story:4-60, setting:timeout.secret
#[test]
fn a_busy_provider_is_asked_again_with_backoff_until_it_answers() {
    let (answer, gaps) = through_busy(3, None);

    assert_eq!(answer, Ok(format!("value of {VAULT}")));
    assert_eq!(gaps.len(), 3);
    assert!(
        gaps[0] <= Duration::from_millis(200)
            && gaps[1] <= Duration::from_millis(400)
            && gaps[2] <= Duration::from_millis(800),
        "{gaps:?}"
    );
}

/// A busy provider's `Retry-After` is waited for before it is asked again.
/// Covers: story:4-60, setting:timeout.secret
#[test]
fn a_busy_providers_retry_after_is_waited_for() {
    let (answer, gaps) = through_busy(1, Some(Duration::from_secs(10)));

    assert_eq!(
        (answer, gaps),
        (
            Ok(format!("value of {VAULT}")),
            vec![Duration::from_secs(10)]
        )
    );
}

/// A provider still busy when `timeout.secret` would pass before its next
/// ask fails the resolution as `provider_failed`, with its fix.
/// Covers: story:4-60, error:secret_unavailable, setting:timeout.secret
#[test]
fn a_provider_still_busy_at_timeout_secret_fails_the_resolution() {
    let (answer, _) = through_busy(1, Some(Duration::from_secs(60)));

    assert_eq!(answer, Err(SecretFailure::ProviderFailed));
}

/// A reference `secrets.resolve_each_use` lists, such as a rotating
/// token's, is resolved again at each use.
/// Covers: story:4-49, setting:secrets.resolve_each_use
#[test]
fn a_reference_listed_to_resolve_at_each_use_asks_its_provider_each_time() {
    let env = TestEnv::new();
    let counting = std::sync::Arc::new(Counting::default());
    let settings = resolve(
        &Layers {
            user_configuration: Values::read(
                goodfetch_protocol::settings::SettingsLayer::UserConfiguration,
                json!({ "secrets": { "resolve_each_use": [VAULT] } })
                    .as_object()
                    .unwrap(),
                goodfetch_protocol::catalog::Product::Collie,
            )
            .unwrap(),
            ..Layers::default()
        },
        goodfetch_protocol::catalog::Product::Collie,
        goodfetch_protocol::catalog::Product::Collie,
    )
    .unwrap();

    let asked = goodfetch_test_support::runtime().block_on(async {
        let secrets = driver(&env, &[])
            .with(
                Provider::Vault,
                std::sync::Arc::<Counting>::clone(&counting),
            )
            .for_run_under(
                &settings,
                Duration::from_secs(30),
                crate::mask::Redaction::On,
            );
        for _ in 0..3 {
            secrets
                .resolve(&reference(VAULT), &for_the_key(None, &[]))
                .await
                .unwrap();
        }
        counting.asked.lock().await.len()
    });

    assert_eq!(asked, 3);
}
