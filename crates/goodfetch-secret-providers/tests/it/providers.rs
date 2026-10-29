//! Each provider through the driver, its CLI a shell-script stand-in that
//! answers as 1Password's `op` and Bitwarden's `bw` document: a value
//! planted in the stand-in's store resolves at the moment of use and is
//! found in no output (the Secret canary harness, #379); a provider signed
//! out or unreachable fails with `secret_unavailable` naming the provider
//! and the failure, never a value; and `check_config` refuses a missing or
//! malformed `secret_providers.<scheme>.command` with a fix.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::secrets::{
    Carried, Keychain, Purpose, RunSecrets, SecretError, SecretProvider, Secrets,
};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_secret_providers::{Bitwarden, OnePassword};
use goodfetch_test_support::{Canaries, SecretKind, TestEnv};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::support::{leaks, reference, runtime, unavailable};

/// The token the `op` stand-in takes as signed in; no real token's shape.
const OP_TOKEN: &str = "stand-in-service-account";

/// The session the `bw` stand-in takes as unlocked.
const BW_SESSION: &str = "stand-in-session";

/// `op` as its documentation describes `op read`, for one vault `Shop`
/// holding the item `portal`, whose `password` is the store file's content.
fn op_script(store: &Path) -> String {
    format!(
        r#"#!/bin/sh
if [ "$OP_SERVICE_ACCOUNT_TOKEN" != "{OP_TOKEN}" ]; then
  echo '[ERROR] 2026/10/26 10:00:00 You are not currently signed in. Please run `op signin --help` for instructions' >&2
  exit 1
fi
if [ -n "$STAND_IN_UNREACHABLE" ]; then
  echo '[ERROR] 2026/10/26 10:00:00 Get "https://my.1password.com/api/v2/account": dial tcp: lookup my.1password.com: no such host' >&2
  exit 1
fi
if [ "$1 $2 $3" = "read --no-newline op://Shop/portal/password" ]; then
  cat '{store}'
  exit 0
fi
echo "[ERROR] 2026/10/26 10:00:00 could not read secret '$3': could not get item: \"admin\" isn't an item in the \"Shop\" vault. Specify the item with its UUID, name, or domain." >&2
exit 1
"#,
        store = store.display()
    )
}

/// `bw` as `bitwarden/clients`' CLI answers `get` without interaction and
/// without a terminal (no final newline), for one item `portal`: its
/// `password` is the store's `password` file, and `bw get item` its
/// `item.json`.
fn bw_script(store: &Path) -> String {
    format!(
        r#"#!/bin/sh
if [ "$BW_NOINTERACTION" != "true" ]; then
  printf '? Master password: [input is hidden]' >&2
  exit 1
fi
if [ "$BW_SESSION" != "{BW_SESSION}" ]; then
  printf 'Vault is locked.' >&2
  exit 1
fi
if [ -n "$STAND_IN_UNREACHABLE" ]; then
  printf 'FetchError: request to https://vault.example.test/api/sync failed, reason: getaddrinfo ENOTFOUND vault.example.test' >&2
  exit 1
fi
case "$*" in
  "get password -- portal") cat '{store}/password' ;;
  "get item -- portal") cat '{store}/item.json' ;;
  *) printf 'Not found.' >&2; exit 1 ;;
esac
"#,
        store = store.display()
    )
}

/// Which of the two providers a case drives.
#[derive(Clone, Copy, Debug)]
enum Vendor {
    OnePassword,
    Bitwarden,
}

impl Vendor {
    fn provider(self) -> Provider {
        match self {
            Self::OnePassword => Provider::OnePassword,
            Self::Bitwarden => Provider::Bitwarden,
        }
    }

    /// The reference to the planted password.
    fn reference(self) -> SecretReference {
        reference(match self {
            Self::OnePassword => "op://Shop/portal/password",
            Self::Bitwarden => "bw:portal/password",
        })
    }

