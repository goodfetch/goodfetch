//! The per-host decision for a TLS client whose rustls configuration every
//! connection shares (ADR 0042): reqwest in Foxhound's HTTP fetch tier,
//! which hands rustls one configuration per client and no hook between the
//! handshake and the request. Its verifier decides each host from what the
//! client learnt of it before the connection: Mozilla's roots and the
//! domain's `tls.trusted_cas` for a host no exception covers, and for a
//! host `tls.accept_invalid_certs` covers, its pin.
//!
//! A covered host's certificate passes the handshake only when the client
//! already knows the answer: the host has no pin and its certificate
//! verifies, or its exception admitted that very key. Any other handshake
//! stops before anything is sent, carrying the key it was shown; the client
//! then takes the exception's decision ([`SharedTrust::decide`]), which may
//! pin the key on first use, and connects again, or ends the request with
//! `certificate_changed`. A first use therefore costs one handshake more,
//! and no request byte ever goes to a host before its decision.
//!
//! A client presenting a client certificate passes a covered host only when
//! it has no pin and its certificate verifies: the certificate is never
//! sent to a host reached through an exception.
//!
//! A TLS library that verifies outside rustls takes the same decision
//! through [`SharedTrust::verdict`] and [`SharedTrust::decide_presented`]:
//! `wreq`, whose custom verification runs during its handshake (spec #565).

#![expect(
    clippy::disallowed_types,
    reason = "rustls's verifier is synchronous and cannot await: one Mutex guards the hosts' states, read or written in one step with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_protocol::tls::CertificateFailure;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::sign::CertifiedKey;
use rustls::{CertificateError, DigitallySignedStruct, OtherError, SignatureScheme};

use super::exceptions::{CertificateChanged, CertificateExceptions, ExceptionUsed, Presented};
use super::{OnClock, Rejected, Trust, Verifier, client_certificates, failure_of};
use crate::clock::Clock;
use crate::host_name;
use crate::known_certificates::KeyHash;

/// What a [`SharedTrust`]'s client learnt of one covered host.
#[derive(Clone, Debug)]
enum Known {
    /// It has no pin: a certificate that verifies passes, with no
    /// exception.
    Unpinned,
    /// It has a pin, or a row the engine cannot read, whose key the
    /// exception has not admitted yet.
    Pinned,
    /// Its exception admitted this key.
    Admitted(ExceptionUsed),
}

/// The per-host decision of every connection a TLS client opens under one
/// shared rustls configuration: [`Trust`]'s for each host, and the
/// exceptions of `tls.accept_invalid_certs` for those they cover. Clones
/// share what the client learnt of each host.
#[derive(Clone, Debug)]
pub struct SharedTrust {
    trust: Trust,
    exceptions: CertificateExceptions,
    clock: Clock,
    known: Arc<Mutex<BTreeMap<String, Known>>>,
}

/// What the client does once a handshake stopped before its decision.
#[derive(Debug)]
pub enum Decision {
    /// The host's decision is known now: connect again.
    Again,
    /// The host's pinned key changed, or it was stopped earlier in the
    /// Run: nothing goes to it for the rest of the Run.
    Changed(CertificateChanged),
    /// The host passes only through its exception, and the connection
    /// presents a client certificate, which never goes to such a host.
    Excepted,
}

/// How a certificate a host presented fares under a [`SharedTrust`]'s
/// decision, for a TLS library that verifies outside rustls, as `wreq`'s
/// custom verification does during its handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The connection goes on.
    Passes,
    /// It failed verification, and no exception covers its host.
    Rejected(CertificateFailure),
    /// Its host's exception decides first ([`SharedTrust::decide_presented`]):
    /// the handshake stops, and nothing is sent.
    Undecided(Presented),
}

/// The key a covered host presented before the client knew its decision,
/// carried from the verifier through rustls's handshake error.
#[derive(Debug)]
struct Undecided(Presented);

impl std::fmt::Display for Undecided {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("certificate_undecided")
    }
}

impl std::error::Error for Undecided {}

impl SharedTrust {
    /// The decision of `trust`, with `exceptions` for the hosts they cover,
    /// judging a certificate's dates at `clock`'s wall time.
    pub fn new(trust: Trust, exceptions: CertificateExceptions, clock: Clock) -> Self {
        Self {
            trust,
            exceptions,
            clock,
            known: Arc::default(),
        }
    }

