//! The OS keychain as a Secret provider (ADR 0015): `keychain:service/account`
//! is the item of that service and account in this OS's keychain: the
//! login keychain on macOS and the Credential Manager on Windows, through
//! keyring-core's stores, and on Linux the Secret Service of the session's
//! D-Bus (`gnome-keyring`, `KWallet`), spoken directly over zbus
//! ([`secret_service`]), each found through the process's own session.
//! It is a translation point (ADR 0127): keyring-core's errors become a
//! [`ProviderFailure`] in [`translate`], the Secret Service's in
//! [`secret_service::translate`], and nowhere else; a store's own text,
//! which may hold what it was given, is never logged.

#[cfg(target_os = "linux")]
pub(super) mod secret_service;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::secret_reference::SecretReference;
use keyring_core::{CredentialStore, Error};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::OnceCell;

use super::{ConfigProblem, PartyText, ProviderFailure, Resolving, SecretProvider};
use crate::tasks::TaskOwner;

/// The OS keychain, or its absence: opened on first use, once per value
/// and its clones, since opening it may connect to a service.
#[derive(Clone)]
pub struct Keychain(Arc<Opening>);

enum Opening {
    /// This OS's keychain, opened on first use by its opener; none where it
    /// does not open.
    Os(OnceCell<Option<Backend>>, Opener),
    /// A keychain chosen for the engine, or none.
    Chosen(Option<Backend>),
}

/// What opens the OS keychain: [`open_os`], or a test's stand-in.
type Opener = Box<dyn Fn() -> Pin<Box<dyn Future<Output = Option<Backend>> + Send>> + Send + Sync>;

/// What reaches a keychain's items.
#[derive(Clone)]
enum Backend {
    /// A keyring-core store, whose calls block.
    Store(Arc<CredentialStore>),
    /// Linux's Secret Service.
    #[cfg(target_os = "linux")]
    SecretService(Arc<secret_service::SecretService>),
}

impl std::fmt::Debug for Keychain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let opening = match &*self.0 {
            Opening::Os(..) => "os",
            Opening::Chosen(Some(_)) => "chosen",
            Opening::Chosen(None) => "absent",
        };
        formatter.debug_tuple("Keychain").field(&opening).finish()
    }
}

impl Keychain {
    /// This OS's keychain, opened on first use.
    pub fn os() -> Self {
        Self(Arc::new(Opening::Os(
            OnceCell::new(),
            Box::new(|| Box::pin(open_os())),
        )))
    }

    /// A keychain opened on first use as this OS's is, by `open`, which
    /// answers the store it opened, or none where it does not open: a
    /// test's view of when, and how often, the OS keychain opens.
    #[cfg(test)]
    pub(crate) fn opened_by(
        open: impl Fn() -> Option<Arc<CredentialStore>> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Opening::Os(
            OnceCell::new(),
            Box::new(move || Box::pin(std::future::ready(open().map(Backend::Store)))),
        )))
    }

    /// No keychain, as on a host without one.
    pub fn absent() -> Self {
        Self(Arc::new(Opening::Chosen(None)))
    }

    /// The keychain `store` holds, such as keyring-core's mock store.
    pub fn of(store: Arc<CredentialStore>) -> Self {
        Self(Arc::new(Opening::Chosen(Some(Backend::Store(store)))))
    }

    /// Whether this host has the keychain: whether it opens, which it does
    /// now if no one has.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the keychain to open on the next call.
    pub async fn available(&self) -> bool {
        self.backend().await.is_some()
    }

    /// Stores `value` as `service`'s `account`, in place of any value it
    /// had, the store's blocking calls running under `owner`.
    ///
    /// # Errors
    ///
    /// The translation of the keychain's failure, `NotConfigured` with no
    /// keychain.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the value stored or not.
    pub async fn store(
        &self,
        service: &str,
        account: &str,
        value: &SecretString,
        owner: &TaskOwner,
    ) -> Result<(), ProviderFailure> {
        match self.backend().await {
            None => Err(no_keychain()),
            Some(Backend::Store(store)) => {
                let (service, account) = (service.to_owned(), account.to_owned());
                let value = value.clone();
                owner
                    .blocking(move || {
                        store
                            .build(&service, &account, None)
                            .and_then(|entry| entry.set_password(value.expose_secret()))
                            .map_err(translate)
                    })
                    .await
            }
            #[cfg(target_os = "linux")]
            Some(Backend::SecretService(service_bus)) => {
                service_bus.store(service, account, value).await
            }
        }
    }

    /// Removes `service`'s `account`, if it is there.
    ///
    /// # Errors
    ///
    /// The translation of the keychain's failure, `NotConfigured` with no
    /// keychain.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the item there or not.
    pub async fn remove(
        &self,
        service: &str,
        account: &str,
        owner: &TaskOwner,
    ) -> Result<(), ProviderFailure> {
        match self.backend().await {
            None => Err(no_keychain()),
            Some(Backend::Store(store)) => {
                let (service, account) = (service.to_owned(), account.to_owned());
                owner
                    .blocking(move || {
                        match store
                            .build(&service, &account, None)
                            .and_then(|entry| entry.delete_credential())
                        {
                            Ok(()) | Err(Error::NoEntry) => Ok(()),
                            Err(error) => Err(translate(error)),
                        }
                    })
                    .await
            }
            #[cfg(target_os = "linux")]
            Some(Backend::SecretService(service_bus)) => service_bus.remove(service, account).await,
        }
    }

    /// The value of `service`'s `account`.
    pub(super) async fn read(
        &self,
        service: String,
        account: String,
        owner: &TaskOwner,
    ) -> Result<SecretString, ProviderFailure> {
        match self.backend().await {
            None => Err(no_keychain()),
            Some(Backend::Store(store)) => {
                owner
                    .blocking(move || {
                        store
                            .build(&service, &account, None)
                            .and_then(|entry| entry.get_password())
                            .map(SecretString::from)
                            .map_err(translate)
                    })
                    .await
            }
            #[cfg(target_os = "linux")]
            Some(Backend::SecretService(service_bus)) => service_bus.read(&service, &account).await,
        }
    }

    async fn backend(&self) -> Option<Backend> {
        match &*self.0 {
            Opening::Os(opened, open) => opened.get_or_init(open).await.clone(),
            Opening::Chosen(backend) => backend.clone(),
        }
    }
}

