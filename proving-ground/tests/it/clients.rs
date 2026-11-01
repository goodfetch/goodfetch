//! Plain clients that reach the Proving ground the way any tool would, each
//! connecting to the reported address with the page's host name in its SNI
//! and `Host` (the `resolve` setting's mapping, ADR 0059): a raw HTTP/1.1
//! client whose bytes the test writes itself, the same over TLS, an HTTP/2
//! client over TLS, a client that tunnels TLS through a forward proxy's
//! `CONNECT`, and a stub resolver's DNS query over UDP. TLS clients trust
//! only the certificate authority they are given, and verify every
//! certificate.
#![expect(
    clippy::disallowed_types,
    reason = "the test clients open their own sockets to the Proving ground"
)]

use std::io::{Read as _, Write as _};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
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

/// As [`trusting`], for a client that presents `certificate` (PEM) with
/// its private `key` (PEM) when a server asks for one.
pub(crate) fn presenting(authority: &str, certificate: &str, key: &str) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for anchor in CertificateDer::pem_slice_iter(authority.as_bytes()) {
        roots
            .add(anchor.expect("a PEM certificate"))
            .expect("a root certificate");
    }
    let chain = CertificateDer::pem_slice_iter(certificate.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .expect("a PEM client certificate");
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).expect("a PEM private key");
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the default versions")
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, key)
            .expect("the key signs");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// What a raw exchange saw: the response's bytes, how many bytes the
/// client wrote and read on the wire (TLS included), and the client's own
/// address, the connection's source.
pub(crate) struct Exchange {
    pub(crate) response: Vec<u8>,
    pub(crate) written: u64,
    pub(crate) read: u64,
    pub(crate) source: String,
}

/// The status code of a response's head.
fn status_of(response: &[u8]) -> u16 {
    let line = String::from_utf8_lossy(response);
    line.split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status line")
}

impl Exchange {
    /// The response's status code.
    pub(crate) fn status(&self) -> u16 {
        status_of(&self.response)
    }

    /// The value of the response's header `name`, if it has one.
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let response = String::from_utf8_lossy(&self.response);
        let head = response.split("\r\n\r\n").next().unwrap_or_default();
        head.split("\r\n").skip(1).find_map(|line| {
            let (field, value) = line.split_once(':')?;
            field
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
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

impl Counted {
    /// The client's own address.
    fn source(&self) -> String {
        self.stream
            .local_addr()
            .expect("a connected stream has an address")
            .to_string()
    }
}

/// Whether a connection to `address` is refused, as one to a port where
/// nothing listens is.
pub(crate) fn connection_refused(address: &str) -> bool {
    TcpStream::connect(address)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
}

/// A listener on `address`, opened as any other program on the machine
/// opens one (the standard library's, which reuses addresses on Unix).
pub(crate) fn listen(address: &str) -> std::io::Result<TcpListener> {
    TcpListener::bind(address)
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
        source: stream.source(),
    }
}

/// Writes `request` as [`raw`] does and reads the response until the server
/// ends its side, then keeps the connection open: the stream is returned,
/// and the connection closes once it is dropped.
pub(crate) fn raw_kept_open(address: &str, request: &[u8]) -> (Exchange, TcpStream) {
    let mut stream = connect(address);
    stream.write_all(request).expect("the request is written");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("the response is read");
    let exchange = Exchange {
        response,
        written: stream.written,
        read: stream.read,
        source: stream.source(),
    };
    (exchange, stream.stream)
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
            source: stream.sock.source(),
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

/// What a forward proxy answered to `CONNECT`: its status, and when it
/// opened the tunnel, the exchange the client then had through it over TLS.
pub(crate) struct Tunnel {
    pub(crate) status: u16,
    pub(crate) exchange: Option<Exchange>,
}

/// Where a SOCKS5 exchange stopped short of its tunnel: the step and the
/// code the proxy answered it with.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Socks5Refusal {
    /// It took none of the methods offered.
    Method(u8),
    /// It refused the credentials.
    Credentials(u8),
    /// It refused the `CONNECT`.
    Connect(u8),
}

/// Asks the SOCKS5 proxy at `proxy` (RFC 1928), with `username` and
/// `password` (RFC 1929), for a tunnel to the domain `host` on `port`, then
/// writes `request` through it as it is and reads the response until the
/// server closes it.
pub(crate) fn socks5(
    proxy: &str,
    (username, password): (&str, &str),
    (host, port): (&str, u16),
    request: &[u8],
) -> Result<Exchange, Socks5Refusal> {
    let mut stream = connect(proxy);
    let mut answer = [0; 2];
    stream
        .write_all(&[5, 1, 2])
        .expect("the greeting is written");
    stream
        .read_exact(&mut answer)
        .expect("the method is answered");
    if answer != [5, 2] {
        return Err(Socks5Refusal::Method(answer[1]));
    }
    let mut credentials = vec![
        1,
        u8::try_from(username.len()).expect("a username of at most 255 bytes"),
    ];
    credentials.extend_from_slice(username.as_bytes());
    credentials.push(u8::try_from(password.len()).expect("a password of at most 255 bytes"));
    credentials.extend_from_slice(password.as_bytes());
    stream
        .write_all(&credentials)
        .expect("the credentials are written");
    stream
        .read_exact(&mut answer)
        .expect("the credentials are answered");
    if answer != [1, 0] {
        return Err(Socks5Refusal::Credentials(answer[1]));
    }
    let mut connect = vec![
        5,
        1,
        0,
        3,
        u8::try_from(host.len()).expect("a host of at most 255 bytes"),
    ];
    connect.extend_from_slice(host.as_bytes());
    connect.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&connect).expect("the request is written");
    let mut reply = [0; 10];
    stream
        .read_exact(&mut reply)
        .expect("the request is answered");
    if reply[1] != 0 {
        return Err(Socks5Refusal::Connect(reply[1]));
    }
    stream.write_all(request).expect("the request is written");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("the response is read");
    Ok(Exchange {
        response,
        written: stream.written,
        read: stream.read,
        source: stream.source(),
    })
}

/// Asks the proxy at `proxy` for a tunnel to `authority` (with
/// `authorization` as its `Proxy-Authorization`, if any), then writes
/// `request` over TLS to `host` through it and reads the response until the
/// server closes it.
pub(crate) fn tunnel_tls(
    proxy: &str,
    authority: &str,
    authorization: Option<&str>,
    host: &str,
    config: Arc<ClientConfig>,
    request: &[u8],
) -> Tunnel {
    let mut stream = connect(proxy);
    let mut head = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(authorization) = authorization {
        head.push_str("Proxy-Authorization: ");
        head.push_str(authorization);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .expect("the request is written");
    let mut answer = Vec::new();
    let mut byte = [0; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).expect("the answer is read") == 0 {
            break;
        }
        answer.push(byte[0]);
    }
    let status = status_of(&answer);
    if status != 200 {
        return Tunnel {
            status,
            exchange: None,
        };
    }
    let name = ServerName::try_from(host.to_owned()).expect("a server name");
    let connection = ClientConnection::new(config, name).expect("a TLS client");
    let mut tls = StreamOwned::new(connection, stream);
    tls.write_all(request).expect("the request is written");
    let mut response = Vec::new();
    tls.read_to_end(&mut response)
        .expect("the response is read");
    Tunnel {
        status,
        exchange: Some(Exchange {
            response,
            written: tls.sock.written,
            read: tls.sock.read,
            source: tls.sock.source(),
        }),
    }
}

/// A DNS record type a test asks for.
#[derive(Clone, Copy)]
pub(crate) enum RecordType {
    A = 1,
    Aaaa = 28,
}

/// What a DNS server answered: its response code, each answer's TTL and
/// address, in order, and the negative TTL its authority section's SOA
/// gives, the smaller of the SOA's TTL and its minimum (RFC 2308).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DnsAnswer {
    pub(crate) rcode: u8,
    pub(crate) answers: Vec<(u32, IpAddr)>,
    pub(crate) negative_ttl: Option<u32>,
}

