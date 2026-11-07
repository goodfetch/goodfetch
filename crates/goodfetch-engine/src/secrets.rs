//! The Secret-provider driver (ADRs 0015, 0016, 0092 and 0129): what every
//! Secret provider does the same way, written once above them. A value is
//! resolved at the moment of use and never earlier, only for a domain its
//! binding allows, within `timeout.secret`; each resolution is recorded in
//! the Audit log with its placeholder, reference and receiving domain,
//! never the value; the value is held for the rest of the run as a
//! `SecretString`, zeroized once the last handle drops, where the run's
//! `Mask` finds it, and reused at the reference's later uses for the same
//! placeholder, its binding checked again each time, unless
//! `secrets.resolve_each_use` lists it; a provider that answers it is busy
//! is asked again within `timeout.secret` (ADR 0014's note of 2026-10-28);
//! and a failure is one of [`SecretFailure`]'s variants, each leading to
//! `secret_unavailable` with its fix.
//!
//! A provider fills only two primitives, [`SecretProvider::resolve`] and
//! [`SecretProvider::check_config`]. This module carries the providers
//! every engine needs (ADR 0015, spec #564): the environment's,
//! `env:NAME`; the OS keychain's, `keychain:service/account`; and the
//! encrypted local file's, `secretfile:name`, where no keychain exists,
//! which is also where `run_state.key_reference`'s default falls back to
//! ([`host_defaults`]). Each is a translation point (ADR 0127): what its
//! party answers becomes a [`ProviderFailure`] there and nowhere else.
//!
//! Size: one driver, the resolution's checks, reuse, retry, Audit record
//! and hold, kept with the engine key's making that resolves through them;
//! the providers live in their own modules.

mod environment;
mod keychain;
mod secretfile;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::audit::{Actor, SECRET_REFERENCE_USED};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_protocol::secrets::{
    KEY_REFERENCE_IN_FILE, KEY_REFERENCE_IN_KEYCHAIN, RUN_STATE_KEY_REFERENCE,
    SECRETS_RESOLVE_EACH_USE, SecretFailure, SecretInputRefusedReason,
};
use goodfetch_protocol::settings::SettingsLayer;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use subtle::ConstantTimeEq as _;

pub use keychain::Keychain;

use crate::audit::{AuditError, AuditLog, Origin};
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::home::Home;
use crate::known_certificates::{ExceptionKind, KeyHash};
use crate::mask::{Mask, Masks, Reach, Redaction};
use crate::settings::{HostDefaults, Resolved};
use crate::tasks::TaskOwner;

/// A provider's answer to one reference, which the driver bounds by
/// `timeout.secret`.
pub type Resolving<'a> =
    Pin<Box<dyn Future<Output = Result<SecretString, ProviderFailure>> + Send + 'a>>;

/// The two primitives a Secret provider fills (ADR 0129); everything else
/// is the driver's.
pub trait SecretProvider: Send + Sync {
    /// The value `reference` points to. A reference of another provider
    /// never reaches it.
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a>;

    /// Whether its configuration lets it answer, such as its
    /// `secret_providers.<scheme>.*` settings.
    ///
    /// # Errors
    ///
    /// What is missing, as a fix naming the settings, never a value.
    fn check_config(&self) -> Result<(), ConfigProblem>;
}

/// How a provider failed to give a value, which the driver turns into
/// [`SecretFailure`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderFailure {
    /// It holds no value for the reference, such as an unset variable:
    /// `missing`.
    Missing,
    /// It cannot answer as this host or its setup stands, such as on a host
    /// with no OS keychain or with an identity file other users may read:
    /// `not_configured`, with what makes it answer.
    NotConfigured(ConfigProblem),
    /// It refused in a way its translation point knows, such as a locked
    /// keychain or a file encrypted to another identity: `provider_failed`,
    /// with what makes it answer.
    Refused {
        /// What makes it answer, never a value.
        fix: String,
    },
    /// It is briefly unable to answer, such as a rate limit's 429 or a
    /// 503, and may answer if asked again: the driver asks again within
    /// `timeout.secret`, after `retry_after` when it gave one, and then
    /// fails `provider_failed` with `fix`.
    Busy {
        /// What makes it answer, never a value.
        fix: String,
        /// How long it asked to be left alone, its `Retry-After`.
        retry_after: Option<Duration>,
    },
    /// It failed in a way its translation point does not know (ADR 0127),
    /// logged at `warn` without its text: `provider_failed`.
    Unrecognized(PartyText),
}