/// This OS's keychain, or none where it does not open, such as a Linux
/// host whose session has no Secret Service.
#[cfg(target_os = "linux")]
async fn open_os() -> Option<Backend> {
    let opened = secret_service::SecretService::open().await;
    if opened.is_err() {
        tracing::debug!("no OS keychain on this host; the encrypted local file stands in");
    }
    opened
        .ok()
        .map(|service| Backend::SecretService(Arc::new(service)))
}

/// This OS's keychain, or none where its store does not open. A future, as
/// Linux's, which opens over the bus.
#[cfg(not(target_os = "linux"))]
async fn open_os() -> Option<Backend> {
    match os_store() {
        Ok(store) => Some(Backend::Store(store)),
        Err(error) => {
            tracing::debug!(
                failure = kind(&error),
                "no OS keychain on this host; the encrypted local file stands in"
            );
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn os_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(windows)]
fn os_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn os_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Err(Error::NoDefaultStore)
}

/// The `keychain:` provider over one keychain, whose blocking reads run
/// under `owner`.
pub(super) struct KeychainProvider {
    keychain: Keychain,
    owner: Arc<TaskOwner>,
}

impl KeychainProvider {
    pub(super) fn new(keychain: Keychain, owner: Arc<TaskOwner>) -> Self {
        Self { keychain, owner }
    }
}

impl SecretProvider for KeychainProvider {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        let (service, account) = reference
            .rest()
            .split_once('/')
            .map(|(service, account)| (service.to_owned(), account.to_owned()))
            .expect("invariant: a `keychain:` reference parses as service/account");
        Box::pin(self.keychain.read(service, account, &self.owner))
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// No keychain on this host.
fn no_keychain() -> ProviderFailure {
    ProviderFailure::NotConfigured(ConfigProblem {
        fix: "This host has no OS keychain (on Linux, no Secret Service such as gnome-keyring on the session's D-Bus); start one, or use a `secretfile:` reference.".to_owned(),
    })
}

/// What a keyring-core error means for a resolution: the translation table
/// (ADR 0127). An error it does not know is logged at `warn` by its kind,
/// never its text.
pub(super) fn translate(error: Error) -> ProviderFailure {
    match error {
        Error::NoEntry => ProviderFailure::Missing,
        Error::NoStorageAccess(_) => ProviderFailure::Refused {
            fix: "The OS keychain refused access: it is locked, or this program may not read the item; unlock it, then try again.".to_owned(),
        },
        Error::BadEncoding(_) => ProviderFailure::Refused {
            fix: "The keychain item holds no UTF-8 text; store the value as text.".to_owned(),
        },
        Error::Ambiguous(_) => ProviderFailure::Refused {
            fix: "Several keychain items match this service and account; remove all but one."
                .to_owned(),
        },
        Error::TooLong(..) | Error::Invalid(..) => ProviderFailure::Refused {
            fix: "The OS keychain refuses this service or account name; name the item otherwise."
                .to_owned(),
        },
        other => {
            let failure = kind(&other);
            tracing::warn!(
                provider = "keychain",
                failure,
                "an unrecognized failure of the OS keychain"
            );
            ProviderFailure::Unrecognized(PartyText::new(other.to_string()))
        }
    }
}

/// An error's kind, the only part of it ever logged.
fn kind(error: &Error) -> &'static str {
    match error {
        Error::PlatformFailure(_) => "platform_failure",
        Error::NoStorageAccess(_) => "no_storage_access",
        Error::NoEntry => "no_entry",
        Error::BadEncoding(_) => "bad_encoding",
        Error::BadDataFormat(..) => "bad_data_format",
        Error::BadStoreFormat(_) => "bad_store_format",
        Error::TooLong(..) => "too_long",
        Error::Invalid(..) => "invalid",
        Error::Ambiguous(_) => "ambiguous",
        Error::NoDefaultStore => "no_default_store",
        Error::NotSupportedByStore(_) => "not_supported_by_store",
        _ => "unknown",
    }
}
