//! The translation point of an upstream proxy's answer to the engine's
//! tunnel request (ADRs 0053 and 0127; spec #564): the engine asks the
//! Session's proxy for a `CONNECT` tunnel to the site and reads its answer
//! here, and nowhere else. A 2xx opens the tunnel. ADR 0053's refusals have
//! their own variants: a 407 is a credential error (`proxy_auth_failed`),
//! and a 429 or a 502 from the proxy itself is a proxy refusal, which the
//! proxy's ceiling learns from (`proxy_unavailable`). Any other answer, or
//! none, is [`ProxyFailure::Unrecognized`], `proxy_unavailable` too, logged
//! at `warn`; the answer's text is a private field no other code reads.

use goodfetch_protocol::page::proxy::{
    PROXY_AUTH_FAILED, PROXY_UNAVAILABLE, ProxyAuthFailedDetails, ProxyUnavailableDetails,
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::call_error::CallError;

use super::UpstreamProxy;

#[cfg(test)]
mod tests;

/// The longest answer head the engine reads from a proxy (STYLE.md: every
/// buffer is bounded).
const HEAD_LIMIT: usize = 16 * 1024;

/// Why the Session's upstream proxy carried no request to the site.
#[derive(Debug, PartialEq, Eq)]
pub enum ProxyFailure {
    /// No connection to the proxy opened.
    Unreachable,
    /// The proxy answered `407 Proxy Authentication Required`.
    AuthenticationRequired,
    /// The proxy answered `429 Too Many Requests` itself (ADR 0053).
    RateLimited,
    /// The proxy answered `502 Bad Gateway` itself (ADR 0053).
    BadGateway,
    /// Any other answer, or none the engine could read.
    Unrecognized(Unrecognized),
}

/// An answer of the proxy the engine does not recognize, kept for the
/// translation point alone.
#[derive(PartialEq, Eq)]
pub struct Unrecognized {
    /// The answer's status line, or what the proxy sent instead.
    answer: String,
}

impl std::fmt::Debug for Unrecognized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Unrecognized")
            .finish_non_exhaustive()
    }
}

impl ProxyFailure {
    /// The failure the call that waited for a request to `url` through
    /// `proxy` answers with: `proxy_auth_failed` for a 407, and
    /// `proxy_unavailable` for every other.
    pub fn error(&self, proxy: &UpstreamProxy, url: &str) -> CallError {
        let proxy = proxy.to_string();
        match self {
            Self::AuthenticationRequired => CallError::new(
                &PROXY_AUTH_FAILED,
                format!("the proxy {proxy} asked for credentials the Session does not have"),
                &ProxyAuthFailedDetails {
                    proxy,
                    url: url.to_owned(),
                    username: None,
                },
            ),
            Self::Unreachable | Self::RateLimited | Self::BadGateway | Self::Unrecognized(_) => {
                let cause = match self {
                    Self::Unreachable => "nothing answers at its address",
                    Self::RateLimited => "it refused the tunnel with a 429",
                    Self::BadGateway => "it refused the tunnel with a 502",
                    _ => "it refused the tunnel",
                };
                CallError::new(
                    &PROXY_UNAVAILABLE,
                    format!("the proxy {proxy} is unavailable: {cause}"),
                    &ProxyUnavailableDetails {
                        proxy,
                        url: Some(url.to_owned()),
                        error: None,
                    },
                )
            }
        }
    }
}

/// Asks the proxy at the other end of `stream` for a tunnel to
/// `authority`, `host:port`, and reads its answer's head, nothing past it.
///
/// # Errors
///
/// [`ProxyFailure`] for any answer but a 2xx.
///
/// # Cancel safety
///
/// Not cancel safe: dropping the future leaves `stream` mid-exchange.
pub(super) async fn open(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    authority: &str,
) -> Result<(), ProxyFailure> {
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    let unanswered = || {
        ProxyFailure::Unrecognized(Unrecognized {
            answer: String::new(),
        })
    };
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| unanswered())?;
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() == HEAD_LIMIT {
            return Err(answered(&head).err().unwrap_or_else(unanswered));
        }
        match stream.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => return Err(answered(&head).err().unwrap_or_else(unanswered)),
        }
    }
    answered(&head)
}

/// What the proxy's answer head `head` means for the tunnel.
fn answered(head: &[u8]) -> Result<(), ProxyFailure> {
    let line = head.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line).trim_end().to_owned();
    let status = line
        .strip_prefix("HTTP/1.")
        .and_then(|rest| rest.get(1..))
        .and_then(|rest| rest.strip_prefix(' '))
        .and_then(|rest| rest.get(..3))
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        Some(200..=299) => Ok(()),
        Some(407) => Err(ProxyFailure::AuthenticationRequired),
        Some(429) => Err(ProxyFailure::RateLimited),
        Some(502) => Err(ProxyFailure::BadGateway),
        _ => {
            tracing::warn!(
                status,
                "the upstream proxy answered the tunnel request with an answer the engine does not recognize"
            );
            Err(ProxyFailure::Unrecognized(Unrecognized { answer: line }))
        }
    }
}