/// What a provider's configuration lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigProblem {
    /// What makes it answer, never a value.
    pub fix: String,
}

/// A party's own text about a failure its translation point does not know
/// (ADR 0127): kept for diagnosis, read by no code, and redacted wherever
/// it is formatted, since a party may echo what it was given, a value
/// included.
#[derive(Clone, PartialEq, Eq)]
pub struct PartyText(String);

impl PartyText {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl std::fmt::Debug for PartyText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "PartyText(<{} bytes, redacted>)", self.0.len())
    }
}

/// What a Product's provider is built from for one Run: the Run's resolved
/// settings, with its `secret_providers.<scheme>.*`, the engine's
/// environment map (ADR 0083), where a provider's own sign-in is, and the
/// engine's clock.
#[derive(Clone, Copy, Debug)]
pub struct ProviderInputs<'a> {
    /// The Run's resolved settings.
    pub settings: &'a Resolved,
    /// The engine's environment map.
    pub environment: &'a BTreeMap<OsString, OsString>,
    /// The engine's clock.
    pub clock: Clock,
}

/// Builds a Product's provider for each Run from [`ProviderInputs`], since
/// its configuration may come from the Run's own project configuration.
pub type ProviderFactory =
    Arc<dyn for<'a> Fn(&ProviderInputs<'a>) -> Arc<dyn SecretProvider> + Send + Sync>;

/// The schemes whose providers every engine carries, which
/// [`Secrets::new`] registers: `env:`, `keychain:` and `secretfile:`.
pub const CARRIED: [Provider; 3] = [
    Provider::Environment,
    Provider::Keychain,
    Provider::SecretFile,
];

/// The providers every engine carries beside the ones a Product adds: the
/// environment map `env:` reads, the OS keychain `keychain:` reads, and the
/// Goodfetch home whose encrypted local files `secretfile:` reads.
#[derive(Debug)]
pub struct Carried {
    /// The engine's environment map (ADR 0083).
    pub environment: BTreeMap<OsString, OsString>,
    /// The OS keychain, or none.
    pub keychain: Keychain,
    /// The Goodfetch home.
    pub home: Home,
    /// The engine's entropy, which the waits before a busy provider is
    /// asked again draw from.
    pub entropy: Entropy,
}

/// The defaults this host gives in place of the catalog's: the encrypted
/// local file's `run_state.key_reference` where `keychain` is absent. It
/// opens the OS keychain if no one has.
///
/// # Cancel safety
///
/// Dropping the future leaves the keychain to open on the next call.
pub async fn host_defaults(keychain: &Keychain) -> HostDefaults {
    if keychain.available().await {
        HostDefaults::default()
    } else {
        HostDefaults::default().with(&RUN_STATE_KEY_REFERENCE, json!(KEY_REFERENCE_IN_FILE))
    }
}

/// What a value is resolved for: the name it is used under, where it goes
/// and the Run it is used in.
#[derive(Clone, Copy, Debug)]
pub struct Purpose<'a> {
    /// The Secret placeholder, or the setting's key where no placeholder
    /// names the value, such as `run_state.key_reference`.
    pub placeholder: &'a str,
    /// The host that receives the value; none when the engine itself uses
    /// it, such as a run's key-encryption key.
    pub domain: Option<&'a str>,
    /// The domains its binding allows the value to go to, each with its
    /// subdomains; empty when no binding restricts it.
    pub domains: &'a [String],
    /// The settings layer the reference came from.
    pub layer: Option<SettingsLayer>,
    /// The Run it is used in, once known.
    pub run_id: Option<&'a RunId>,
}

