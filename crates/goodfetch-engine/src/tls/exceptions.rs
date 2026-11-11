//! Trust on first use (ADR 0042): a domain listed in
//! `tls.accept_invalid_certs` without a pin accepts its host's first invalid
//! certificate, pins the SHA-256 of its public key in the person's
//! known-certificates store and records the first pin in the Audit log.
//! From then on, in every run that lists the domain, the host passes only
//! with that key, whatever the default roots or `tls.trusted_cas` would
//! accept; another key, or a pin row the engine cannot read, stops it with
//! `certificate_changed` for the rest of the run, and the key it presented
//! waits in the store until a person accepts it (`certs accept`). A listed
//! host with no pin whose certificate verifies passes as any other, with no
//! exception.
//!
//! A domain listed with `any`, for a test fixture that regenerates its
//! certificate, lets every certificate of a host with no pin through and
//! pins none, recorded in the Audit log once per run and host; a pinned host
//! under it still passes only with its pin, which only `certs forget`
//! removes.
//!
//! The decision is taken once the handshake is done and before the request
//! is written, so nothing is sent to a host it stops. Each host reached
//! through an exception joins the Run's [`ExceptionHosts`], which the
//! engine's client and the Secret-provider driver read: no Session cookie
//! and no Secret goes to it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_protocol::audit::{Actor, CERTIFICATE_ANY_USED, CERTIFICATE_PINNED};
use goodfetch_protocol::catalog::SharedTypeValue;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::tls::{CERTIFICATE_CHANGED, CertificateChangedDetails, CertificateFailure};
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::InvalidDomain;
use crate::audit::{AuditLog, Origin};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::host_name::{self, ListedDomains};
use crate::known_certificates::{ExceptionKind, Found, KeyHash, KnownCertificates, Pinned};
use crate::secrets::ExceptionHosts;
use crate::tasks::TaskOwner;

/// Where one person's pins are kept, and where a first pin is recorded.
#[derive(Clone, Debug)]
pub struct PinBook {
    /// The user's known-certificates store.
    pub store: KnownCertificates,
    /// The person whose partition holds the pins: the OS user until
    /// accounts exist.
    pub person: String,
    /// The Run whose connections pin, if any.
    pub run_id: Option<RunId>,
    /// The user's Audit log, where each first pin is recorded.
    pub audit: Arc<AuditLog>,
    /// Where the store's and the log's blocking calls run.
    pub owner: Arc<TaskOwner>,
    /// What a first pin is dated by.
    pub clock: Clock,
}

/// The exception a response was fetched under, which its Provenance
/// carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionUsed {
    /// The kind of exception.
    pub kind: ExceptionKind,
    /// The SHA-256 of the public key of the certificate it accepted.
    pub certificate: KeyHash,
    /// Whether the admission this answers pinned the key: a transport's later
    /// connection through the exception it kept answers the value of the
    /// admission that kept it.
    pub first_pin: bool,
}

/// A pinned host that presented another key, or whose pin row the engine
/// cannot read: the request ends before anything is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateChanged {
    /// The host, as the URL parser leaves it.
    pub host: String,
    /// The key pinned for it; none when its row cannot be read.
    pub pinned: Option<KeyHash>,
    /// The key it presented.
    pub presented: KeyHash,
}

impl CertificateChanged {
    /// What `certificate_changed` details: the host, both hashes and what
    /// a person does about it.
    pub fn details(&self) -> CertificateChangedDetails {
        let host = &self.host;
        CertificateChangedDetails {
            host: host.clone(),
            pinned_spki_sha256: self.pinned.map(|key| key.to_string()),
            presented_spki_sha256: self.presented.to_string(),
            fix: format!(
                "A person accepts the new key of {host} with `certs accept {host}` in a terminal or the Kennel, or removes its pin with `certs forget {host}`; no agent, Recipe or `--json` command can."
            ),
        }
    }

    /// The failure the call that waited for the request answers with.
    pub fn error(&self) -> CallError {
        CallError::new(&CERTIFICATE_CHANGED, self.to_string(), &self.details())
    }
}

impl std::fmt::Display for CertificateChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.pinned {
            Some(pinned) => write!(
                formatter,
                "{} presented the key {}, not its pin {pinned}",
                self.host, self.presented
            ),
            None => write!(
                formatter,
                "the pin of {} cannot be read, and it presented the key {}",
                self.host, self.presented
            ),
        }
    }
}

impl std::error::Error for CertificateChanged {}

/// What a listed host's certificate showed in the handshake: its key, and
/// how it failed verification, if it did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presented {
    /// The SHA-256 of its public key.
    pub key: KeyHash,
    /// How it failed verification; none when it verified.
    pub failure: Option<CertificateFailure>,
}

/// What a domain's entry in `tls.accept_invalid_certs` makes of its hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    /// `null`: the first invalid certificate is pinned.
    TrustOnFirstUse,
    /// `any`: every certificate passes, none pinned.
    Any,
}

