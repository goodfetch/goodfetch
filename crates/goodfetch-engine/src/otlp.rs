//! The OTLP exports' collector (ADRs 0017, 0042, 0050 and 0132): the
//! endpoint `otlp.traces_endpoint` or `otlp.metrics_endpoint` names, an
//! Operator-configured endpoint of the OTLP kind, which each export reaches
//! through the engine's own HTTP client. The egress policy judges it as its
//! class: a collector on loopback or a private network is reached unless
//! `egress.no_private_endpoints` refuses it, each export's addresses judged
//! again before it connects. Its certificate chains to Mozilla's roots or
//! to the authorities `tls.trusted_cas` lists for its own domain, and no
//! exception of `tls.accept_invalid_certs` lets an invalid one through.

use hyper::body::Bytes;
use hyper::{Request, Response};

use crate::clock::Clock;
use crate::egress::{ConfiguredKind, Refused};
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::settings::Resolved;

/// The most bytes of a collector's answer an export reads: an answer
/// carries at most a partial-success message.
pub const ANSWER_LIMIT: usize = 1024 * 1024;

/// An OTLP collector, as the machine's settings configure it.
#[derive(Clone, Debug)]
pub struct Collector {
    endpoint: EndpointSettings,
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
    /// `tls.trusted_cas`.
    pub fn of(settings: &Resolved, clock: Clock) -> Self {
        Self {
            endpoint: EndpointSettings::of(settings, clock),
        }
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
        self.endpoint
            .send_whole(ConfiguredKind::Otlp, request, ANSWER_LIMIT)
            .await
            .map_err(|failure| match failure {
                WholeFailure::Refused(refused) => ExportFailure::Refused(refused),
                WholeFailure::CertificateInvalid(invalid) => {
                    ExportFailure::CertificateInvalid(invalid)
                }
                WholeFailure::Network => ExportFailure::Network,
                WholeFailure::TooLong => ExportFailure::TooLong,
            })
    }
}