/// Why a reference gave no value. Each names the placeholder and the
/// reference, never a value (ADR 0092).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// The provider gave no value: `secret_unavailable`.
    #[error(
        "the Secret reference `{reference}` for `{placeholder}` gave no value ({}): {fix}",
        failure.as_str()
    )]
    Unavailable {
        /// The placeholder or the setting's key.
        placeholder: String,
        /// The reference.
        reference: SecretReference,
        /// What failed.
        failure: SecretFailure,
        /// What makes it resolve.
        fix: String,
    },
    /// The receiving domain is outside the binding's: `secret_domain_not_allowed`.
    #[error("`{placeholder}` may not be sent to `{domain}`, outside its binding's domains")]
    DomainNotAllowed {
        /// The placeholder.
        placeholder: String,
        /// The reference.
        reference: SecretReference,
        /// The host it would have gone to, or `engine` when none.
        domain: String,
        /// The binding's domains.
        domains: Vec<String>,
    },
    /// The receiving host may not get a value at all, such as one reached
    /// through an invalid-certificate exception: `secret_input_refused`.
    #[error("`{placeholder}` may not be sent to `{domain}` ({})", reason_name(*reason))]
    InputRefused {
        /// The placeholder.
        placeholder: String,
        /// The reference.
        reference: SecretReference,
        /// The host it would have gone to.
        domain: String,
        /// Why it may not go there.
        reason: SecretInputRefusedReason,
    },
    /// The resolution could not be recorded in the Audit log, so its value
    /// was dropped unused.
    #[error("the use of `{reference}` for `{placeholder}` cannot be recorded: {source}")]
    Unrecorded {
        /// The placeholder.
        placeholder: String,
        /// The reference.
        reference: SecretReference,
        /// The Audit log's failure.
        #[source]
        source: AuditError,
    },
}

/// A provider as the driver holds it.
#[derive(Clone)]
struct Implementation(Arc<dyn SecretProvider>);

/// The engine's Secret-provider driver: its providers by scheme, the
/// Audit log every resolution is recorded in, and where its blocking
/// writes run. Clones share it.
#[derive(Clone)]
pub struct Secrets(Arc<Shared>);

