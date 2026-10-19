//! Plain clients that reach the Proving ground the way any tool would, each
//! connecting to the reported address with the page's host name in its SNI
//! and `Host` (the `resolve` setting's mapping, ADR 0059): a raw HTTP/1.1
//! client whose bytes the test writes itself, the same over TLS, and an
//! HTTP/2 client over TLS. TLS clients trust only the certificate authority
//! they are given, and verify every certificate.
#![expect(
    clippy::disallowed_types,
    reason = "the test clients open their own sockets to the Proving ground"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

/// A client configuration that trusts only `authority` (PEM) and offers
/// `alpn`.
pub(crate) fn trusting(authority: &str, alpn: &[&str]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(authority.as_bytes()) {
        roots
            .add(certificate.expect("a PEM certificate"))
            .expect("a root certificate");
    }
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the default versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|name| name.as_bytes().to_vec()).collect();
    Arc::new(config)
}

/// What a raw exchange saw: the response's bytes, and how many bytes the
/// client wrote and read on the wire (TLS included).
pub(crate) struct Exchange {
    pub(crate) response: Vec<u8>,
    pub(crate) written: u64,
    pub(crate) read: u64,
}

impl Exchange {
    /// The response's status code.
    pub(crate) fn status(&self) -> u16 {
        let line = String::from_utf8_lossy(&self.response);
        line.split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("a status line")
    }

    /// The response's body, after its head.
    pub(crate) fn body(&self) -> &[u8] {
        let at = self
            .response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("a complete head");
        &self.response[at + 4..]
    }
}

/// A TCP stream that counts what crosses it.
struct Counted {
    stream: TcpStream,
    written: u64,
    read: u64,
}

impl std::io::Read for Counted {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.stream.read(buffer)?;
        self.read += read as u64;
        Ok(read)
    }
}

impl std::io::Write for Counted {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.stream.write(buffer)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

fn connect(address: &str) -> Counted {
    Counted {
        stream: TcpStream::connect(address).expect("the Proving ground accepts"),
        written: 0,
        read: 0,
    }
}

/// Writes `request` as it is on a new plain connection and reads the
/// response until the server closes it (send `Connection: close`).
pub(crate) fn raw(address: &str, request: &[u8]) -> Exchange {
    let mut stream = connect(address);
    stream.write_all(request).expect("the request is written");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("the response is read");
    Exchange {
        response,
        written: stream.written,
        read: stream.read,
    }
}

/// Writes `request` on a new TLS connection to `host`, then reads the
/// response until the server closes it; a refused handshake is the TLS
/// error.
pub(crate) fn raw_tls(
    address: &str,
    host: &str,
    config: Arc<ClientConfig>,
    request: &[u8],
) -> Result<Exchange, rustls::Error> {
    let name = ServerName::try_from(host.to_owned()).expect("a server name");
    let connection = ClientConnection::new(config, name).expect("a TLS client");
    let mut stream = StreamOwned::new(connection, connect(address));
    let outcome = stream.write_all(request).and_then(|()| {
        let mut response = Vec::new();
        stream.read_to_end(&mut response).map(|_| response)
    });
    match outcome {
        Ok(response) => Ok(Exchange {
            response,
            written: stream.sock.written,
            read: stream.sock.read,
        }),
        Err(error) => Err(*error
            .into_inner()
            .and_then(|inner| inner.downcast::<rustls::Error>().ok())
            .expect("a TLS error")),
    }
}

/// An HTTP/2 response's status and body.
pub(crate) struct Http2Answer {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

/// HTTP/2 settings the test client sends.
pub(crate) struct Http2Settings {
    pub(crate) initial_window_size: u32,
    pub(crate) initial_connection_window_size: u32,
    pub(crate) max_concurrent_streams: u32,
    pub(crate) header_table_size: u32,
}

/// Fetches `paths` on `host` over one HTTP/2 connection with TLS (ALPN
/// `h2`), each request carrying `headers` in order after its pseudo-headers.
pub(crate) fn http2(
    address: &str,
    host: &str,
    config: Arc<ClientConfig>,
    settings: &Http2Settings,
    paths: &[&str],
    headers: &[(&str, &str)],
) -> Vec<Http2Answer> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper builds the runtime its HTTP/2 client runs on (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let tcp = tokio::net::TcpStream::connect(address)
            .await
            .expect("the Proving ground accepts");
        let name = ServerName::try_from(host.to_owned()).expect("a server name");
        let tls = tokio_rustls::TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .expect("the handshake succeeds");
        let (mut client, connection) = h2::client::Builder::new()
            .enable_push(false)
            .initial_window_size(settings.initial_window_size)
            .initial_connection_window_size(settings.initial_connection_window_size)
            .max_concurrent_streams(settings.max_concurrent_streams)
            .header_table_size(settings.header_table_size)
            .handshake::<_, hyper::body::Bytes>(tls)
            .await
            .expect("the HTTP/2 handshake succeeds");
        let work = async {
            let mut answers = Vec::new();
            for path in paths {
                let mut request = hyper::Request::get(format!("https://{host}{path}"));
                for (name, value) in headers {
                    request = request.header(*name, *value);
                }
                let request = request.body(()).expect("a request");
                let (response, _) = client
                    .send_request(request, true)
                    .expect("the request is sent");
                let response = response.await.expect("a response");
                let status = response.status().as_u16();
                let mut body = response.into_body();
                let mut bytes = Vec::new();
                while let Some(chunk) = body.data().await {
                    let chunk = chunk.expect("a body chunk");
                    let _ = body.flow_control().release_capacity(chunk.len());
                    bytes.extend_from_slice(&chunk);
                }
                answers.push(Http2Answer {
                    status,
                    body: bytes,
                });
            }
            answers
        };
        tokio::pin!(connection);
        tokio::select! {
            answers = work => answers,
            result = &mut connection => panic!("the connection ended early: {result:?}"),
        }
    })
}
