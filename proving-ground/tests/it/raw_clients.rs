//! Clients for the hosts the raw server answers, which break HTTP on
//! purpose: an HTTP/1.1 client over TLS that keeps what arrived when a body
//! is cut, reuses one connection for several requests, and an HTTP/2 client
//! that reports a stream's reset or a connection's `GOAWAY` as the error it
//! is. Every one trusts only the seed's certificate authority.
#![expect(
    clippy::disallowed_types,
    reason = "the test clients open their own sockets to the Proving ground"
)]

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use crate::clients;
use crate::proving_ground::ProvingGround;

/// An HTTP/1.1 response as it arrived: its bytes, and whether the
/// connection then ended cleanly (a TLS `close_notify`).
pub(crate) struct Arrived {
    pub(crate) bytes: Vec<u8>,
    pub(crate) closed_cleanly: bool,
}

impl Arrived {
    /// The status code, if a status line arrived.
    pub(crate) fn status(&self) -> Option<u16> {
        String::from_utf8_lossy(&self.bytes)
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
    }

    /// The value of the header `name` (any case), if the head has it.
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let text = String::from_utf8_lossy(&self.bytes);
        let head = text.split("\r\n\r\n").next().unwrap_or_default();
        head.split("\r\n").skip(1).find_map(|line| {
            let (field, value) = line.split_once(':')?;
            field
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    }

    /// The bytes after the head.
    pub(crate) fn body(&self) -> &[u8] {
        self.bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map_or(&[][..], |at| &self.bytes[at + 4..])
    }
}

/// The TLS client configuration that trusts the Proving ground and offers
/// `alpn`.
pub(crate) fn trusting(proving_ground: &ProvingGround, alpn: &[&str]) -> Arc<ClientConfig> {
    clients::trusting(&proving_ground.certificate_authority(), alpn)
}

fn connect(
    proving_ground: &ProvingGround,
    host: &str,
    config: Arc<ClientConfig>,
) -> StreamOwned<ClientConnection, TcpStream> {
    let name = ServerName::try_from(host.to_owned()).expect("a server name");
    let connection = ClientConnection::new(config, name).expect("a TLS client");
    let socket = TcpStream::connect(proving_ground.address()).expect("the Proving ground accepts");
    StreamOwned::new(connection, socket)
}

/// A `GET` of `path` on `host` over HTTP/1.1 with TLS, with `headers`, read
/// until the connection ends, however it ends.
pub(crate) fn get(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> Arrived {
    get_within(proving_ground, host, path, headers, None)
}

/// A `GET` as [`get`] sends it, giving up on reading after `wait`.
pub(crate) fn get_within(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    wait: Option<std::time::Duration>,
) -> Arrived {
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{}\r\n",
        proving_ground.port()
    );
    for (name, value) in headers {
        let _ = write!(request, "{name}: {value}\r\n");
    }
    request.push_str("Connection: close\r\n\r\n");
    let mut stream = connect(
        proving_ground,
        host,
        trusting(proving_ground, &["http/1.1"]),
    );
    stream
        .write_all(request.as_bytes())
        .expect("the request is written");
    stream
        .sock
        .set_read_timeout(wait)
        .expect("a read timeout is set");
    read_to_end(&mut stream)
}

/// A `GET` as [`get`] sends it, read until `at_least` bytes arrived or the
/// connection ended, whichever is first: the start of a body that may never
/// end. A body past what was read is counted, not kept.
pub(crate) fn get_prefix(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
    at_least: usize,
) -> Arrived {
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{}\r\nConnection: close\r\n\r\n",
        proving_ground.port()
    );
    let mut stream = connect(
        proving_ground,
        host,
        trusting(proving_ground, &["http/1.1"]),
    );
    stream
        .write_all(request.as_bytes())
        .expect("the request is written");
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    while bytes.len() < at_least {
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Arrived {
                    bytes,
                    closed_cleanly: true,
                };
            }
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(_) => {
                return Arrived {
                    bytes,
                    closed_cleanly: false,
                };
            }
        }
    }
    Arrived {
        bytes,
        closed_cleanly: false,
    }
}

/// How many bytes a `GET` as [`get`] sends gets until the connection ends,
/// counted as they arrive, never kept: the length of a long body.
pub(crate) fn count(proving_ground: &ProvingGround, host: &str, path: &str) -> usize {
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{}\r\nConnection: close\r\n\r\n",
        proving_ground.port()
    );
    let mut stream = connect(
        proving_ground,
        host,
        trusting(proving_ground, &["http/1.1"]),
    );
    stream
        .write_all(request.as_bytes())
        .expect("the request is written");
    let mut total = 0;
    let mut chunk = vec![0; 256 * 1024];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        total += read;
    }
    total
}

/// Everything the stream gives until it ends, and how it ended.
fn read_to_end(stream: &mut StreamOwned<ClientConnection, TcpStream>) -> Arrived {
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                return Arrived {
                    bytes,
                    closed_cleanly: true,
                };
            }
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(_) => {
                return Arrived {
                    bytes,
                    closed_cleanly: false,
                };
            }
        }
    }
}