/// Asks the DNS server at `server` for `name`'s records of one type, over
/// UDP, as a stub resolver does (RFC 1035).
/// Sends one datagram of `bytes` to `server` from a socket of its own, and
/// returns the address it was sent from.
pub(crate) fn udp_datagram(server: &str, bytes: &[u8]) -> std::net::SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("a UDP socket");
    socket.send_to(bytes, server).expect("the datagram is sent");
    socket.local_addr().expect("the socket's address")
}

pub(crate) fn dns_query(server: &str, name: &str, record_type: RecordType) -> DnsAnswer {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("a UDP socket");
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout");
    // Header: ID 0x1234, recursion desired, one question.
    let mut query = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        query.push(u8::try_from(label.len()).expect("a short label"));
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&(record_type as u16).to_be_bytes());
    query.extend_from_slice(&1u16.to_be_bytes());
    socket.send_to(&query, server).expect("the query is sent");
    let mut buffer = [0; 1500];
    let (length, _) = socket.recv_from(&mut buffer).expect("an answer");
    let message = &buffer[..length];
    assert_eq!(
        &message[..2],
        &[0x12, 0x34],
        "the answer carries the query's ID"
    );
    let count = usize::from(u16::from_be_bytes([message[6], message[7]]));
    // The question is echoed as sent: skip it.
    let mut at = query.len();
    let mut answers = Vec::new();
    for _ in 0..count {
        at = skip_name(message, at);
        let ttl = u32::from_be_bytes([
            message[at + 4],
            message[at + 5],
            message[at + 6],
            message[at + 7],
        ]);
        let length = usize::from(u16::from_be_bytes([message[at + 8], message[at + 9]]));
        let data = &message[at + 10..at + 10 + length];
        let address = match length {
            4 => IpAddr::from(<[u8; 4]>::try_from(data).expect("four bytes")),
            16 => IpAddr::from(<[u8; 16]>::try_from(data).expect("sixteen bytes")),
            other => panic!("an address answer of {other} bytes"),
        };
        answers.push((ttl, address));
        at += 10 + length;
    }
    let authorities = usize::from(u16::from_be_bytes([message[8], message[9]]));
    let mut negative_ttl = None;
    for _ in 0..authorities {
        at = skip_name(message, at);
        let record_type = u16::from_be_bytes([message[at], message[at + 1]]);
        let ttl = u32::from_be_bytes(message[at + 4..at + 8].try_into().expect("four bytes"));
        let length = usize::from(u16::from_be_bytes([message[at + 8], message[at + 9]]));
        let data = &message[at + 10..at + 10 + length];
        if record_type == SOA {
            // MNAME and RNAME, then five numbers, MINIMUM last.
            let minimum = u32::from_be_bytes(data[length - 4..].try_into().expect("four bytes"));
            negative_ttl = Some(ttl.min(minimum));
        }
        at += 10 + length;
    }
    DnsAnswer {
        rcode: message[3] & 0x0f,
        answers,
        negative_ttl,
    }
}

/// The record type of a zone's start of authority.
const SOA: u16 = 6;

/// The offset just past the name at `at`, a compression pointer included.
fn skip_name(message: &[u8], mut at: usize) -> usize {
    loop {
        match message[at] {
            0 => return at + 1,
            length if length & 0xc0 == 0xc0 => return at + 2,
            length => at += 1 + usize::from(length),
        }
    }
}
