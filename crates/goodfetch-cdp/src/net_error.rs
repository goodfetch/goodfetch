//! The network errors the browser reports for a request or a navigation
//! (`Network.loadingFailed`'s and `Page.navigate`'s `errorText`), as typed
//! variants (ADR 0127): only here is that text read. The errors the engine
//! tells apart have their own variants: among them a tunnel the proxy
//! refused, any other failure of the proxy's own, and a site's certificate
//! the browser refused; any other the browser names keeps its name,
//! `net::ERR_*`, which a failure reports as the browser's own
//! (`navigation_failed`'s `error`); and a text that names none is
//! unrecognized, logged at `warn`, and reported as `net::ERR_FAILED`,
//! Chromium's generic network error.

use crate::failure::PartyText;

/// A network error of the browser's.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetError {
    /// The browser cancelled the request or navigation: another superseded
    /// it, it was stopped, or its answer has no document.
    Aborted,
    /// A navigation went past the browser's 19 redirects.
    TooManyRedirects,
    /// A body's listed content coding did not decode.
    ContentDecodingFailed,
    /// A zstd body asked for a window past RFC 9659's 8 MB, which the
    /// browser refuses to decode.
    ZstdWindowSizeTooBig,
    /// A navigation's response, an error status with an empty body, which
    /// the browser shows as an error page of its own: the response is still
    /// the navigation's answer.
    HttpResponseCodeFailure,
    /// A blocker failed the request: the interception's Filter lists, or
    /// the page's own.
    BlockedByClient,
    /// The browser's generic network error, which it also gives a request
    /// the interception failed.
    Failed,
    /// The connection closed with no response.
    EmptyResponse,
    /// The site's name has no address.
    NameNotResolved,
    /// Every address of the site refused the connection.
    ConnectionRefused,
    /// The connection to the site timed out.
    ConnectionTimedOut,
    /// Any other failure to connect to the site.
    ConnectionFailed,
    /// The proxy answered the tunnel's `CONNECT` with a refusal other than
    /// its 407, which the browser reports as no more than this.
    TunnelRefused,
    /// Any other failure of the proxy's own, kept by its `net::ERR_*`
    /// name: the proxy could not be reached, its TLS failed, it refused
    /// the SOCKS handshake, or no proxy the browser supports was given.
    Proxy(String),
    /// The browser refused the site's certificate.
    Certificate(CertificateError),
    /// Any other error the browser names, kept by its `net::ERR_*` name.
    Other(String),
    /// A text that names no network error, kept for diagnosis.
    Unrecognized(PartyText),
}

/// Why the browser refused a site's certificate. It names an expired
/// certificate and one not valid yet alike, and an unknown issuer and a
/// self-signed certificate alike.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CertificateError {
    /// It names another host.
    NameMismatch,
    /// No trusted authority issued it.
    UnknownIssuer,
    /// Its issuer revoked it.
    Revoked,
    /// Any other refusal, kept by its `net::ERR_*` name.
    Other(String),
}

impl CertificateError {
    /// The refusal's code: its name past `net::ERR_CERT_`, or past
    /// `net::ERR_` for a `CERTIFICATE_` one, in lowercase, such as
    /// `date_invalid`.
    #[must_use]
    pub fn code(&self) -> String {
        let name = NetError::Certificate(self.clone()).name().to_owned();
        let rest = name.strip_prefix(PREFIX).unwrap_or(&name);
        rest.strip_prefix(CERT).unwrap_or(rest).to_ascii_lowercase()
    }
}

/// The prefix of every network error's name.
const PREFIX: &str = "net::ERR_";

/// The suffix the browser adds to a network error its debugging client
/// caused, such as the interception's block: no blocker in a person's
/// browser gives it, so the error is named without it.
const INSPECTOR: &str = ".Inspector";

