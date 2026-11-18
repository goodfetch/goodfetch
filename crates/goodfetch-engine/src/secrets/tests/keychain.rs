//! The OS keychain provider against this OS's real keychain (TESTING.md,
//! Storage, network and OS services): `gnome-keyring`'s Secret Service on
//! Linux, the login keychain on macOS, the Credential Manager on Windows,
//! each item under a service named for the test alone and removed after
//! it. A host without the service skips this module (`keychain::`). The
//! test runs on a runtime of real time, since the keychain answers over a
//! real connection that a paused clock would time out.

use std::time::Duration;

use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use secrecy::{ExposeSecret as _, SecretString};

use super::{driver_with, for_the_key, reference};
use crate::secrets::{Keychain, SecretError};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Covers: story:4-50, error:secret_unavailable
#[test]
fn a_value_stored_in_the_os_keychain_resolves_through_keychain_and_once_removed_is_missing() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let value = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let keychain = Keychain::os();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let service = format!("goodfetch-test-{:016x}", env.seed());
    let planted = reference(&format!("keychain:{service}/portal"));
    let runtime = goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap();

    let (found, after) = runtime.block_on(async {
        keychain
            .store(
                &service,
                "portal",
                &SecretString::from(value.clone()),
                &owner,
            )
            .await
            .expect("this host has its OS keychain (TESTING.md: `ci/os-services.sh setup`)");
        let secrets = driver_with(&env, &[], keychain.clone()).for_run(Duration::from_secs(30));
        let found = secrets
            .resolve(&planted, &for_the_key(None, &[]))
            .await
            .map(|value| value.expose_secret().to_owned());
        keychain.remove(&service, "portal", &owner).await.unwrap();
        // A Run reuses the value it resolved (ADR 0014's note of
        // 2026-10-28), so the next Run asks the keychain again.
        let next = driver_with(&env, &[], keychain.clone()).for_run(Duration::from_secs(30));
        let after = match next.resolve(&planted, &for_the_key(None, &[])).await {
            Err(SecretError::Unavailable { failure, .. }) => Some(failure),
            _ => None,
        };
        (found, after)
    });

    assert_eq!(
        (
            found.ok().as_deref() == Some(value.as_str()),
            after,
            canaries.leaks(&[Output::Path(env.home())]).unwrap()
        ),
        (true, Some(SecretFailure::Missing), vec![])
    );
}
