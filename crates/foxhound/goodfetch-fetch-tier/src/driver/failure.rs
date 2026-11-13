//! A fetch's failure as it leaves the fetch tier (ADR 0127): the error its
//! attempt ends with beside the transport's failure that made it, and the
//! Error codes a proxy's own failure ends with (ADRs 0053 and 0017).

use goodfetch_engine::CallError;
use goodfetch_protocol::page::proxy::{
    PROXY_AUTH_FAILED, PROXY_UNAVAILABLE, ProxyAuthFailedDetails, ProxyUnavailableDetails,
};
use url::Url;

use super::TransportFailure;

/// A fetch that failed: the error its attempt ends with, written once
/// here, and the transport's failure when one made it, which says what
/// the hop met to whoever retries or paces on it, so no one reads the
/// error's details back (ADR 0127).
#[derive(Clone, Debug)]
pub struct FetchFailure {
    error: CallError,
    transport: Option<TransportFailure>,
}

impl FetchFailure {
    /// The failure of a hop to `url` that `transport` gave no whole answer
    /// to, through `proxy` when its transport leaves through one.
    #[must_use]
    pub fn of(transport: TransportFailure, url: &str, proxy: Option<&Url>) -> Self {
        Self {
            error: transport.error_through(url, proxy),
            transport: Some(transport),
        }
    }

    /// The error the attempt ends with.
    #[must_use]
    pub fn error(&self) -> &CallError {
        &self.error
    }

    /// The error the attempt ends with, the failure given up.
    #[must_use]
    pub fn into_error(self) -> CallError {
        self.error
    }

    /// The transport's failure, when one made it: none for an answer the
    /// crawl refuses, a deadline or a limit.
    #[must_use]
    pub fn transport(&self) -> Option<&TransportFailure> {
        self.transport.as_ref()
    }
}

impl From<CallError> for FetchFailure {
    fn from(error: CallError) -> Self {
        Self {
            error,
            transport: None,
        }
    }
}

impl TransportFailure {
    /// [`TransportFailure::error`] for a hop that left through `proxy`: a
    /// failure of the proxy's own ends the attempt with the proxy's Error
    /// code, never `navigation_failed` (ADRs 0053 and 0017): a 407 with
    /// `proxy_auth_failed`, not retryable, and a tunnel it refused or a
    /// proxy that could not be reached with `proxy_unavailable`.
    pub(super) fn error_through(&self, url: &str, proxy: Option<&Url>) -> CallError {
        let Some(proxy) = proxy else {
            return self.error(url);
        };
        let server = format!(
            "{}://{}{}",
            proxy.scheme(),
            proxy.host_str().unwrap_or_default(),
            proxy
                .port_or_known_default()
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
        );
        match self {
            Self::ProxyAuthenticationRequired => CallError::new(
                &PROXY_AUTH_FAILED,
                format!(
                    "The proxy {server} answered `407 Proxy Authentication Required` to the fetch of {url}."
                ),
                &ProxyAuthFailedDetails {
                    proxy: server,
                    url: url.to_owned(),
                    username: Some(proxy.username().to_owned()).filter(|name| !name.is_empty()),
                },
            ),
            Self::TunnelRefused | Self::ProxyUnreachable => {
                let error = self.network_error();
                CallError::new(
                    &PROXY_UNAVAILABLE,
                    format!("The proxy {server} failed the fetch of {url}: {error}."),
                    &ProxyUnavailableDetails {
                        proxy: server,
                        url: Some(url.to_owned()),
                        error: Some(error.to_owned()),
                    },
                )
            }
            _ => self.error(url),
        }
    }
}