    /// The rustls configuration every connection of one client shares,
    /// presenting `client` when a server asks for a client certificate and
    /// offering HTTP/1.1 alone in ALPN, which a transport replaces.
    pub fn client_config(&self, client: Option<Arc<CertifiedKey>>) -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Deciding {
            inner: Verifier::new(&self.trust, &provider),
            shared: self.clone(),
            presents: client.is_some(),
        };
        let builder =
            rustls::ClientConfig::builder_with_details(provider, OnClock::shared(self.clock))
                .with_safe_default_protocol_versions()
                .expect("invariant: ring supports rustls's default protocol versions")
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier));
        let mut config = match client {
            Some(certified) => builder
                .with_client_cert_resolver(Arc::new(client_certificates::Presenting(certified))),
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config
    }

    /// Readies the decision for a connection to `host` before anything is
    /// sent: learns whether a covered host has a pin.
    ///
    /// # Errors
    ///
    /// [`CertificateChanged`] when its changed key stopped it earlier in
    /// the Run.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn before(&self, host: &str) -> Result<(), CertificateChanged> {
        if !self.exceptions.covers(host) {
            return Ok(());
        }
        if let Some(stopped) = self.exceptions.stopped(host).await {
            return Err(stopped);
        }
        let key = host_name::lookup_key(host);
        if self.known().contains_key(&key) {
            return Ok(());
        }
        let known = if self.exceptions.pinned(host).await {
            Known::Pinned
        } else {
            Known::Unpinned
        };
        self.known().entry(key).or_insert(known);
        Ok(())
    }

    /// What the client does once a handshake with `host` ended with
    /// `error`, by a connection that presents a client certificate when
    /// `presents`: none when the handshake did not stop before its
    /// decision.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the store's write leaves the pin in place
    /// and may lose its Audit log entry.
    pub async fn decide(
        &self,
        host: &str,
        error: &rustls::Error,
        presents: bool,
    ) -> Option<Decision> {
        let rustls::Error::InvalidCertificate(CertificateError::Other(OtherError(other))) = error
        else {
            return None;
        };
        let Undecided(presented) = other.downcast_ref::<Undecided>()?;
        Some(
            self.decide_presented(host, presented.clone(), presents)
                .await,
        )
    }

    /// What the client does once a handshake with `host` stopped before
    /// its decision, the host having presented `presented` over a
    /// connection that presents a client certificate when `presents`.
    ///
    /// # Cancel safety
    ///
    /// As [`SharedTrust::decide`].
    pub async fn decide_presented(
        &self,
        host: &str,
        presented: Presented,
        presents: bool,
    ) -> Decision {
        let key = host_name::lookup_key(host);
        match self.exceptions.admit(host, presented).await {
            Err(changed) => Decision::Changed(changed),
            Ok(None) => {
                self.known().insert(key, Known::Unpinned);
                Decision::Again
            }
            Ok(Some(used)) => {
                self.known().insert(key, Known::Admitted(used));
                if presents {
                    Decision::Excepted
                } else {
                    Decision::Again
                }
            }
        }
    }

    /// The verdict on `presented`, what `host` presented in a handshake
    /// over a connection that presents a client certificate when
    /// `presents`, from what the client learnt of the host before the
    /// connection ([`SharedTrust::before`]): the rustls verifier's
    /// decision, for a TLS library that verifies outside rustls.
    pub fn verdict(&self, host: &str, presented: Presented, presents: bool) -> Verdict {
        if !self.exceptions.covers(host) {
            return match presented.failure {
                None => Verdict::Passes,
                Some(failure) => Verdict::Rejected(failure),
            };
        }
        let passes = match self.known().get(&host_name::lookup_key(host)) {
            Some(Known::Unpinned) => presented.failure.is_none(),
            Some(Known::Admitted(used)) => !presents && used.certificate == presented.key,
            Some(Known::Pinned) | None => false,
        };
        if passes {
            Verdict::Passes
        } else {
            Verdict::Undecided(presented)
        }
    }

    /// The exception `host`'s connections pass through: none when no
    /// exception admitted its key.
    pub fn exception(&self, host: &str) -> Option<ExceptionUsed> {
        match self.known().get(&host_name::lookup_key(host)) {
            Some(Known::Admitted(used)) => Some(used.clone()),
            _ => None,
        }
    }

    fn known(&self) -> MutexGuard<'_, BTreeMap<String, Known>> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// How `error`, a handshake's, says the host's certificate failed
/// verification: none when it failed for another reason, or stopped before
/// its decision.
pub fn certificate_failure(error: &rustls::Error) -> Option<CertificateFailure> {
    let rustls::Error::InvalidCertificate(certificate) = error else {
        return None;
    };
    let CertificateError::Other(OtherError(other)) = certificate else {
        return Some(failure_of(certificate));
    };
    if other.downcast_ref::<Undecided>().is_some() {
        return None;
    }
    Some(match other.downcast_ref::<Rejected>() {
        Some(Rejected(failure)) => failure.clone(),
        None => failure_of(certificate),
    })
}

/// The verifier of a [`SharedTrust`]'s configuration.
#[derive(Debug)]
struct Deciding {
    inner: Verifier,
    shared: SharedTrust,
    /// Whether its connections present a client certificate.
    presents: bool,
}

impl ServerCertVerifier for Deciding {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let failure =
            self.inner
                .failure(end_entity, intermediates, server_name, ocsp_response, now)?;
        let host = server_name.to_str();
        let verdict = if self.shared.exceptions.covers(&host) {
            let parsed = ParsedCertificate::try_from(end_entity)?;
            let key = KeyHash::of_public_key(parsed.subject_public_key_info().as_ref());
            self.shared
                .verdict(&host, Presented { key, failure }, self.presents)
        } else {
            failure.map_or(Verdict::Passes, Verdict::Rejected)
        };
        let refusal: Arc<dyn std::error::Error + Send + Sync> = match verdict {
            Verdict::Passes => return Ok(ServerCertVerified::assertion()),
            Verdict::Rejected(failure) => Arc::new(Rejected(failure)),
            Verdict::Undecided(presented) => Arc::new(Undecided(presented)),
        };
        Err(rustls::Error::InvalidCertificate(CertificateError::Other(
            OtherError(refusal),
        )))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}
