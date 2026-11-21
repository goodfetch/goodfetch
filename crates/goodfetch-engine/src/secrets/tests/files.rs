//! The encrypted local file through the driver (spec #564): a value planted
//! in an age file of the test's Goodfetch home, encrypted to the identity
//! beside it as `age -r` encrypts it, resolves through `secretfile:`, held
//! for the run and found in no file of the home; an identity file other
//! users may read is refused with the fix that closes it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use age::x25519;
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use secrecy::ExposeSecret as _;

use super::{driver, resolved};
use crate::mask::Redaction;
use crate::secrets::SecretError;

/// The folder of the encrypted local files in `env`'s home.
fn folder(env: &TestEnv) -> PathBuf {
    env.home().join("secrets")
}

/// Makes the identity file, mode `0600` where modes exist, as `age-keygen
/// -o` writes it, and answers its identity.
fn identity(env: &TestEnv) -> x25519::Identity {
    let identity = x25519::Identity::generate();
    std::fs::create_dir_all(folder(env)).unwrap();
    let path = folder(env).join("identity.txt");
    std::fs::write(
        &path,
        format!(
            "# created: 2026-10-22T00:00:00Z\n# public key: {}\n{}\n",
            identity.to_public(),
            identity.to_string().expose_secret()
        ),
    )
    .unwrap();
    owner_only(&path);
    identity
}

#[cfg(unix)]
fn owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(not(unix))]
fn owner_only(_: &Path) {}

/// Writes `value` to the encrypted local file `name`, encrypted to
/// `recipient`, armored or not, as `age -r` (`-a`) does.
fn plant(env: &TestEnv, name: &str, value: &str, recipient: &x25519::Recipient, armored: bool) {
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient)).unwrap();
    let mut encrypted = Vec::new();
    {
        let format = if armored {
            age::armor::Format::AsciiArmor
        } else {
            age::armor::Format::Binary
        };
        let armor = age::armor::ArmoredWriter::wrap_output(&mut encrypted, format).unwrap();
        let mut writer = encryptor.wrap_output(armor).unwrap();
        writer.write_all(value.as_bytes()).unwrap();
        writer.finish().unwrap().finish().unwrap();
    }
    std::fs::write(folder(env).join(format!("{name}.age")), encrypted).unwrap();
}

/// The failure and fix of a resolution of `text` that gives no value.
fn unavailable(env: &TestEnv, text: &str) -> (SecretFailure, String) {
    let secrets = goodfetch_test_support::runtime()
        .block_on(async { driver(env, &[]).for_run_with(Duration::from_secs(30), Redaction::On) });
    match resolved(&secrets, text) {
        Err(SecretError::Unavailable { failure, fix, .. }) => (failure, fix),
        Err(other) => panic!("expected `secret_unavailable`, found {other:?}"),
        Ok(_) => panic!("expected `secret_unavailable`, found a value"),
    }
}

/// Covers: story:4-50
#[rstest::rstest]
#[case::binary(false)]
#[case::armored(true)]
fn a_value_in_an_encrypted_local_file_resolves_through_secretfile_and_is_in_no_file_of_the_home(
    #[case] armored: bool,
) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let value = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let identity = identity(&env);
    plant(
        &env,
        "shop-key",
        &format!("{value}\n"),
        &identity.to_public(),
        armored,
    );
    let secrets = goodfetch_test_support::runtime()
        .block_on(async { driver(&env, &[]).for_run_with(Duration::from_secs(30), Redaction::On) });

    let resolved = resolved(&secrets, "secretfile:shop-key").unwrap();

    assert_eq!(
        (
            resolved.expose_secret(),
            canaries.leaks(&[Output::Path(env.home())]).unwrap()
        ),
        (value.as_str(), vec![])
    );
}

/// Covers: story:4-50, error:secret_unavailable
#[test]
fn a_name_with_no_encrypted_local_file_is_missing() {
    let env = TestEnv::new();
    identity(&env);

    assert_eq!(
        unavailable(&env, "secretfile:shop-key").0,
        SecretFailure::Missing
    );
}

/// Covers: story:4-50, error:secret_unavailable
#[test]
fn a_home_with_no_identity_file_is_not_configured_with_the_command_that_makes_one() {
    let env = TestEnv::new();

    let (failure, fix) = unavailable(&env, "secretfile:shop-key");

    assert_eq!(
        (failure, fix.contains("age-keygen -o")),
        (SecretFailure::NotConfigured, true),
        "{fix}"
    );
}

/// Covers: story:4-50, error:secret_unavailable
#[cfg(unix)]
#[rstest::rstest]
#[case::readable_by_its_group(0o640)]
#[case::readable_by_everyone(0o644)]
fn an_identity_file_other_users_may_read_is_refused_with_the_chmod_that_closes_it(
    #[case] mode: u32,
) {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let identity = identity(&env);
    plant(
        &env,
        "shop-key",
        "s3cr3t-value",
        &identity.to_public(),
        false,
    );
    let path = folder(&env).join("identity.txt");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();

    let (failure, fix) = unavailable(&env, "secretfile:shop-key");

    assert_eq!(
        (
            failure,
            fix.contains(&format!("chmod 600 {}", path.display()))
        ),
        (SecretFailure::NotConfigured, true),
        "{fix}"
    );
}