/// What one Run's connections learnt of its listed hosts: those stopped as
/// a changed key, and those whose use of `any` the Audit log holds.
#[derive(Debug, Default)]
struct RunState {
    stopped: BTreeMap<String, CertificateChanged>,
    any_recorded: BTreeSet<String>,
}

/// The exceptions of `tls.accept_invalid_certs` a Run's connections take,
/// and where they pin. None by default. Clones share what the Run learnt:
/// a host stopped as a changed key stays stopped for the rest of the Run,
/// and its use of `any` is recorded once.
#[derive(Clone, Debug, Default)]
pub struct CertificateExceptions {
    /// Each listed domain's entry, by the domain as the URL parser leaves
    /// a host.
    listed: ListedDomains<Entry>,
    book: Option<PinBook>,
    reached: ExceptionHosts,
    run: Arc<Mutex<RunState>>,
}

impl CertificateExceptions {
    /// The exceptions `setting`, the resolved value of
    /// `tls.accept_invalid_certs`, makes: each domain listed with `null`
    /// takes trust on first use, pinning in `book`, and each listed with
    /// `any` accepts every certificate of a host with no pin; each host
    /// reached through one joins `reached`. A domain listed with a pin
    /// makes no exception here, so its host verifies as any other.
    ///
    /// # Errors
    ///
    /// [`InvalidDomain`] when an entry names no host.
    pub fn listed(
        setting: &Value,
        book: PinBook,
        reached: ExceptionHosts,
    ) -> Result<Self, InvalidDomain> {
        let mut listed = ListedDomains::new();
        for (domain, value) in setting.as_object().into_iter().flatten() {
            let entry = match value {
                Value::Null => Entry::TrustOnFirstUse,
                Value::String(text) if text == ANY => Entry::Any,
                _ => continue,
            };
            listed
                .insert(domain, entry)
                .map_err(|host_name::NoHost(domain)| InvalidDomain { domain })?;
        }
        Ok(Self {
            listed,
            book: Some(book),
            reached,
            run: Arc::default(),
        })
    }

    /// The entry of `host`'s most specific listed domain.
    fn entry_of(&self, host: &str) -> Option<Entry> {
        self.listed.most_specific(host).map(|(_, entry)| *entry)
    }

    /// The domains listed for trust on first use or `any`, as the URL
    /// parser leaves a host: the hosts the egress relay terminates are
    /// these and their subdomains.
    pub fn domains(&self) -> Vec<String> {
        if self.book.is_none() {
            return Vec::new();
        }
        self.listed
            .iter()
            .map(|(domain, _)| domain.to_owned())
            .collect()
    }

    /// Whether `host`'s domain is listed for trust on first use or `any`.
    pub fn covers(&self, host: &str) -> bool {
        self.book.is_some() && self.entry_of(host).is_some()
    }

    /// The changed key that stopped `host` earlier in the Run, if any: no
    /// later request reaches it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn stopped(&self, host: &str) -> Option<CertificateChanged> {
        let host = host_name::lookup_key(host);
        self.run.lock().await.stopped.get(&host).cloned()
    }

    /// Whether `host`, which [`Self::covers`], has a pin, or a row or a
    /// store the engine cannot read: then it passes only through the
    /// exception, whatever its certificate.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn pinned(&self, host: &str) -> bool {
        let Some(book) = &self.book else {
            return false;
        };
        let host = host_name::lookup_key(host);
        book.find(&host).await != Found::Unpinned
    }

    /// The exception `host`, which [`Self::covers`], is reached through
    /// once it presented `presented`: none when its certificate verified
    /// and it has no pin.
    ///
    /// # Errors
    ///
    /// [`CertificateChanged`] when it is pinned to another key, its pin row
    /// cannot be read, or it was stopped earlier in the Run; the key it
    /// presented then waits in the store for a person to accept it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the store's write leaves the pin in place
    /// and may lose its Audit log entry.
    pub async fn admit(
        &self,
        host: &str,
        presented: Presented,
    ) -> Result<Option<ExceptionUsed>, CertificateChanged> {
        let Some(book) = &self.book else {
            return Ok(None);
        };
        let host = host_name::lookup_key(host);
        if let Some(stopped) = self.stopped(&host).await {
            return Err(stopped);
        }
        let entry = self.entry_of(&host).unwrap_or(Entry::TrustOnFirstUse);
        let key = presented.key;
        let mut first_pin = false;
        let found = match (book.find(&host).await, presented.failure) {
            (Found::Unpinned, None) => return Ok(None),
            (Found::Unpinned, Some(failure)) if entry == Entry::Any => {
                self.record_any(book, &host, &key, &failure).await;
                self.reached.add(&host, key, ExceptionKind::Any).await;
                return Ok(Some(ExceptionUsed {
                    kind: ExceptionKind::Any,
                    certificate: key,
                    first_pin: false,
                }));
            }
            (Found::Unpinned, Some(failure)) => match book.pin(&host, &key, &failure).await {
                Some(Pinned::First) => {
                    first_pin = true;
                    Found::Pinned(key)
                }
                Some(Pinned::Already(found)) => found,
                None => Found::Unreadable,
            },
            (found, _) => found,
        };
        match found {
            Found::Pinned(pinned) if pinned == key => {
                self.reached
                    .add(&host, key, ExceptionKind::TrustOnFirstUse)
                    .await;
                Ok(Some(ExceptionUsed {
                    kind: ExceptionKind::TrustOnFirstUse,
                    certificate: key,
                    first_pin,
                }))
            }
            Found::Pinned(pinned) => Err(self
                .stop(
                    book,
                    CertificateChanged {
                        host,
                        pinned: Some(pinned),
                        presented: key,
                    },
                )
                .await),
            Found::Unpinned | Found::Unreadable => Err(self
                .stop(
                    book,
                    CertificateChanged {
                        host,
                        pinned: None,
                        presented: key,
                    },
                )
                .await),
        }
    }

    /// Stops `changed`'s host for the rest of the Run, and keeps the key it
    /// presented in the store, waiting for a person to accept it.
    async fn stop(&self, book: &PinBook, changed: CertificateChanged) -> CertificateChanged {
        self.run
            .lock()
            .await
            .stopped
            .entry(changed.host.clone())
            .or_insert_with(|| changed.clone());
        book.record_change(&changed.host, &changed.presented).await;
        changed
    }

    /// Records in the Audit log, once per Run and host, that `any` let
    /// `host`'s certificate, which failed with `failure`, through.
    async fn record_any(
        &self,
        book: &PinBook,
        host: &str,
        key: &KeyHash,
        failure: &CertificateFailure,
    ) {
        if !self.run.lock().await.any_recorded.insert(host.to_owned()) {
            return;
        }
        let details = json!({
            "host": host,
            "spki_sha256": key.to_string(),
            "failure": failure.as_str(),
        });
        book.record(&CERTIFICATE_ANY_USED, details).await;
    }
}