/// The names of the errors with their own variant.
const NAMED: [(&str, NetError); 16] = [
    ("net::ERR_ABORTED", NetError::Aborted),
    ("net::ERR_TOO_MANY_REDIRECTS", NetError::TooManyRedirects),
    (
        "net::ERR_CONTENT_DECODING_FAILED",
        NetError::ContentDecodingFailed,
    ),
    (
        "net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG",
        NetError::ZstdWindowSizeTooBig,
    ),
    (
        "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
        NetError::HttpResponseCodeFailure,
    ),
    ("net::ERR_BLOCKED_BY_CLIENT", NetError::BlockedByClient),
    (FAILED, NetError::Failed),
    ("net::ERR_EMPTY_RESPONSE", NetError::EmptyResponse),
    ("net::ERR_NAME_NOT_RESOLVED", NetError::NameNotResolved),
    ("net::ERR_CONNECTION_REFUSED", NetError::ConnectionRefused),
    (
        "net::ERR_CONNECTION_TIMED_OUT",
        NetError::ConnectionTimedOut,
    ),
    ("net::ERR_CONNECTION_FAILED", NetError::ConnectionFailed),
    ("net::ERR_TUNNEL_CONNECTION_FAILED", NetError::TunnelRefused),
    (
        "net::ERR_CERT_COMMON_NAME_INVALID",
        NetError::Certificate(CertificateError::NameMismatch),
    ),
    (
        "net::ERR_CERT_AUTHORITY_INVALID",
        NetError::Certificate(CertificateError::UnknownIssuer),
    ),
    (
        "net::ERR_CERT_REVOKED",
        NetError::Certificate(CertificateError::Revoked),
    ),
];

/// The name of the browser's generic network error, which an unrecognized
/// error is reported with too.
const FAILED: &str = "net::ERR_FAILED";

/// The codes, past `net::ERR_`, of the failures that are the proxy's own:
/// one it gives when the proxy cannot be reached, its TLS fails, or it
/// answers the tunnel's `CONNECT` or the SOCKS handshake with a refusal.
const PROXY_CODES: [&str; 6] = [
    "PROXY_",
    "TUNNEL_",
    "SOCKS_",
    "HTTPS_PROXY_",
    "MANDATORY_PROXY_",
    "NO_SUPPORTED_PROXIES",
];

/// What a certificate's refusal's code, past `net::ERR_`, starts with
/// when the browser does not name it `CERT_`.
const CERTIFICATE: &str = "CERTIFICATE_";

/// What a certificate's refusal's code, past `net::ERR_`, starts with.
const CERT: &str = "CERT_";

impl NetError {
    /// The network error `text`, the browser's `errorText`, reports.
    #[must_use]
    pub fn of(text: &str) -> Self {
        let name = text.strip_suffix(INSPECTOR).unwrap_or(text);
        if let Some((_, error)) = NAMED.iter().find(|(named, _)| *named == name) {
            return error.clone();
        }
        let code = name.strip_prefix(PREFIX).filter(|code| {
            !code.is_empty()
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        });
        let Some(code) = code else {
            tracing::warn!(text, "an unrecognized network error of the browser's");
            return Self::Unrecognized(PartyText::new(text));
        };
        if PROXY_CODES.iter().any(|prefix| code.starts_with(prefix)) {
            Self::Proxy(name.to_owned())
        } else if code.starts_with(CERT) || code.starts_with(CERTIFICATE) {
            Self::Certificate(CertificateError::Other(name.to_owned()))
        } else {
            Self::Other(name.to_owned())
        }
    }

    /// Whether the error is a body's content coding that did not decode, a
    /// zstd window too big for the browser among them: after a response
    /// arrived, a failure of its own, never a cut body (ADR 0118's #699
    /// note).
    #[must_use]
    pub fn fails_to_decode(&self) -> bool {
        matches!(
            self,
            Self::ContentDecodingFailed | Self::ZstdWindowSizeTooBig
        )
    }

    /// Whether the error comes from the proxy: a tunnel it refused, or any
    /// other failure the browser gives only for its proxy.
    #[must_use]
    pub fn from_the_proxy(&self) -> bool {
        matches!(self, Self::TunnelRefused | Self::Proxy(_))
    }

    /// Its name as the browser gives it, `net::ERR_*`, which a failure
    /// reports as the browser's network error.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Proxy(name)
            | Self::Certificate(CertificateError::Other(name))
            | Self::Other(name) => name,
            Self::Unrecognized(_) => FAILED,
            named => NAMED
                .iter()
                .find(|(_, error)| error == named)
                .map_or(FAILED, |(name, _)| name),
        }
    }
}

#[cfg(test)]
mod tests;
