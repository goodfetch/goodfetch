//! The OTLP exports' collector (ADRs 0017, 0042, 0050 and 0132): the
//! endpoint `otlp.traces_endpoint` or `otlp.metrics_endpoint` names, an
//! Operator-configured endpoint of the OTLP kind, which each export reaches
//! through the engine's own HTTP client. The egress policy judges it as its
//! class: a collector on loopback or a private network is reached unless
//! `egress.no_private_endpoints` refuses it, each export's addresses judged
//! again before it connects. Its certificate chains to Mozilla's roots or
//! to the authorities `tls.trusted_cas` lists for its own domain, and no
//! exception of `tls.accept_invalid_certs` lets an invalid one through.

use std::path::PathBuf;

use goodfetch_protocol::egress::ConfiguredKind;
use goodfetch_protocol::settings::NETWORK_RESOLVE;
use goodfetch_protocol::tls::TLS_TRUSTED_CAS;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response};

use crate::clock::Clock;
use crate::egress::{EndpointPolicy, Refused};
use crate::happy_eyeballs::Delays;
use crate::http_client::{ConfiguredEndpoint, EndpointFailure, Failure, Resolve};
use crate::settings::Resolved;
use crate::tls::{Trust, read_certificates};

/// The most bytes of a collector's answer an export reads: an answer
/// carries at most a partial-success message.
pub const ANSWER_LIMIT: usize = 1024 * 1024;

/// An OTLP collector, as the machine's settings configure it.
#[derive(Clone, Debug)]
pub struct Collector {
    policy: EndpointPolicy,
    resolve: Resolve,
    authorities: Vec<(String, Vec<PathBuf>)>,
    clock: Clock,
    delays: Delays,
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
        let resolve = settings
            .hosts(NETWORK_RESOLVE.name)
            .into_iter()
            .fold(Resolve::new(), |mapping, (host, addresses)| {
                mapping.with(host, addresses)
            });
        let authorities = settings
            .value(TLS_TRUSTED_CAS.name)
            .as_object()
            .into_iter()
            .flatten()
            .map(|(domain, files)| {
                let files = files
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|file| file.as_str().map(PathBuf::from))
                    .collect();
                (domain.clone(), files)
            })
            .collect();
        Self {
            policy: EndpointPolicy::of(settings),
            resolve,
            authorities,
            clock,
            delays: Delays::of(settings),
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
        let trust = self.trust().await;
        let endpoint = ConfiguredEndpoint {
            kind: ConfiguredKind::Otlp,
            policy: self.policy,
            resolve: &self.resolve,
            trust: &trust,
            clock: self.clock,
            delays: self.delays,
        };
        let mut answer = endpoint
            .send(request.map(Full::new))
            .await
            .map_err(|failure| match failure {
                EndpointFailure::Refused(refused) => ExportFailure::Refused(refused),
                EndpointFailure::Failed(Failure::CertificateInvalid(invalid)) => {
                    ExportFailure::CertificateInvalid(invalid)
                }
                EndpointFailure::Failed(_) => ExportFailure::Network,
            })?;
        let mut body = Vec::new();
        while let Some(bytes) = answer.chunk().await.map_err(|_| ExportFailure::Network)? {
            if body.len() + bytes.len() > ANSWER_LIMIT {
                return Err(ExportFailure::TooLong);
            }
            body.extend_from_slice(&bytes);
        }
        let mut response = Response::new(Bytes::from(body));
        *response.status_mut() = answer.status;
        *response.headers_mut() = answer.headers;
        Ok(response)
    }

    /// The trust of the collector's certificate: the authorities
    /// `tls.trusted_cas` lists for each domain, read now; a file that
    /// cannot be read is logged and trusts nothing.
    async fn trust(&self) -> Trust {
        let mut trust = Trust::default_roots();
        for (domain, files) in &self.authorities {
            let mut authorities = Vec::new();
            for file in files {
                let read = tokio::fs::read(file)
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| read_certificates(&bytes).map_err(|error| error.to_string()));
                match read {
                    Ok(read) => authorities.extend(read),
                    Err(error) => tracing::warn!(
                        %error,
                        file = %file.display(),
                        "a certificate authority of `tls.trusted_cas` cannot be read"
                    ),
                }
            }
            match trust.clone().with_trusted_cas(domain, authorities) {
                Ok(with) => trust = with,
                Err(error) => tracing::warn!(%error, "an entry of `tls.trusted_cas` names no host"),
            }
        }
        trust
    }
}
