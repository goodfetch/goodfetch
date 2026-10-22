//! Trust on first use (ADR 0042): a domain listed in
//! `tls.accept_invalid_certs` without a pin accepts its host's first invalid
//! certificate, pins the SHA-256 of its public key in the person's
//! known-certificates store and records the first pin in the Audit log.
//! From then on, in every run that lists the domain, the host passes only
//! with that key, whatever the default roots or `tls.trusted_cas` would
//! accept; another key, or a pin row the engine cannot read, stops it as a
//! changed key. A listed host with no pin whose certificate verifies passes
//! as any other, with no exception.
//!
//! The decision is taken once the handshake is done and before the request
//! is written, so nothing is sent to a host it stops. Each host reached
//! through an exception joins the Run's [`ExceptionHosts`], which the
//! engine's client and the Secret-provider driver read: no Session cookie
//! and no Secret goes to it.

use std::collections::BTreeSet;
use std::sync::Arc;

use goodfetch_protocol::audit::{Actor, CERTIFICATE_PINNED};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::tls::CertificateFailure;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::{InvalidDomain, covered, domain_key};
use crate::audit::{AuditLog, Origin};
use crate::clock::Clock;
use crate::known_certificates::{Found, KeyHash, KnownCertificates, Pinned};
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

/// The kind of exception a host was reached through, as Provenance marks
/// what was fetched under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExceptionKind {
    /// Its domain was listed without a pin: the key pinned on first use.
    TrustOnFirstUse,
}

/// The exception a response was fetched under, which its Provenance
/// carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionUsed {
    /// The kind of exception.
    pub kind: ExceptionKind,
    /// The SHA-256 of the public key of the certificate it accepted.
    pub certificate: KeyHash,
    /// Whether this connection pinned the key.
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

/// What a listed host's certificate showed in the handshake: its key, and
/// how it failed verification, if it did.
#[derive(Clone, Debug)]
pub(crate) struct Presented {
    pub(crate) key: KeyHash,
    pub(crate) failure: Option<CertificateFailure>,
}

/// The exceptions of `tls.accept_invalid_certs` a Session's connections
/// take, and where they pin. None by default.
#[derive(Clone, Debug, Default)]
pub struct CertificateExceptions {
    /// The domains listed for trust on first use, as the URL parser leaves
    /// a host.
    trust_on_first_use: BTreeSet<String>,
    book: Option<PinBook>,
    reached: ExceptionHosts,
}

impl CertificateExceptions {
    /// The exceptions `setting`, the resolved value of
    /// `tls.accept_invalid_certs`, makes: each domain listed with `null`
    /// takes trust on first use, pinning in `book`; each host reached
    /// through one joins `reached`. A domain listed with a pin or `any`
    /// makes no exception here (#629), so its host verifies as any other.
    ///
    /// # Errors
    ///
    /// [`InvalidDomain`] when an entry names no host.
    pub fn listed(
        setting: &Value,
        book: PinBook,
        reached: ExceptionHosts,
    ) -> Result<Self, InvalidDomain> {
        let mut trust_on_first_use = BTreeSet::new();
        for (domain, value) in setting.as_object().into_iter().flatten() {
            if value.is_null() {
                trust_on_first_use.insert(domain_key(domain).ok_or_else(|| InvalidDomain {
                    domain: domain.clone(),
                })?);
            }
        }
        Ok(Self {
            trust_on_first_use,
            book: Some(book),
            reached,
        })
    }

    /// Whether `host`'s domain is listed for trust on first use.
    pub(crate) fn covers(&self, host: &str) -> bool {
        self.book.is_some() && covered(host, |domain| self.trust_on_first_use.contains(domain))
    }

    /// The exception `host`, which [`Self::covers`], is reached through
    /// once it presented `presented`: none when its certificate verified
    /// and it has no pin.
    ///
    /// # Errors
    ///
    /// [`CertificateChanged`] when it is pinned to another key, or its pin
    /// row cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the store's write leaves the pin in place
    /// and may lose its Audit log entry.
    pub(crate) async fn admit(
        &self,
        host: &str,
        presented: Presented,
    ) -> Result<Option<ExceptionUsed>, CertificateChanged> {
        let Some(book) = &self.book else {
            return Ok(None);
        };
        let host = domain_key(host).unwrap_or_else(|| host.to_owned());
        let key = presented.key;
        let mut first_pin = false;
        let found = match book.find(&host).await {
            Found::Unpinned => match presented.failure {
                None => return Ok(None),
                Some(failure) => match book.pin(&host, &key, &failure).await {
                    Some(Pinned::First) => {
                        first_pin = true;
                        Found::Pinned(key)
                    }
                    Some(Pinned::Already(found)) => found,
                    None => Found::Unreadable,
                },
            },
            found => found,
        };
        match found {
            Found::Pinned(pinned) if pinned == key => {
                self.reached.add(&host, key).await;
                Ok(Some(ExceptionUsed {
                    kind: ExceptionKind::TrustOnFirstUse,
                    certificate: key,
                    first_pin,
                }))
            }
            Found::Pinned(pinned) => Err(CertificateChanged {
                host,
                pinned: Some(pinned),
                presented: key,
            }),
            Found::Unpinned | Found::Unreadable => Err(CertificateChanged {
                host,
                pinned: None,
                presented: key,
            }),
        }
    }
}

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
        let now = Timestamp::try_from(self.clock.wall()).unwrap_or(Timestamp::UNIX_EPOCH);
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
            let audit = Arc::clone(&self.audit);
            let origin = Origin {
                actor: Actor::Engine,
                surface: None,
                layer: None,
                run_id: self.run_id.clone(),
            };
            let details = json!({
                "host": host,
                "spki_sha256": key.to_string(),
                "failure": failure.as_str(),
            });
            if let Err(error) = self
                .owner
                .blocking(move || audit.record(&origin, &CERTIFICATE_PINNED, details, now))
                .await
            {
                tracing::warn!(%error, "the Audit log cannot record a first pin");
            }
        }
        Some(pinned)
    }
}