/// Covers: story:4-50, error:secret_unavailable
#[test]
fn a_file_encrypted_to_another_identity_names_the_recipient_to_encrypt_it_to() {
    let env = TestEnv::new();
    let identity = identity(&env);
    plant(
        &env,
        "shop-key",
        "s3cr3t-value",
        &x25519::Identity::generate().to_public(),
        false,
    );

    let (failure, fix) = unavailable(&env, "secretfile:shop-key");

    assert_eq!(
        (
            failure,
            fix.contains(&format!("age -r {}", identity.to_public()))
        ),
        (SecretFailure::ProviderFailed, true),
        "{fix}"
    );
}

/// Covers: story:4-50, error:secret_unavailable
#[cfg(windows)]
#[test]
fn an_identity_file_everyone_may_read_is_refused_with_the_icacls_that_closes_it() {
    let env = TestEnv::new();
    let identity = identity(&env);
    plant(
        &env,
        "shop-key",
        "s3cr3t-value",
        &identity.to_public(),
        false,
    );
    let path = folder(&env).join("identity.txt");
    let granted = std::process::Command::new("icacls")
        .arg(&path)
        .args(["/grant", "*S-1-1-0:R"])
        .status()
        .unwrap();
    assert!(
        granted.success(),
        "icacls could not widen the identity file"
    );

    let (failure, fix) = unavailable(&env, "secretfile:shop-key");

    assert_eq!(
        (failure, fix.contains("icacls")),
        (SecretFailure::NotConfigured, true),
        "{fix}"
    );
}

/// Makes the engine's own key-encryption key in `env`'s encrypted local
/// file `run-state-key`, as a Run's first seal does, and answers how it
/// went.
fn make(env: &TestEnv) -> Result<(), SecretError> {
    goodfetch_test_support::runtime().block_on(async {
        driver(env, &[])
            .make_key(&super::reference("secretfile:run-state-key"))
            .await
    })
}

/// In a home with no encrypted local file, the engine makes the identity,
/// only its owner's, and the file of its own key: 32 random bytes in
/// Base64, which resolve through `secretfile:`; made again, the first key
/// stays, since a new one would open no Run's data key.
/// Covers: story:4-64, setting:run_state.key_reference
#[test]
fn the_engines_own_key_is_made_once_in_the_encrypted_local_file_with_an_owner_only_identity() {
    let env = TestEnv::new();

    make(&env).unwrap();
    let first = resolved_key(&env);
    make(&env).unwrap();
    let second = resolved_key(&env);

    let decoded = data_encoding::BASE64
        .decode(first.as_bytes())
        .map(|key| key.len());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(folder(&env).join("identity.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the identity is {mode:o}");
    }
    assert_eq!((decoded, first == second), (Ok(32), true));
}

/// A Run whose engine key was not configured at its first resolution, in a
/// home with no encrypted local file yet, resolves it once the engine made
/// it, as a Run's first seal does: the reuse within a Run keeps no failure.
/// Covers: story:4-64, setting:run_state.key_reference
#[test]
fn a_key_missing_at_a_runs_first_resolution_resolves_in_that_run_once_made() {
    let env = TestEnv::new();
    let (secrets, run) = goodfetch_test_support::runtime().block_on(async {
        let secrets = driver(&env, &[]);
        let run = secrets.for_run_with(Duration::from_secs(30), Redaction::On);
        (secrets, run)
    });

    let first = resolved(&run, "secretfile:run-state-key");
    goodfetch_test_support::runtime()
        .block_on(secrets.make_key(&super::reference("secretfile:run-state-key")))
        .unwrap();
    let again = resolved(&run, "secretfile:run-state-key");

    assert!(
        matches!(
            (&first, &again),
            (
                Err(SecretError::Unavailable {
                    failure: SecretFailure::NotConfigured,
                    ..
                }),
                Ok(_)
            )
        ),
        "first {first:?}, again {again:?}"
    );
}

/// The value `secretfile:run-state-key` resolves to in `env`'s home.
fn resolved_key(env: &TestEnv) -> String {
    let secrets = goodfetch_test_support::runtime()
        .block_on(async { driver(env, &[]).for_run_with(Duration::from_secs(30), Redaction::On) });
    resolved(&secrets, "secretfile:run-state-key")
        .unwrap()
        .expose_secret()
        .to_owned()
}

/// An identity others may read refuses the making of the engine's key as
/// it refuses a read, with the `chmod` that closes it, and no key file is
/// written.
/// Covers: story:4-64, error:secret_unavailable
#[cfg(unix)]
#[test]
fn the_engines_own_key_is_not_made_beside_an_identity_others_may_read() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    identity(&env);
    let path = folder(&env).join("identity.txt");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let made = make(&env);

    let Err(SecretError::Unavailable { failure, fix, .. }) = made else {
        panic!("expected `secret_unavailable`, found {made:?}");
    };
    assert_eq!(
        (
            failure,
            fix.contains("chmod 600"),
            folder(&env).join("run-state-key.age").exists()
        ),
        (SecretFailure::NotConfigured, true, false),
        "{fix}"
    );
}

/// The engine makes only its own key: any other reference is refused, and
/// nothing is written.
/// Covers: story:4-64, error:secret_unavailable
#[test]
fn the_engine_makes_no_key_where_another_reference_points() {
    let env = TestEnv::new();

    let made = goodfetch_test_support::runtime().block_on(async {
        driver(&env, &[])
            .make_key(&super::reference("secretfile:shop-key"))
            .await
    });

    assert!(
        matches!(
            made,
            Err(SecretError::Unavailable {
                failure: SecretFailure::NotConfigured,
                ..
            })
        ) && !folder(&env).exists(),
        "{made:?}"
    );
}