/// Sends `requests` one after another on one keep-alive HTTP/1.1
/// connection with TLS to `host`, each read by its `Content-Length`; the
/// answers that arrived, `None` from the first the connection ended
/// without.
pub(crate) fn keep_alive(
    proving_ground: &ProvingGround,
    host: &str,
    requests: &[(&str, &str)],
) -> Vec<Option<Arrived>> {
    let mut stream = connect(
        proving_ground,
        host,
        trusting(proving_ground, &["http/1.1"]),
    );
    let mut answers = Vec::new();
    let mut ended = false;
    for (method, path) in requests {
        if ended {
            answers.push(None);
            continue;
        }
        let body = if *method == "POST" { "a=1" } else { "" };
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}:{}\r\nContent-Length: {}\r\n\r\n{body}",
            proving_ground.port(),
            body.len()
        );
        if stream.write_all(request.as_bytes()).is_err() {
            ended = true;
            answers.push(None);
            continue;
        }
        let answer = read_one(&mut stream);
        ended = answer.is_none();
        answers.push(answer);
    }
    answers
}

/// One response read by its `Content-Length`; `None` when the connection
/// ends first.
fn read_one(stream: &mut StreamOwned<ClientConnection, TcpStream>) -> Option<Arrived> {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => bytes.push(byte[0]),
            _ => return None,
        }
    }
    let arrived = Arrived {
        bytes,
        closed_cleanly: false,
    };
    let length: usize = arrived
        .header("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).ok()?;
    let mut bytes = arrived.bytes;
    bytes.extend_from_slice(&body);
    Some(Arrived {
        bytes,
        closed_cleanly: false,
    })
}

/// An HTTP/2 answer: its status and body, or how the stream or connection
/// ended without one.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Http2Outcome {
    Answered {
        status: u16,
        body: Vec<u8>,
    },
    /// The stream was reset with this error code.
    Reset(h2::Reason),
    /// The connection went away (`GOAWAY` or its close) with this code.
    GoneAway(Option<h2::Reason>),
    /// The body broke off after this many bytes.
    BodyCut {
        status: u16,
        received: usize,
    },
}

/// Sends a `GET` of each of `paths` on `host` over one HTTP/2 connection
/// with TLS, all at once, and waits for each.
pub(crate) fn http2(
    proving_ground: &ProvingGround,
    host: &str,
    paths: &[&str],
) -> Vec<Http2Outcome> {
    http2_coalesced(proving_ground, host, host, paths)
}

/// Sends a `GET` of each of `paths` on `authority_host` over one HTTP/2
/// connection whose TLS handshake named `host`, as a client that coalesces
/// connections does.
pub(crate) fn http2_coalesced(
    proving_ground: &ProvingGround,
    host: &str,
    authority_host: &str,
    paths: &[&str],
) -> Vec<Http2Outcome> {
    let config = trusting(proving_ground, &["h2"]);
    let address = proving_ground.address().to_owned();
    let authority = format!("{authority_host}:{}", proving_ground.port());
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
        let (client, connection) = h2::client::Builder::new()
            .enable_push(false)
            .handshake::<_, hyper::body::Bytes>(tls)
            .await
            .expect("the HTTP/2 handshake succeeds");
        let driver = async {
            let _ = connection.await;
        };
        let requests = async {
            let mut pending = Vec::new();
            for path in paths {
                let request = hyper::Request::get(format!("https://{authority}{path}"))
                    .body(())
                    .expect("a request");
                let sent = match client.clone().ready().await {
                    Ok(mut ready) => ready
                        .send_request(request, true)
                        .ok()
                        .map(|(response, _)| response),
                    Err(_) => None,
                };
                pending.push(sent);
            }
            let mut outcomes = Vec::new();
            for response in pending {
                let Some(response) = response else {
                    outcomes.push(Http2Outcome::GoneAway(None));
                    continue;
                };
                outcomes.push(match response.await {
                    Err(error) => outcome_of(&error),
                    Ok(response) => {
                        let status = response.status().as_u16();
                        let mut body = response.into_body();
                        let mut bytes = Vec::new();
                        let mut cut = false;
                        while let Some(chunk) = body.data().await {
                            let Ok(chunk) = chunk else {
                                cut = true;
                                break;
                            };
                            let _ = body.flow_control().release_capacity(chunk.len());
                            bytes.extend_from_slice(&chunk);
                        }
                        if cut {
                            Http2Outcome::BodyCut {
                                status,
                                received: bytes.len(),
                            }
                        } else {
                            Http2Outcome::Answered {
                                status,
                                body: bytes,
                            }
                        }
                    }
                });
            }
            outcomes
        };
        tokio::pin!(driver);
        tokio::pin!(requests);
        tokio::select! {
            outcomes = &mut requests => outcomes,
            () = &mut driver => requests.await,
        }
    })
}

fn outcome_of(error: &h2::Error) -> Http2Outcome {
    if error.is_go_away() || error.is_io() {
        Http2Outcome::GoneAway(error.reason())
    } else {
        error
            .reason()
            .map_or(Http2Outcome::GoneAway(None), Http2Outcome::Reset)
    }
}