    /// The reference to an item the stand-in does not hold.
    fn missing(self) -> SecretReference {
        reference(match self {
            Self::OnePassword => "op://Shop/admin/password",
            Self::Bitwarden => "bw:admin/password",
        })
    }

    /// The environment that signs the stand-in in.
    fn signed_in(self) -> (&'static str, &'static str) {
        match self {
            Self::OnePassword => ("OP_SERVICE_ACCOUNT_TOKEN", OP_TOKEN),
            Self::Bitwarden => ("BW_SESSION", BW_SESSION),
        }
    }

    fn implementation(
        self,
        command: Option<String>,
        environment: BTreeMap<OsString, OsString>,
    ) -> Arc<dyn SecretProvider> {
        match self {
            Self::OnePassword => Arc::new(OnePassword::new(command, environment)),
            Self::Bitwarden => Arc::new(Bitwarden::new(command, environment)),
        }
    }
}

/// How the stand-in's party stands.
#[derive(Clone, Copy, Debug)]
enum Party {
    SignedIn,
    SignedOut,
    Unreachable,
}

/// One test's stand-in CLIs, their stores and the canaries planted there.
struct Fixture {
    env: TestEnv,
    canaries: Canaries,
    /// The stand-ins' folder, outside the home.
    stand_ins: PathBuf,
    /// The value planted as each item's password.
    password: String,
    /// The value planted as the Bitwarden item's custom field `pin`.
    pin: String,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the fixture writes the stand-ins and their stores before any runtime starts"
)]
fn fixture() -> Fixture {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let stand_ins = env.folder().join("stand-ins");
    let store = stand_ins.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let (password_id, password) = {
        let canary = canaries.plant(SecretKind::TypedPassword);
        (canary.id().to_owned(), canary.value().to_owned())
    };
    let (pin_id, pin) = {
        let canary = canaries.plant(SecretKind::TypedPassword);
        (canary.id().to_owned(), canary.value().to_owned())
    };
    std::fs::write(store.join("password"), &password).unwrap();
    let item = json!({
        "object": "item",
        "id": "6d1f0b2e-0000-0000-0000-000000000001",
        "type": 1,
        "name": "portal",
        "login": { "username": "shopper", "password": password },
        "fields": [{ "name": "pin", "value": pin, "type": 1 }],
    });
    std::fs::write(store.join("item.json"), item.to_string()).unwrap();
    for (id, file) in [
        (&password_id, "password"),
        (&password_id, "item.json"),
        (&pin_id, "item.json"),
    ] {
        canaries.record_planted(id, &store.join(file).display().to_string());
    }
    for (name, script) in [
        ("op", op_script(&store.join("password"))),
        ("bw", bw_script(&store)),
    ] {
        let path = stand_ins.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    Fixture {
        env,
        canaries,
        stand_ins,
        password,
        pin,
    }
}

impl Fixture {
    fn command(&self, vendor: Vendor) -> String {
        let name = match vendor {
            Vendor::OnePassword => "op",
            Vendor::Bitwarden => "bw",
        };
        self.stand_ins.join(name).display().to_string()
    }

    fn environment(vendor: Vendor, party: Party) -> BTreeMap<OsString, OsString> {
        let mut environment = BTreeMap::new();
        if !matches!(party, Party::SignedOut) {
            let (name, value) = vendor.signed_in();
            environment.insert(OsString::from(name), OsString::from(value));
        }
        if matches!(party, Party::Unreachable) {
            environment.insert(OsString::from("STAND_IN_UNREACHABLE"), OsString::from("1"));
        }
        environment
    }

    /// The driver over the test's home with `implementation` answering
    /// `vendor`'s references.
    fn driver(&self, vendor: Vendor, implementation: Arc<dyn SecretProvider>) -> RunSecrets {
        let home = Home::new(self.env.home().to_path_buf());
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
        )]
        let entropy = goodfetch_engine::entropy::Entropy::seeded(0);
        runtime().block_on(async {
            Secrets::new(
                Carried {
                    environment: BTreeMap::new(),
                    keychain: Keychain::absent(),
                    home: home.clone(),
                    entropy,
                },
                Arc::new(AuditLog::of(&home, None)),
                Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
                Clock::system(),
            )
            .with(vendor.provider(), implementation)
            .for_run(Duration::from_secs(30))
        })
    }

    /// The driver over `vendor`'s stand-in, its party as `party` stands.
    fn over(&self, vendor: Vendor, party: Party) -> RunSecrets {
        self.driver(
            vendor,
            vendor.implementation(Some(self.command(vendor)), Self::environment(vendor, party)),
        )
    }
}

