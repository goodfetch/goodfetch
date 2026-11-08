//! What every relay of a Session answers its client itself, the egress
//! relay the browser connects to and the loopback relay that chains to the
//! Session's proxy alike: their credentials' user name, the `407` that
//! asks for them, the `200` of a tunnel that opened, and a failure of the
//! relay's own, whose `Proxy-Status` member names the relay (RFC 9209), so
//! the engine never reads it as the proxy's refusal (ADR 0053).

/// The name a relay gives its own member of a `Proxy-Status` field.
pub(crate) const RELAY: &str = "goodfetch-relay";

/// The user name of every relay's client credentials; the password is the
/// Session's relay's own.
pub(crate) const USERNAME: &str = "goodfetch";

/// The answer to a client without the relay's credentials.
pub(crate) const AUTHENTICATE: &[u8] = b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"goodfetch\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// The answer to a tunnel that opened.
pub(crate) const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

/// The relay's own answer to a request or tunnel it could not take on,
/// with `status` and the `Proxy-Status` member naming `error`.
pub(crate) fn failed(status: &str, error: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nProxy-Status: {RELAY}; error={error}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}