#[derive(Clone)]
struct Shared {
    providers: BTreeMap<Provider, Implementation>,
    /// The providers a Product builds for each Run from its settings.
    configured: BTreeMap<Provider, ProviderFactory>,
    /// The engine's environment map, which a configured provider reads.
    environment: BTreeMap<OsString, OsString>,
    audit: Arc<AuditLog>,
    owner: Arc<TaskOwner>,
    clock: Clock,
    /// The OS keychain and the encrypted local files' folder, where the
    /// engine makes its own key-encryption key.
    keychain: Keychain,
    secret_files: PathBuf,
    /// The file whose lock the engine holds while it makes its own key in
    /// the OS keychain.
    key_lock: PathBuf,
    /// Each engine-owned key-encryption key ([`ENGINE_KEYS`]) once this
    /// driver made it or found it made: the first caller makes it, and
    /// every later or concurrent one waits for that making, so the engine
    /// makes one.
    made: Arc<[tokio::sync::OnceCell<()>; 2]>,
    /// Every live Run's resolved values, which each Run's own join.
    masks: Masks,
    /// Where the waits before a busy provider is asked again draw from.
    entropy: Arc<tokio::sync::Mutex<Entropy>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Secrets")
            .field("providers", &self.0.providers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Secrets {
    /// The driver with the providers every engine carries, over `carried`.
    /// Each resolution is recorded in `audit`; the providers' reads and the
    /// Audit log's writes block under `owner`.
    pub fn new(
        carried: Carried,
        audit: Arc<AuditLog>,
        owner: Arc<TaskOwner>,
        clock: Clock,
    ) -> Self {
        let keychain = carried.keychain.clone();
        let secret_files = carried.home.secret_files();
        let key_lock = carried.home.engine_key_lock();
        let mut providers = BTreeMap::new();
        let environment_map = carried.environment.clone();
        providers.insert(
            Provider::Environment,
            Implementation(Arc::new(environment::Environment::new(carried.environment))),
        );
        providers.insert(
            Provider::Keychain,
            Implementation(Arc::new(keychain::KeychainProvider::new(
                carried.keychain,
                Arc::clone(&owner),
            ))),
        );
        providers.insert(
            Provider::SecretFile,
            Implementation(Arc::new(secretfile::SecretFiles::new(
                secret_files.clone(),
                Arc::clone(&owner),
            ))),
        );
        Self(Arc::new(Shared {
            providers,
            configured: BTreeMap::new(),
            environment: environment_map,
            audit,
            owner,
            clock,
            keychain,
            secret_files,
            key_lock,
            made: Arc::default(),
            masks: Masks::new(),
            entropy: Arc::new(tokio::sync::Mutex::new(carried.entropy)),
        }))
    }

    /// The same driver, each Run's values joining `masks`, the engine's,
    /// in place of a set of its own.
    #[must_use]
    pub fn masked_by(mut self, masks: Masks) -> Self {
        Arc::make_mut(&mut self.0).masks = masks;
        self
    }

    /// Makes the item `reference` names hold a new random key-encryption
    /// key, unless it holds a value already: only for the items the engine
    /// owns, `run_state.key_reference`'s defaults ([`is_engine_key`]), in
    /// the OS keychain or the encrypted local file, whose identity it makes
    /// too. The key is 32 bytes from the OS generator, in Base64. In the
    /// keychain the item is read and stored under the home's
    /// `engine-key.lock`, so two engines making it at once store one key,
    /// which both then read.
    ///
    /// # Errors
    ///
    /// `Unavailable` naming `run_state.key_reference` when the provider
    /// refuses the write, `not_configured` for a reference the engine does
    /// not own.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the item made or not; the next call
    /// makes it again where it is missing.
    pub(crate) async fn make_key(&self, reference: &SecretReference) -> Result<(), SecretError> {
        let unavailable = |failure: ProviderFailure| {
            let (failure, fix) = failure_of(failure, reference);
            SecretError::Unavailable {
                placeholder: RUN_STATE_KEY_REFERENCE.name.to_owned(),
                reference: reference.clone(),
                failure,
                fix,
            }
        };
        let Some(engine_key) = ENGINE_KEYS
            .iter()
            .position(|engine_key| *engine_key == reference.as_str())
        else {
            return Err(unavailable(ProviderFailure::NotConfigured(ConfigProblem {
                fix: format!(
                    "The engine makes only its own key-encryption key; store one where `{reference}` points."
                ),
            })));
        };
        self.0.made[engine_key]
            .get_or_try_init(|| self.make_engine_key(reference, unavailable))
            .await
            .copied()
    }

    /// Makes the engine-owned item `reference` names, unless it holds a
    /// value already, failing through `unavailable`.
    async fn make_engine_key(
        &self,
        reference: &SecretReference,
        unavailable: impl Fn(ProviderFailure) -> SecretError,
    ) -> Result<(), SecretError> {
        let mut bytes = secrecy::zeroize::Zeroizing::new([0_u8; 32]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| {
            unavailable(ProviderFailure::Refused {
                fix: "The OS generator gave no random bytes for the key-encryption key; try again."
                    .to_owned(),
            })
        })?;
        let key = SecretString::from(data_encoding::BASE64.encode(bytes.as_slice()));
        if reference.provider() == Provider::Keychain {
            let (service, account) = reference
                .rest()
                .split_once('/')
                .expect("invariant: a `keychain:` reference parses as service/account");
            // The item is read, then stored where it is missing, under a
            // lock every engine of the user takes, so of two engines making
            // it at once the second reads the first's key and stores none.
            let path = self.0.key_lock.clone();
            let _lock = self
                .0
                .owner
                .blocking(move || lock_exclusive(&path))
                .await
                .map_err(|error| {
                    unavailable(ProviderFailure::Refused {
                        fix: format!(
                            "The engine cannot lock `{}` while it makes its key-encryption key ({error}); check that the Goodfetch home is writable.",
                            self.0.key_lock.display()
                        ),
                    })
                })?;
            match self
                .0
                .keychain
                .read(service.to_owned(), account.to_owned(), &self.0.owner)
                .await
            {
                Ok(_) => Ok(()),
                Err(ProviderFailure::Missing) => self
                    .0
                    .keychain
                    .store(service, account, &key, &self.0.owner)
                    .await
                    .map_err(unavailable),
                Err(failure) => Err(unavailable(failure)),
            }
        } else {
            let (folder, name) = (self.0.secret_files.clone(), reference.rest().to_owned());
            self.0
                .owner
                .blocking(move || secretfile::make(&folder, &name, &key))
                .await
                .map_err(unavailable)
        }
    }

    /// The same driver with `implementation` answering `provider`'s
    /// references in place of any it had, as a crate of built-in
    /// providers or a Secret provider Plugin adds one.
    #[must_use]
    pub fn with(mut self, provider: Provider, implementation: Arc<dyn SecretProvider>) -> Self {
        Arc::make_mut(&mut self.0)
            .providers
            .insert(provider, Implementation(implementation));
        self
    }

    /// The same driver with `factory` building, for each Run, the provider
    /// that answers `provider`'s references from the Run's settings
    /// ([`Secrets::for_run_under`]), in place of any it had: how a Product
    /// adds a provider configured by `secret_providers.<scheme>.*`.
    #[must_use]
    pub fn with_configured(mut self, provider: Provider, factory: ProviderFactory) -> Self {
        let shared = Arc::make_mut(&mut self.0);
        shared.providers.remove(&provider);
        shared.configured.insert(provider, factory);
        self
    }

    /// What one Run resolves through, as [`Secrets::for_run_with`], each
    /// provider a Product builds for each Run built from `settings`, the
    /// Run's resolved settings.
    pub fn for_run_under(
        &self,
        settings: &Resolved,
        timeout: Duration,
        redaction: Redaction,
    ) -> RunSecrets {
        let mut secrets = self.clone();
        if !self.0.configured.is_empty() {
            let shared = Arc::make_mut(&mut secrets.0);
            let inputs = ProviderInputs {
                settings,
                environment: &self.0.environment,
                clock: self.0.clock,
            };
            for (provider, factory) in &self.0.configured {
                shared
                    .providers
                    .insert(*provider, Implementation(factory(&inputs)));
            }
        }
        let each_use = settings
            .value(&SECRETS_RESOLVE_EACH_USE)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect();
        secrets.for_run_reusing(timeout, redaction, each_use)
    }

    /// Every live Run's resolved values.
    pub(crate) fn masks(&self) -> &Masks {
        &self.0.masks
    }

    /// What one Run resolves through, each provider bounded by `timeout`,
    /// its `timeout.secret`, holding every value it resolved until its last
    /// handle drops, its values masked in its kept outputs too.
    pub fn for_run(&self, timeout: Duration) -> RunSecrets {
        self.for_run_with(timeout, Redaction::On)
    }

    /// The same, its values masked in its kept outputs unless `redaction`,
    /// its `security.redaction`, is off.
    pub fn for_run_with(&self, timeout: Duration, redaction: Redaction) -> RunSecrets {
        self.for_run_reusing(timeout, redaction, BTreeSet::new())
    }

    /// The same, the references `each_use` names resolved again at each
    /// use.
    fn for_run_reusing(
        &self,
        timeout: Duration,
        redaction: Redaction,
        each_use: BTreeSet<String>,
    ) -> RunSecrets {
        RunSecrets(Arc::new(ForRun {
            secrets: self.clone(),
            timeout,
            held: tokio::sync::Mutex::new(Vec::new()),
            exception_hosts: ExceptionHosts::default(),
            mask: self.0.masks.for_run(redaction),
            values: tokio::sync::Mutex::default(),
            each_use,
        }))
    }
}

/// The hosts one Run reached through an invalid-certificate exception
/// (ADR 0042), with the key each presented and the kind of exception: the
/// engine's client sends them no Session cookie, the driver resolves no
/// value for them, and the Run's end records them once in the Audit log.
/// Hosts are held as the URL parser leaves them, without a trailing dot.
/// Clones share it.
#[derive(Clone, Debug, Default)]
pub struct ExceptionHosts(Arc<tokio::sync::Mutex<BTreeMap<String, (KeyHash, ExceptionKind)>>>);

impl ExceptionHosts {
    /// The key `host` presented, when the Run reached it through an
    /// exception.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn reached(&self, host: &str) -> Option<KeyHash> {
        self.0
            .lock()
            .await
            .get(&host_key(host))
            .map(|(key, _)| *key)
    }

    /// Adds `host`, reached through an exception of `kind` with `key`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the host out.
    pub(crate) async fn add(&self, host: &str, key: KeyHash, kind: ExceptionKind) {
        self.0.lock().await.insert(host_key(host), (key, kind));
    }

    /// Every host reached, in order, with the key it presented last and its
    /// exception's kind.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn used(&self) -> Vec<(String, KeyHash, ExceptionKind)> {
        self.0
            .lock()
            .await
            .iter()
            .map(|(host, (key, kind))| (host.clone(), *key, *kind))
            .collect()
    }
}