fn through(
    secrets: &RunSecrets,
    reference: &SecretReference,
) -> Result<secrecy::SecretString, SecretError> {
    runtime().block_on(secrets.resolve(
        reference,
        &Purpose {
            placeholder: "portal_password",
            domain: Some("shop.test"),
            domains: &["shop.test".to_owned()],
            layer: Some(SettingsLayer::UserConfiguration),
            run_id: None,
        },
    ))
}

/// A value planted in each provider's store resolves through its CLI at
/// the moment of use, is held for the run, and is in no output: not the
/// home, its Audit log included, and not what the test formats.
/// Covers: story:4-50
#[rstest]
#[case::one_password(Vendor::OnePassword)]
#[case::bitwarden(Vendor::Bitwarden)]
fn a_planted_value_resolves_through_its_provider_and_is_found_in_no_output(#[case] vendor: Vendor) {
    let fixture = fixture();
    let secrets = fixture.over(vendor, Party::SignedIn);

    let resolved = through(&secrets, &vendor.reference()).unwrap();
    let held = runtime().block_on(secrets.held());
    let formatted = format!("{resolved:?} {held:?} {secrets:?}");

    assert_eq!(
        (
            resolved.expose_secret() == fixture.password,
            held.iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("formatted", formatted.as_bytes())]
            ),
        ),
        (
            true,
            vec!["portal_password".to_owned()],
            Vec::<String>::new()
        )
    );
}

/// A Bitwarden reference to a custom field reads it from the item.
/// Covers: story:4-50
#[test]
fn a_bitwarden_custom_field_resolves_from_its_item() {
    let fixture = fixture();
    let secrets = fixture.over(Vendor::Bitwarden, Party::SignedIn);

    let pin = through(&secrets, &reference("bw:portal/pin")).unwrap();
    let absent = unavailable(through(&secrets, &reference("bw:portal/badge"))).0;

    assert_eq!(
        (pin.expose_secret() == fixture.pin, absent),
        (true, SecretFailure::Missing)
    );
}

/// A signed-out or unreachable provider fails with `secret_unavailable`,
/// `provider_failed`, its message naming the provider's reference and its
/// fix, never a value; an item it does not hold is `missing`.
/// Covers: story:4-50, error:secret_unavailable
#[rstest]
#[case::one_password_signed_out(Vendor::OnePassword, Party::SignedOut, "sign")]
#[case::one_password_unreachable(Vendor::OnePassword, Party::Unreachable, "reach")]
#[case::bitwarden_locked(Vendor::Bitwarden, Party::SignedOut, "BW_SESSION")]
#[case::bitwarden_unreachable(Vendor::Bitwarden, Party::Unreachable, "reach")]
fn an_unauthorised_or_unreachable_provider_fails_naming_it_and_no_value(
    #[case] vendor: Vendor,
    #[case] party: Party,
    #[case] fix_names: &str,
) {
    let fixture = fixture();
    let secrets = fixture.over(vendor, party);

    let (failure, message) = unavailable(through(&secrets, &vendor.reference()));
    let (missing, _) = unavailable(through(
        &fixture.over(vendor, Party::SignedIn),
        &vendor.missing(),
    ));

    assert_eq!(
        (
            failure,
            message.contains(vendor.reference().as_str()) && message.contains(fix_names),
            missing,
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("message", message.as_bytes())]
            ),
        ),
        (
            SecretFailure::ProviderFailed,
            true,
            SecretFailure::Missing,
            Vec::<String>::new()
        ),
        "{message}"
    );
}

