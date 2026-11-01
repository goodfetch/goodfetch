//! The Clean-up rules' configured sources (ADRs 0043 and 0047): the daily
//! channel's registry, which `clean_up.update = daily` makes a configured
//! destination, and the Filter lists' own URLs, which `upstream` does. Each
//! is an HTTPS GET through the engine's HTTP client with the pinned
//! downloads' user agent, which names no version, and no cookie or
//! identifier; `egress.offline` refuses it before it connects. A source is an
//! Operator-configured endpoint (ADR 0017), so `egress.no_private_endpoints`
//! refuses one that resolves to a private address. A redirect is
//! followed to another `https` URL, at most [`MAX_REDIRECTS`] times, and the
//! request's own headers, such as a registry's token, go only to the host
//! they were meant for. The body is read whole, up to the caller's limit.

use goodfetch_protocol::egress::ConfiguredKind;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::{Request, StatusCode, Uri};

use crate::egress::Refused;
use crate::http_client::{self, ConfiguredEndpoint, EndpointFailure};
use crate::pinned_download::{Fetching, USER_AGENT};
use crate::tls::Trust;

/// The most redirects one GET follows, as a registry's blob that its
/// storage serves needs one.
pub const MAX_REDIRECTS: usize = 5;

/// What a source answered.
#[derive(Clone, Debug)]
pub struct Fetched {
    /// Its status.
    pub status: u16,
    /// Its `WWW-Authenticate` header, when it has one: a registry asks for
    /// its anonymous token by it.
    pub authenticate: Option<String>,
    /// Its body.
    pub body: Vec<u8>,
}

/// Why a source gave nothing.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SourceFailure {
    /// `egress.offline` refused it before it connected.
    #[error("`egress.offline` turns the fetching of the Clean-up rules off")]
    Offline,
    /// The URL is no `https` URL with a host.
    #[error("`{0}` is no https URL")]
    NotHttps(String),
    /// The egress policy refused it before it connected: an address
    /// `egress.no_private_endpoints` refuses.
    #[error("{0}")]
    Refused(Refused),
    /// No answer, a connection that broke, or a certificate that failed.
    #[error("{host} did not answer")]
    Network {
        /// The host.
        host: String,
    },
    /// More redirects than [`MAX_REDIRECTS`], or one with no `Location`.
    #[error("{host} redirected too often")]
    Redirects {
        /// The host.
        host: String,
    },
    /// A body longer than the caller's limit.
    #[error("{host} answered more than {limit} bytes")]
    TooLong {
        /// The host.
        host: String,
        /// The limit.
        limit: usize,
    },
}

/// GETs `url` under `fetching`, with `headers` sent to `url`'s own host,
/// following redirects, and reads the answer's body up to `limit` bytes.
///
/// # Errors
///
/// As [`SourceFailure`] says.
///
/// # Cancel safety
///
/// Dropping the future closes the connection.
pub async fn get(
    url: &str,
    headers: &[(&str, String)],
    fetching: &Fetching,
    limit: usize,
) -> Result<Fetched, SourceFailure> {
    if fetching.offline.is_some() {
        return Err(SourceFailure::Offline);
    }
    let first = parse(url)?;
    let mut uri = first.clone();
    for _ in 0..=MAX_REDIRECTS {
        let own_host = uri.host() == first.host();
        let mut response = send(&uri, if own_host { headers } else { &[] }, fetching).await?;
        let host = uri.host().unwrap_or_default().to_owned();
        if response.status.is_redirection() && response.status != StatusCode::NOT_MODIFIED {
            let location = response
                .headers
                .get(hyper::header::LOCATION)
                .and_then(|location| location.to_str().ok())
                .ok_or_else(|| SourceFailure::Redirects { host: host.clone() })?;
            uri = parse(&joined(&uri, location))?;
            continue;
        }
        let authenticate = response
            .headers
            .get(hyper::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|_| SourceFailure::Network { host: host.clone() })?
        {
            if body.len() + bytes.len() > limit {
                return Err(SourceFailure::TooLong { host, limit });
            }
            body.extend_from_slice(&bytes);
        }
        return Ok(Fetched {
            status: response.status.as_u16(),
            authenticate,
            body,
        });
    }
    Err(SourceFailure::Redirects {
        host: uri.host().unwrap_or_default().to_owned(),
    })
}

/// `url` read as an `https` URL with a host.
fn parse(url: &str) -> Result<Uri, SourceFailure> {
    let uri: Uri = url
        .parse()
        .map_err(|_| SourceFailure::NotHttps(url.to_owned()))?;
    if uri.scheme_str() != Some("https") || uri.host().is_none_or(str::is_empty) {
        return Err(SourceFailure::NotHttps(url.to_owned()));
    }
    Ok(uri)
}

/// `location`, a redirect's target, made absolute against `base`.
fn joined(base: &Uri, location: &str) -> String {
    if location.starts_with("https://") || location.starts_with("http://") {
        return location.to_owned();
    }
    let authority = base.authority().map_or("", |authority| authority.as_str());
    if location.starts_with('/') {
        return format!("https://{authority}{location}");
    }
    let path = base.path();
    let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
    format!("https://{authority}{folder}/{location}")
}

/// Sends one GET of `uri` with `headers`.
async fn send(
    uri: &Uri,
    headers: &[(&str, String)],
    fetching: &Fetching,
) -> Result<http_client::Response, SourceFailure> {
    let host = uri
        .host()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let failed = || SourceFailure::Network {
        host: host.to_owned(),
    };
    let mut request = Request::get(uri.clone()).header(hyper::header::USER_AGENT, USER_AGENT);
    for (name, value) in headers {
        request = request.header(*name, value.as_str());
    }
    let request = request.body(Empty::<Bytes>::new()).map_err(|_| failed())?;
    let trust = Trust::endpoint(&fetching.trusted_certificates);
    // A source is an Operator-configured endpoint (ADRs 0017 and 0047):
    // `egress.no_private_endpoints` judges where it resolves.
    let endpoint = ConfiguredEndpoint {
        kind: ConfiguredKind::CleanUpRulesSource,
        policy: fetching.endpoints,
        resolve: &fetching.resolve,
        trust: &trust,
        clock: fetching.clock,
        delays: fetching.delays,
    };
    endpoint
        .send(request)
        .await
        .map_err(|failure| match failure {
            EndpointFailure::Refused(refused) => SourceFailure::Refused(refused),
            EndpointFailure::Failed(_) => failed(),
        })
}

#[cfg(test)]
mod tests;