/// The value of `tls.accept_invalid_certs` that accepts every certificate.
const ANY: &str = "any";

impl PinBook {
    /// What the store holds for `host`; a store it cannot read reads as an
    /// unreadable row, which stops the host.
    async fn find(&self, host: &str) -> Found {
        let (store, person, host) = (self.store.clone(), self.person.clone(), host.to_owned());
        match self
            .owner
            .blocking(move || store.find(&person, &host))
            .await
        {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(%error, "the known-certificates store cannot be read, so the host stops");
                Found::Unreadable
            }
        }
    }

    /// Pins `key` for `host`, whose certificate failed with `failure`, and
    /// records a first pin in the Audit log; none when the store cannot be
    /// written.
    async fn pin(&self, host: &str, key: &KeyHash, failure: &CertificateFailure) -> Option<Pinned> {
        let now = self.now();
        let (store, person, written, run_id) = (
            self.store.clone(),
            self.person.clone(),
            host.to_owned(),
            self.run_id.clone(),
        );
        let key = *key;
        let pinned = match self
            .owner
            .blocking(move || store.pin(&person, &written, &key, run_id.as_ref(), now))
            .await
        {
            Ok(pinned) => pinned,
            Err(error) => {
                tracing::warn!(%error, "the known-certificates store cannot be written, so the host stops");
                return None;
            }
        };
        if pinned == Pinned::First {
            let details = json!({
                "host": host,
                "spki_sha256": key.to_string(),
                "failure": failure.as_str(),
            });
            self.record(&CERTIFICATE_PINNED, details).await;
        }
        Some(pinned)
    }

    /// Keeps `key`, which stopped `host`, as the change waiting for a
    /// person; the host stays stopped when the store cannot be written.
    async fn record_change(&self, host: &str, key: &KeyHash) {
        let now = self.now();
        let (store, person, host, run_id) = (
            self.store.clone(),
            self.person.clone(),
            host.to_owned(),
            self.run_id.clone(),
        );
        let key = *key;
        if let Err(error) = self
            .owner
            .blocking(move || store.record_change(&person, &host, &key, run_id.as_ref(), now))
            .await
        {
            tracing::warn!(%error, "the known-certificates store cannot keep a changed key");
        }
    }

    /// Records an entry of `kind` with `details` in the Audit log, by the
    /// engine for the Run.
    async fn record(&self, kind: &'static SharedTypeValue, details: Value) {
        let now = self.now();
        let audit = Arc::clone(&self.audit);
        let origin = Origin {
            actor: Actor::Engine,
            surface: None,
            layer: None,
            run_id: self.run_id.clone(),
        };
        if let Err(error) = self
            .owner
            .blocking(move || audit.record(&origin, kind, details, now))
            .await
        {
            tracing::warn!(%error, kind = kind.name, "the Audit log cannot record a certificate exception");
        }
    }

    /// Now, on the book's clock.
    fn now(&self) -> Timestamp {
        self.clock.timestamp()
    }
}
