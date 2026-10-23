//! The Secret-provider driver (ADRs 0015, 0016, 0092 and 0129): what every
//! Secret provider does the same way, written once above them. A value is
//! resolved at the moment of use and never earlier, only for a domain its
//! binding allows, within `timeout.secret`; each resolution is recorded in
//! the Audit log with its placeholder, reference and receiving domain,
//! never the value; the value is held for the rest of the run as a
//! `SecretString`, zeroized once the last handle drops, where the run's
//! `Mask` finds it; and a failure is one of [`SecretFailure`]'s variants,
//! each leading to `secret_unavailable` with its fix.
//!
//! A provider fills only two primitives, [`SecretProvider::resolve`] and
//! [`SecretProvider::check_config`]. This module carries the providers
//! every engine needs (ADR 0015, spec #564): the environment's,
//! `env:NAME`; the OS keychain's, `keychain:service/account`; and the
//! encrypted local file's, `secretfile:name`, where no keychain exists,
//! which is also where `run_state.key_reference`'s default falls back to
//! ([`host_defaults`]). Each is a translation point (ADR 0127): what its
//! party answers becomes a [`ProviderFailure`] there and nowhere else.

mod environment;
mod keychain;
mod secretfile;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::audit::{Actor, SECRET_REFERENCE_USED};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_protocol::secrets::{
    KEY_REFERENCE_IN_FILE, RUN_STATE_KEY_REFERENCE, SecretFailure, SecretInputRefusedReason,
};
use goodfetch_protocol::settings::SettingsLayer;
use jiff::Timestamp;
use secrecy::SecretString;
use serde_json::json;

pub use keychain::Keychain;

use crate::audit::{AuditError, AuditLog, Origin};
use crate::clock::Clock;
use crate::home::Home;
use crate::known_certificates::KeyHash;
use crate::settings::HostDefaults;
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
    /// It failed in a way its translation point does not know (ADR 0127),
    /// logged at `warn` without its text: `provider_failed`.
    Unrecognized(PartyText),
    /// The caller's resolver is missing, threw or gave no answer.
    ResolverFailed,
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
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl std::fmt::Debug for PartyText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "PartyText(<{} bytes, redacted>)", self.0.len())
    }
}

/// The providers every engine carries beside the ones a Product adds: the
/// environment map `env:` reads, the OS keychain `keychain:` reads, and the
/// Goodfetch home whose encrypted local files `secretfile:` reads.
#[derive(Clone, Debug)]
pub struct Carried {
    /// The engine's environment map (ADR 0083).
    pub environment: BTreeMap<OsString, OsString>,
    /// The OS keychain, or none.
    pub keychain: Keychain,
    /// The Goodfetch home.
    pub home: Home,
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
    audit: Arc<AuditLog>,
    owner: Arc<TaskOwner>,
    clock: Clock,
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
        let mut providers = BTreeMap::new();
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
                carried.home.secret_files(),
                Arc::clone(&owner),
            ))),
        );
        Self(Arc::new(Shared {
            providers,
            audit,
            owner,
            clock,
        }))
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

    /// The Audit log every resolution is recorded in.
    pub(crate) fn audit(&self) -> &AuditLog {
        &self.0.audit
    }

    /// What one Run resolves through, each provider bounded by `timeout`,
    /// its `timeout.secret`, holding every value it resolved until its last
    /// handle drops.
    pub fn for_run(&self, timeout: Duration) -> RunSecrets {
        RunSecrets(Arc::new(ForRun {
            secrets: self.clone(),
            timeout,
            held: tokio::sync::Mutex::new(Vec::new()),
            exception_hosts: ExceptionHosts::default(),
        }))
    }
}

/// The hosts one Run reached through an invalid-certificate exception
/// (ADR 0042), with the key each presented: the engine's client sends them
/// no Session cookie and the driver resolves no value for them. Hosts are
/// held as the URL parser leaves them, without a trailing dot. Clones share
/// it.
#[derive(Clone, Debug, Default)]
pub struct ExceptionHosts(Arc<tokio::sync::Mutex<BTreeMap<String, KeyHash>>>);

impl ExceptionHosts {
    /// The key `host` presented, when the Run reached it through an
    /// exception.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn reached(&self, host: &str) -> Option<KeyHash> {
        self.0.lock().await.get(&host_key(host)).copied()
    }

    /// Adds `host`, reached through an exception with `key`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the host out.
    pub(crate) async fn add(&self, host: &str, key: KeyHash) {
        self.0.lock().await.insert(host_key(host), key);
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
}

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

    /// The value `reference` points to, resolved now for `purpose`: refused
    /// for a domain its binding does not allow, bounded by `timeout.secret`,
    /// recorded in the Audit log and held for the rest of the Run.
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
        let answer = tokio::select! {
            answer = provider.resolve(reference) => answer,
            () = self.0.secrets.0.clock.sleep(self.0.timeout) => {
                return Err(unavailable(
                    SecretFailure::TimedOut,
                    format!(
                        "The `{scheme}` provider gave no answer within `timeout.secret` ({} ms); check that it is reachable, or raise `timeout.secret`.",
                        self.0.timeout.as_millis()
                    ),
                ));
            }
        };
        let value = answer.map_err(|failure| {
            let (failure, fix) = match failure {
                ProviderFailure::Missing => (
                    SecretFailure::Missing,
                    format!("Store a value where `{reference}` points, then try again."),
                ),
                ProviderFailure::NotConfigured(problem) => {
                    (SecretFailure::NotConfigured, problem.fix)
                }
                ProviderFailure::Refused { fix } => (SecretFailure::ProviderFailed, fix),
                ProviderFailure::Unrecognized(_) => (
                    SecretFailure::ProviderFailed,
                    format!("The `{scheme}` provider failed; its logs say why."),
                ),
                ProviderFailure::ResolverFailed => (
                    SecretFailure::ResolverFailed,
                    "Answer the reference from the program's `resolveSecret`.".to_owned(),
                ),
            };
            unavailable(failure, fix)
        })?;
        self.record(reference, purpose).await?;
        self.0.held.lock().await.push(Held {
            placeholder: purpose.placeholder.to_owned(),
            value: value.clone(),
        });
        Ok(value)
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
        let now =
            Timestamp::try_from(self.0.secrets.0.clock.wall()).unwrap_or(Timestamp::UNIX_EPOCH);
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