/// `host` in lower case, without the trailing dot of a fully qualified
/// name.
fn host_key(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    host.strip_suffix('.')
        .map_or_else(|| host.clone(), str::to_owned)
}

/// One value a Run resolved, under the name it was resolved for.
struct Held {
    placeholder: String,
    value: SecretString,
}

/// The driver as one Run resolves through it, with every value resolved
/// for it. Clones share it.
#[derive(Clone)]
pub struct RunSecrets(Arc<ForRun>);

struct ForRun {
    secrets: Secrets,
    timeout: Duration,
    held: tokio::sync::Mutex<Vec<Held>>,
    /// The hosts the Run reached through an invalid-certificate exception,
    /// which get no value.
    exception_hosts: ExceptionHosts,
    /// The Run's mask, which every value it resolves joins.
    mask: Mask,
    /// The value resolved for each reference and placeholder, reused for
    /// the rest of the Run; a resolution in flight is waited for, so
    /// resolutions of one reference at once make one call.
    values: tokio::sync::Mutex<BTreeMap<(String, String), Reused>>,
    /// The references resolved again at each use, its
    /// `secrets.resolve_each_use`.
    each_use: BTreeSet<String>,
}

/// A value a Run resolved once for a reference and placeholder, or the
/// resolution of it in flight, which its later uses wait for.
type Reused = Arc<tokio::sync::OnceCell<SecretString>>;

