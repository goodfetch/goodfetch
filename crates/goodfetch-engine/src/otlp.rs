//! The OTLP exports' collector (ADRs 0017, 0042, 0050 and 0132): the
//! endpoint `otlp.traces_endpoint` or `otlp.metrics_endpoint` names, an
//! Operator-configured endpoint of the OTLP kind, which each export reaches
//! through the engine's own HTTP client. The egress policy judges it as its
//! class: a collector on loopback or a private network is reached unless
//! `egress.no_private_endpoints` refuses it, each export's addresses judged
//! again before it connects. Its certificate chains to Mozilla's roots or
//! to the authorities `tls.trusted_cas` lists for its own domain, and no
//! exception of `tls.accept_invalid_certs` lets an invalid one through. A
//! collector whose host `tls.client_certificates` lists presents that
//! certificate once the engine's driver can read its key; an export sent
//! before then presents none.

use std::sync::{Arc, OnceLock};

use hyper::body::Bytes;
use hyper::{Request, Response};

use crate::clock::Clock;
use crate::egress::{ConfiguredKind, Refused};
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::secrets::{DriverSlot, lists_client_certificates};
use crate::settings::Resolved;
use crate::tasks::BlockingThreads;
use crate::tls::ClientCertificates;

/// The most bytes of a collector's answer an export reads: an answer
/// carries at most a partial-success message.
pub const ANSWER_LIMIT: usize = 1024 * 1024;

/// An OTLP collector, as the machine's settings configure it.
#[derive(Clone, Debug)]
pub struct Collector {
    endpoint: EndpointSettings,
    presenting: Option<Presenting>,
}

/// The client certificates a collector presents, built once the engine's
/// driver is in its slot.
#[derive(Clone, Debug)]
struct Presenting {
    settings: Arc<Resolved>,
    keys: DriverSlot,
    endpoint: Arc<OnceLock<EndpointSettings>>,
}

/// Why an export reached no collector, or got no whole answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExportFailure {
    /// The egress policy refused the collector before any connection.
    #[error("{0}")]
    Refused(Refused),
    /// The collector's certificate failed verification, so nothing was
    /// sent.
    #[error("{0}")]
    CertificateInvalid(crate::tls::CertificateInvalid),
    /// The client certificate `tls.client_certificates` lists for the
    /// collector's host cannot be presented, so nothing was sent.
    #[error("{0}")]
    ClientCertificate(crate::tls::ClientCertificateFailure),
    /// No answer, or a connection that broke.
    #[error("the collector did not answer")]
    Network,
    /// The answer was longer than [`ANSWER_LIMIT`].
    #[error("the collector answered more than {ANSWER_LIMIT} bytes")]
    TooLong,
}

impl Collector {
    /// The collector the machine's `settings` reach: their
    /// `egress.no_private_endpoints`, `network.resolve` and
    /// `tls.trusted_cas`, whose files each export reads on `blocking`.
    pub fn of(settings: &Resolved, clock: Clock, blocking: BlockingThreads) -> Self {
        Self {
            endpoint: EndpointSettings::of(settings, clock, blocking),
            presenting: None,
        }
    }

    /// The same, presenting the client certificate the machine's
    /// `settings` list under `tls.client_certificates` for the collector's
    /// host, its key read through the driver `keys` holds once the engine
    /// filled it; until then, it presents none.
    #[must_use]
    pub fn with_client_certificates(self, settings: &Resolved, keys: DriverSlot) -> Self {
        if !lists_client_certificates(settings) {
            return self;
        }
        Self {
            presenting: Some(Presenting {
                settings: Arc::new(settings.clone()),
                keys,
                endpoint: Arc::new(OnceLock::new()),
            }),
            ..self
        }
    }

    /// The endpoint an export reaches the collector through: with its
    /// client certificates once the driver can read their keys.
    fn endpoint(&self) -> &EndpointSettings {
        let Some(presenting) = &self.presenting else {
            return &self.endpoint;
        };
        if let Some(endpoint) = presenting.endpoint.get() {
            return endpoint;
        }
        let Some(keys) = presenting.keys.endpoint_keys(&presenting.settings) else {
            return &self.endpoint;
        };
        presenting.endpoint.get_or_init(|| {
            self.endpoint
                .clone()
                .presenting(ClientCertificates::for_endpoints(
                    &presenting.settings,
                    keys,
                ))
        })
    }

    /// Sends `request`, whose URI is the collector's endpoint, and answers
    /// the collector's whole answer.
    ///
    /// # Errors
    ///
    /// [`ExportFailure`] when the policy refused the collector, its
    /// certificate failed, or it gave no whole answer; the exporter bounds
    /// how long it waits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn send(&self, request: Request<Bytes>) -> Result<Response<Bytes>, ExportFailure> {
        self.endpoint()
            .send_whole(ConfiguredKind::Otlp, request, ANSWER_LIMIT)
            .await
            .map_err(|failure| match failure {
                WholeFailure::Refused(refused) => ExportFailure::Refused(refused),
                WholeFailure::CertificateInvalid(invalid) => {
                    ExportFailure::CertificateInvalid(invalid)
                }
                WholeFailure::ClientCertificate(failure) => {
                    ExportFailure::ClientCertificate(failure)
                }
                WholeFailure::Network => ExportFailure::Network,
                WholeFailure::TooLong => ExportFailure::TooLong,
            })
    }
}
