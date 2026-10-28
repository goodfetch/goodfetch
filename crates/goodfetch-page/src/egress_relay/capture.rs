//! The capture of a browser build's ALPS payload when the build is pinned
//! (ADR 0042), which `pin-chrome-for-testing alps` runs: a TLS server of
//! its own name, accepting ALPS for `h2`, reads what the build sends.

use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::time::Duration;

use btls::error::ErrorStack;
use btls::pkey::{PKey, Private};
use btls::ssl::{self, AlpnError, SslAcceptor, SslMethod};
use btls::x509::X509;
use rcgen::{CertificateParams, KeyPair};
use tokio::net::TcpListener;
use tokio_btls::SslStream;

use super::application_settings::{ApplicationSettings, of_peer, offer};

/// How long a connection the capture accepted may take to finish its
/// handshake before the capture moves to the next one.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);

/// Captures the ALPS payload of the browser build that connects to
/// `listener` over TLS, as `pin-chrome-for-testing alps` does when it pins
/// the build: a server of its own name, which the browser must be told to
/// accept, choosing `h2` and accepting ALPS for it, until a connection
/// sends a payload.
///
/// # Errors
///
/// The listener's, or the server's set-up's.
///
/// # Cancel safety
///
/// Dropping the future closes the connection being read and the listener.
pub async fn capture_application_settings(
    listener: TcpListener,
) -> io::Result<ApplicationSettings> {
    let acceptor = capturing()?;
    loop {
        let (connection, _) = listener.accept().await?;
        let mut ssl = ssl::Ssl::new(acceptor.context()).map_err(io::Error::other)?;
        ssl.set_alps_use_new_codepoint(true);
        offer(&mut ssl, b"h2", b"").map_err(io::Error::other)?;
        let mut stream = SslStream::new(ssl, connection).map_err(io::Error::other)?;
        let accepted =
            tokio::time::timeout(HANDSHAKE_DEADLINE, Pin::new(&mut stream).accept()).await;
        if !matches!(accepted, Ok(Ok(()))) {
            continue;
        }
        let protocol = stream.ssl().selected_alpn_protocol().map(<[u8]>::to_vec);
        if let (Some(protocol), Some(payload)) = (protocol, of_peer(stream.ssl())) {
            return Ok(BTreeMap::from([(
                String::from_utf8_lossy(&protocol).into_owned(),
                payload,
            )]));
        }
    }
}

/// The capture's server: a certificate of its own, `h2` chosen when
/// offered.
fn capturing() -> io::Result<SslAcceptor> {
    let key = KeyPair::generate().map_err(io::Error::other)?;
    let certificate = CertificateParams::new(vec!["localhost".to_owned()])
        .and_then(|params| params.self_signed(&key))
        .map_err(io::Error::other)?;
    let built = (|| {
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())?;
        let leaf: X509 = X509::from_der(certificate.der())?;
        builder.set_certificate(&leaf)?;
        let private: PKey<Private> = PKey::private_key_from_pkcs8(&key.serialize_der())?;
        builder.set_private_key(&private)?;
        builder.set_alpn_select_callback(|_, offered| {
            ssl::select_next_proto(b"\x02h2", offered).ok_or(AlpnError::NOACK)
        });
        Ok::<_, ErrorStack>(builder.build())
    })();
    built.map_err(io::Error::other)
}