impl std::fmt::Debug for RunSecrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunSecrets")
            .field("timeout", &self.0.timeout)
            .finish_non_exhaustive()
    }
}

impl RunSecrets {
    /// The hosts the Run reached through an invalid-certificate exception,
    /// which its Sessions' traffic adds to: none gets a value (ADR 0042).
    pub fn exception_hosts(&self) -> ExceptionHosts {
        self.0.exception_hosts.clone()
    }

    /// The Run's mask, which every value it resolves joins under its
    /// placeholder, or the setting's key where none names it.
    pub fn mask(&self) -> &Mask {
        &self.0.mask
    }

    /// The value `reference` points to for `purpose`: refused for a domain
    /// its binding does not allow; the value resolved for its placeholder
    /// earlier in the Run, unless `secrets.resolve_each_use` lists it, or
    /// else asked of its provider now within `timeout.secret`; recorded in
    /// the Audit log, held for the rest of the Run and masked as
    /// `%placeholder%` wherever the engine returns, sends or keeps it.
    ///
    /// # Errors
    ///
    /// As [`SecretError`] says.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the resolution; a value it already
    /// recorded stays held.
    pub async fn resolve(
        &self,
        reference: &SecretReference,
        purpose: &Purpose<'_>,
    ) -> Result<SecretString, SecretError> {
        if let Some(domain) = purpose.domain
            && self.0.exception_hosts.reached(domain).await.is_some()
        {
            return Err(SecretError::InputRefused {
                placeholder: purpose.placeholder.to_owned(),
                reference: reference.clone(),
                domain: domain.to_owned(),
                reason: SecretInputRefusedReason::InvalidCertificateException,
            });
        }
        if !purpose.domains.is_empty()
            && !purpose
                .domain
                .is_some_and(|domain| allowed(domain, purpose.domains))
        {
            return Err(SecretError::DomainNotAllowed {
                placeholder: purpose.placeholder.to_owned(),
                reference: reference.clone(),
                domain: purpose.domain.unwrap_or("engine").to_owned(),
                domains: purpose.domains.to_vec(),
            });
        }
        let value = if self.0.each_use.contains(reference.as_str()) {
            self.fetch(reference, purpose).await?
        } else {
            let resolved = Arc::clone(
                self.0
                    .values
                    .lock()
                    .await
                    .entry((
                        reference.as_str().to_owned(),
                        purpose.placeholder.to_owned(),
                    ))
                    .or_default(),
            );
            resolved
                .get_or_try_init(|| self.fetch(reference, purpose))
                .await?
                .clone()
        };
        self.record(reference, purpose).await?;
        self.0.mask.join(purpose.placeholder, &value, Reach::Value);
        let mut held = self.0.held.lock().await;
        held.retain(|held| !(held.placeholder == purpose.placeholder && same(&held.value, &value)));
        held.push(Held {
            placeholder: purpose.placeholder.to_owned(),
            value: value.clone(),
        });
        drop(held);
        Ok(value)
    }

