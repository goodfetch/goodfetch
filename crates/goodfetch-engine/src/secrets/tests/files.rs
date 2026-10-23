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
        .block_on(async { driver(env, &[]).for_run(Duration::from_secs(30)) });
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
        .block_on(async { driver(&env, &[]).for_run(Duration::from_secs(30)) });

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