/// `check_config` refuses an unset or relative command with a fix naming
/// the setting, which the driver answers with `not_configured`; one that
/// names no file is `not_configured` too.
/// Covers: story:4-50, error:secret_unavailable
#[rstest]
/// Covers: setting:secret_providers.op.command
#[case::op_unset(Vendor::OnePassword, None, "secret_providers.op.command")]
/// Covers: setting:secret_providers.op.command
#[case::op_relative(Vendor::OnePassword, Some("op"), "absolute path")]
/// Covers: setting:secret_providers.op.command
#[case::op_absent(Vendor::OnePassword, Some("/nonexistent/op"), "does not exist")]
/// Covers: setting:secret_providers.bw.command
#[case::bw_unset(Vendor::Bitwarden, None, "secret_providers.bw.command")]
/// Covers: setting:secret_providers.bw.command
#[case::bw_relative(Vendor::Bitwarden, Some("bin/bw"), "absolute path")]
/// Covers: setting:secret_providers.bw.command
#[case::bw_absent(Vendor::Bitwarden, Some("/nonexistent/bw"), "does not exist")]
fn a_missing_or_malformed_command_is_not_configured_with_a_fix(
    #[case] vendor: Vendor,
    #[case] command: Option<&str>,
    #[case] fix_names: &str,
) {
    let fixture = fixture();
    let implementation = vendor.implementation(
        command.map(str::to_owned),
        Fixture::environment(vendor, Party::SignedIn),
    );
    let checked = implementation
        .check_config()
        .err()
        .map(|problem| problem.fix);
    let secrets = fixture.driver(vendor, implementation);

    let (failure, message) = unavailable(through(&secrets, &vendor.reference()));

    // An absolute path passes the check; only running it finds no file.
    let absolute = command.is_some_and(|command| command.starts_with('/'));
    assert_eq!(
        (
            failure,
            message.contains(fix_names),
            checked.map(|fix| fix.contains(fix_names)),
        ),
        (
            SecretFailure::NotConfigured,
            true,
            (!absolute).then_some(true)
        ),
        "{message}"
    );
}

/// The provider takes its command from `secret_providers.<scheme>.command`
/// as the Run's settings resolve it.
/// Covers: story:4-50
#[rstest]
/// Covers: setting:secret_providers.op.command
#[case::one_password(Vendor::OnePassword, "op")]
/// Covers: setting:secret_providers.bw.command
#[case::bitwarden(Vendor::Bitwarden, "bw")]
fn the_command_comes_from_configuration(#[case] vendor: Vendor, #[case] scheme: &str) {
    let fixture = fixture();
    let configuration =
        json!({ "secret_providers": { scheme: { "command": fixture.command(vendor) } } });
    let serde_json::Value::Object(table) = configuration else {
        unreachable!()
    };
    let resolved = resolve(
        &Layers {
            user_configuration: Values::read(
                SettingsLayer::UserConfiguration,
                &table,
                Product::Collie,
            )
            .unwrap(),
            ..Layers::default()
        },
        Product::Collie,
        Product::Collie,
    )
    .unwrap();
    let environment = Fixture::environment(vendor, Party::SignedIn);
    let implementation: Arc<dyn SecretProvider> = match vendor {
        Vendor::OnePassword => Arc::new(OnePassword::configured(&resolved, environment)),
        Vendor::Bitwarden => Arc::new(Bitwarden::configured(&resolved, environment)),
    };
    let secrets = fixture.driver(vendor, implementation);

    let resolved = through(&secrets, &vendor.reference()).unwrap();

    assert_eq!(resolved.expose_secret(), fixture.password);
}
