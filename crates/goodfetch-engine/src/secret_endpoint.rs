//! A Secret provider's endpoint (ADRs 0017, 0042 and 0043; spec #564): the
//! server an HTTP-backed built-in Secret provider, such as Vault's, reads
//! from, an Operator-configured endpoint of the Secret provider kind that
//! each request reaches through the engine's own HTTP client. The egress
//! policy judges it as its class: an endpoint on loopback or a private
//! network is reached unless `egress.no_private_endpoints` refuses it, each
//! request's addresses judged again before it connects. Its certificate
//! chains to Mozilla's roots or to the authorities `tls.trusted_cas` lists
//! for its own domain, and no exception of `tls.accept_invalid_certs` lets
//! an invalid one through. It presents the client certificate
//! `tls.client_certificates` lists for its host (issue 1648). A busy
//! answer's `Retry-After` is read as every `Retry-After` the engine meets
//! is ([`retry_after`], ADR 0118).

use std::time::Duration;

use hyper::body::Bytes;
use hyper::{Request, Response};

use crate::clock::Clock;
use crate::egress::{ConfiguredKind, Refused};
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::retry_after::{self, RetryAfter};
use crate::secrets::ProviderInputs;
use crate::tls::{CertificateInvalid, ClientCertificateFailure, ClientCertificates};

#[cfg(test)]
mod tests;

/// The most bytes of an endpoint's answer a request reads: one secret's
/// fields and metadata.
pub const ANSWER_LIMIT: usize = 1024 * 1024;

/// A Secret provider's endpoint, as the machine's settings let it be
/// reached.
#[derive(Clone, Debug)]
pub struct SecretProviderEndpoint {
    endpoint: EndpointSettings,
    clock: Clock,
}

/// Why a request to a Secret provider's endpoint got no whole answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EndpointFailure {
    /// The egress policy refused the endpoint before any connection.
    #[error("{0}")]
    Refused(Refused),
    /// The endpoint's certificate failed verification, so nothing was
    /// sent.
    #[error("{0}")]
    CertificateInvalid(CertificateInvalid),
    /// The client certificate `tls.client_certificates` lists for the
    /// endpoint's host cannot be presented, so nothing was sent.
    #[error("{0}")]
    ClientCertificate(ClientCertificateFailure),
    /// No answer, or a connection that broke.
    #[error("the Secret provider's endpoint did not answer")]
    Network,
    /// The answer was longer than [`ANSWER_LIMIT`].
    #[error("the Secret provider's endpoint answered more than {ANSWER_LIMIT} bytes")]
    TooLong,
}

impl SecretProviderEndpoint {
    /// The endpoint a Run's `inputs` let a provider reach: their settings'
    /// `egress.no_private_endpoints`, `network.resolve` and
    /// `tls.trusted_cas`, whose files each request reads on their blocking
    /// threads, presenting the client certificate their
    /// `tls.client_certificates` lists for its host, its key read through
    /// their endpoint keys' driver.
    pub fn of(inputs: &ProviderInputs<'_>) -> Self {
        let endpoint = EndpointSettings::of(inputs.settings, inputs.clock, inputs.blocking.clone());
        Self {
            endpoint: match inputs.endpoint_keys {
                Some(keys) => endpoint.presenting(ClientCertificates::for_endpoints(
                    inputs.settings,
                    keys.clone(),
                )),
                None => endpoint,
            },
            clock: inputs.clock,
        }
    }

    /// Sends `request`, whose URI is the endpoint's absolute URL, and
    /// answers the endpoint's whole answer.
    ///
    /// # Errors
    ///
    /// [`EndpointFailure`] when the policy refused the endpoint, its
    /// certificate failed, the client certificate listed for its host
    /// cannot be presented, or it gave no whole answer; the Secret-provider
    /// driver bounds how long it waits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn send(&self, request: Request<Bytes>) -> Result<Response<Bytes>, EndpointFailure> {
        self.endpoint
            .send_whole(ConfiguredKind::SecretProvider, request, ANSWER_LIMIT)
            .await
            .map_err(|failure| match failure {
                WholeFailure::Refused(refused) => EndpointFailure::Refused(refused),
                WholeFailure::CertificateInvalid(invalid) => {
                    EndpointFailure::CertificateInvalid(invalid)
                }
                WholeFailure::ClientCertificate(failure) => {
                    EndpointFailure::ClientCertificate(failure)
                }
                WholeFailure::Network => EndpointFailure::Network,
                WholeFailure::TooLong => EndpointFailure::TooLong,
            })
    }

    /// The wait the `Retry-After` of `answer`, an answer the endpoint just
    /// gave, asks for: a number of seconds, or an HTTP date measured
    /// against the answer's own `Date` when it has one that parses, else
    /// against the engine's clock; none when it has none or it is neither
    /// form, and the driver's backoff waits instead.
    pub fn retry_after(&self, answer: &Response<Bytes>) -> Option<Duration> {
        let header = |name: &str| {
            answer
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let received = self.clock.wall();
        match retry_after::read(header("retry-after")?, header("date"), received) {
            RetryAfter::Until(until) => Some(until.duration_since(received).unwrap_or_default()),
            RetryAfter::NoWait => Some(Duration::ZERO),
            RetryAfter::Unparsable => None,
        }
    }
}