    /// The value `reference` points to, asked of its provider now for
    /// `purpose`, within `timeout.secret`: a provider that answers it is
    /// busy is asked again, after its `Retry-After` or a wait drawn with
    /// full jitter under a cap that doubles from [`FIRST_WAIT`] to
    /// [`LONGEST_WAIT`], while the next ask still falls within it.
    async fn fetch(
        &self,
        reference: &SecretReference,
        purpose: &Purpose<'_>,
    ) -> Result<SecretString, SecretError> {
        let unavailable = |failure: SecretFailure, fix: String| SecretError::Unavailable {
            placeholder: purpose.placeholder.to_owned(),
            reference: reference.clone(),
            failure,
            fix,
        };
        let scheme = reference.provider().as_str();
        let Some(Implementation(provider)) = self.0.secrets.0.providers.get(&reference.provider())
        else {
            return Err(unavailable(
                SecretFailure::NotConfigured,
                format!("This engine has no `{scheme}` provider; use another reference."),
            ));
        };
        if let Err(problem) = provider.check_config() {
            return Err(unavailable(SecretFailure::NotConfigured, problem.fix));
        }
        let clock = self.0.secrets.0.clock;
        let deadline = clock.now() + self.0.timeout;
        let mut attempt: u32 = 0;
        loop {
            attempt = attempt.saturating_add(1);
            let answer = tokio::select! {
                answer = provider.resolve(reference) => answer,
                () = clock.sleep_until(deadline) => {
                    return Err(unavailable(
                        SecretFailure::TimedOut,
                        format!(
                            "The `{scheme}` provider gave no answer within `timeout.secret` ({} ms); check that it is reachable, or raise `timeout.secret`.",
                            self.0.timeout.as_millis()
                        ),
                    ));
                }
            };
            match answer {
                Ok(value) => return Ok(value),
                Err(ProviderFailure::Busy { fix, retry_after }) => {
                    let wait = match retry_after {
                        Some(wait) => wait,
                        None => {
                            full_jitter(attempt, self.0.secrets.0.entropy.lock().await.next_u64())
                        }
                    };
                    if clock.now() + wait >= deadline {
                        return Err(unavailable(SecretFailure::ProviderFailed, fix));
                    }
                    clock.sleep(wait).await;
                }
                Err(failure) => {
                    let (failure, fix) = failure_of(failure, reference);
                    return Err(unavailable(failure, fix));
                }
            }
        }
    }

    /// Every value the Run resolved, with the name it was resolved for, in
    /// the order resolved, which the run's `Mask` replaces.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn held(&self) -> Vec<(String, SecretString)> {
        self.0
            .held
            .lock()
            .await
            .iter()
            .map(|held| (held.placeholder.clone(), held.value.clone()))
            .collect()
    }

    /// The value last resolved for `placeholder`, if any.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn held_for(&self, placeholder: &str) -> Option<SecretString> {
        self.0
            .held
            .lock()
            .await
            .iter()
            .rev()
            .find(|held| held.placeholder == placeholder)
            .map(|held| held.value.clone())
    }

    /// Records the use of `reference` for `purpose` in the Audit log.
    async fn record(
        &self,
        reference: &SecretReference,
        purpose: &Purpose<'_>,
    ) -> Result<(), SecretError> {
        let audit = Arc::clone(&self.0.secrets.0.audit);
        let origin = Origin {
            actor: Actor::Engine,
            surface: None,
            layer: purpose.layer,
            run_id: purpose.run_id.cloned(),
        };
        let details = json!({
            "placeholder": purpose.placeholder,
            "reference": reference.as_str(),
            "domain": purpose.domain,
        });
        let now = self.0.secrets.0.clock.timestamp();
        self.0
            .secrets
            .0
            .owner
            .blocking(move || audit.record(&origin, &SECRET_REFERENCE_USED, details, now))
            .await
            .map(|_| ())
            .map_err(|source| SecretError::Unrecorded {
                placeholder: purpose.placeholder.to_owned(),
                reference: reference.clone(),
                source,
            })
    }
}

impl ProviderFailure {
    /// The same failure, waiting `wait` before it is asked again when it
    /// is [`ProviderFailure::Busy`] and `wait` is some: the `Retry-After`
    /// a translation point read beside the answer it translated.
    #[must_use]
    pub fn retrying_after(self, wait: Option<Duration>) -> Self {
        match (self, wait) {
            (Self::Busy { fix, .. }, Some(wait)) => Self::Busy {
                fix,
                retry_after: Some(wait),
            },
            (failure, _) => failure,
        }
    }
}

/// What a provider's failure on `reference` leads to: its variant and fix.
fn failure_of(failure: ProviderFailure, reference: &SecretReference) -> (SecretFailure, String) {
    let scheme = reference.provider().as_str();
    match failure {
        ProviderFailure::Missing => (
            SecretFailure::Missing,
            format!("Store a value where `{reference}` points, then try again."),
        ),
        ProviderFailure::NotConfigured(problem) => (SecretFailure::NotConfigured, problem.fix),
        ProviderFailure::Refused { fix } | ProviderFailure::Busy { fix, .. } => {
            (SecretFailure::ProviderFailed, fix)
        }
        ProviderFailure::Unrecognized(_) => (
            SecretFailure::ProviderFailed,
            format!("The `{scheme}` provider failed; its logs say why."),
        ),
    }
}

/// The exclusive lock on the file at `path`, made if it is missing, once
/// no other process holds it. Blocking: it runs on the tasks module's
/// blocking policy.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn lock_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(folder) = path.parent() {
        #[expect(
            clippy::disallowed_methods,
            reason = "ADR 0140: a folder created on no counted row yet, on the tasks module's blocking policy; the Shared paths audit moves it onto its counted function"
        )]
        std::fs::create_dir_all(folder)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

/// The first wait's cap before a busy provider is asked again.
const FIRST_WAIT: Duration = Duration::from_millis(200);

/// The longest cap on a wait before a busy provider is asked again.
const LONGEST_WAIT: Duration = Duration::from_secs(5);

/// The wait before the ask after `attempt`, the first being 1, drawn from
/// `draw` with full jitter: any whole millisecond from none to the cap,
/// [`FIRST_WAIT`] doubled at each attempt up to [`LONGEST_WAIT`].
fn full_jitter(attempt: u32, draw: u64) -> Duration {
    let cap = FIRST_WAIT
        .saturating_mul(1_u32 << attempt.saturating_sub(1).min(16))
        .min(LONGEST_WAIT);
    let cap_ms = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(draw % (cap_ms + 1))
}

/// Whether `one` and `other` are the same value, in time that depends only
/// on their lengths.
fn same(one: &SecretString, other: &SecretString) -> bool {
    bool::from(
        one.expose_secret()
            .as_bytes()
            .ct_eq(other.expose_secret().as_bytes()),
    )
}

/// Whether `reference` names the item of the engine's own key-encryption
/// key: one of `run_state.key_reference`'s defaults, which the engine makes
/// where run state first wraps a data key with it.
pub fn is_engine_key(reference: &SecretReference) -> bool {
    ENGINE_KEYS.contains(&reference.as_str())
}

/// The items the engine makes its own key-encryption key in, the defaults of
/// `run_state.key_reference`.
const ENGINE_KEYS: [&str; 2] = [KEY_REFERENCE_IN_KEYCHAIN, KEY_REFERENCE_IN_FILE];

/// A refusal's reason as the wire names it.
fn reason_name(reason: SecretInputRefusedReason) -> &'static str {
    match reason {
        SecretInputRefusedReason::InvalidCertificateException => "invalid_certificate_exception",
    }
}

/// Whether `domain` is one of `domains` or a subdomain of one, ignoring
/// ASCII case.
fn allowed(domain: &str, domains: &[String]) -> bool {
    let domain = domain.to_ascii_lowercase();
    domains.iter().any(|allowed| {
        let allowed = allowed.to_ascii_lowercase();
        domain == allowed
            || domain
                .strip_suffix(allowed.as_str())
                .is_some_and(|start| start.ends_with('.'))
    })
}

/// The fuzz target `secret_reference`'s body (ADR 0067): any text read as
/// a Secret reference, which a person wrote in a settings file or a program
/// passed. A reference reads back as its own text, parses again to itself,
/// and splits into its provider and rest; it panics only on a bug.
pub fn fuzz_secret_reference(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(reference) = SecretReference::parse(text) else {
        return;
    };
    assert_eq!(
        reference.to_string(),
        text,
        "a reference reads back as another text"
    );
    assert_eq!(
        SecretReference::parse(reference.as_str()).as_ref(),
        Ok(&reference),
        "a reference's own text parses to another"
    );
    let scheme = reference.provider().as_str();
    assert!(
        text.starts_with(scheme) && text.ends_with(reference.rest()),
        "a reference splits into another provider and rest"
    );
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod secret_reference {
    goodfetch_test_support::fuzz_replay!(
        crate::secrets::fuzz_secret_reference,
        "fuzz/secret_reference"
    );
}
