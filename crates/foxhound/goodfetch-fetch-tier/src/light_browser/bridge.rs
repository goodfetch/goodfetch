//! The light-browser bridge (ADRs 0017 and 0020): the loopback HTTP proxy
//! Lightpanda reaches the network through, one per run on the tier. It
//! resolves each host itself, through the HTTP tier's resolver (the
//! `network.resolve` mapping, then the engine's resolver), judges every
//! address by the Run's egress policy, and connects to those alone, so
//! Lightpanda, which would resolve names itself, reaches nothing the HTTP
//! tier would not. It tunnels `CONNECT` and forwards absolute-form HTTP
//! requests, one per connection, leaving the bytes unchanged but for the
//! request line and the proxy's own headers. A connection without the
//! bridge's credentials gets a `407` and no relayed byte; a refused or
//! unresolved host a `403` or `502`.
#![expect(
    clippy::disallowed_types,
    reason = "the light-browser bridge opens its own loopback listener and its connections out, in this module only (ADR 0060)"
)]

use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use data_encoding::BASE64;
use goodfetch_engine::runs::Run;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::transports::AddressCheck;

/// The longest request head the bridge reads.
const MAX_HEAD: usize = 64 * 1024;

/// The user name of the bridge's credentials.
const USER: &str = "goodfetch";

/// A bridge, listening until dropped.
pub(crate) struct Bridge {
    address: SocketAddr,
    token: String,
    stop: CancellationToken,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl Bridge {
    /// A bridge for one run of `run`'s on the light-browser tier, whose
    /// connections go where `check` lets the HTTP tier's go.
    ///
    /// # Errors
    ///
    /// When no loopback port can be bound or no token drawn.
    pub(crate) async fn open(run: &Run, check: AddressCheck) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let token = goodfetch_engine::endpoint::new_token().map_err(io::Error::other)?;
        let stop = CancellationToken::new();
        let accepting = Accepting {
            run: run.clone(),
            check,
            expected: format!(
                "Basic {}",
                BASE64.encode(format!("{USER}:{token}").as_bytes())
            ),
            stop: stop.clone(),
        };
        run.spawn(accepting.accept(listener));
        Ok(Self {
            address,
            token,
            stop,
        })
    }

    /// The proxy URL Lightpanda is given, its credentials in it.
    pub(crate) fn proxy_url(&self) -> String {
        format!("http://{USER}:{}@{}", self.token, self.address)
    }
}

/// The bridge's accept loop and what each connection needs.
struct Accepting {
    run: Run,
    check: AddressCheck,
    /// The `Proxy-Authorization` value each request must carry.
    expected: String,
    stop: CancellationToken,
}

impl Accepting {
    async fn accept(self, listener: TcpListener) {
        loop {
            let accepted = tokio::select! {
                () = self.stop.cancelled() => return,
                accepted = listener.accept() => accepted,
            };
            let Ok((client, _)) = accepted else {
                continue;
            };
            let (check, expected, stop) =
                (self.check.clone(), self.expected.clone(), self.stop.clone());
            self.run.spawn(async move {
                tokio::select! {
                    () = stop.cancelled() => {}
                    relayed = relay(client, &check, &expected) => {
                        if let Err(error) = relayed {
                            tracing::debug!(%error, "a light-browser bridge connection ended early");
                        }
                    }
                }
            });
        }
    }
}

/// A request head the bridge read: its method, target, version and
/// header lines, and the bytes read after it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Head {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) version: String,
    pub(crate) headers: Vec<(String, String)>,
}

impl Head {
    /// Reads a request head from `bytes`, which end at its blank line.
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.split("\r\n");
        let mut request = lines.next()?.split(' ');
        let (method, target, version) = (request.next()?, request.next()?, request.next()?);
        if request.next().is_some() || !version.starts_with("HTTP/1.") {
            return None;
        }
        let headers = lines
            .take_while(|line| !line.is_empty())
            .map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            method: method.to_owned(),
            target: target.to_owned(),
            version: version.to_owned(),
            headers,
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Where a request goes: the URL a `CONNECT` tunnels to, as `https`,
    /// or an absolute-form request's own `http` URL.
    pub(crate) fn destination(&self) -> Option<Url> {
        if self.method == "CONNECT" {
            return Url::parse(&format!("https://{}/", self.target)).ok();
        }
        Url::parse(&self.target)
            .ok()
            .filter(|url| url.scheme() == "http")
    }

    /// The head sent on to the site for an absolute-form request to
    /// `url`: its target in origin form, the proxy's own headers and
    /// `Connection` dropped, and `Connection: close`, one request per
    /// connection.
    pub(crate) fn forwarded(&self, url: &Url) -> Vec<u8> {
        let mut target = url.path().to_owned();
        if let Some(query) = url.query() {
            target.push('?');
            target.push_str(query);
        }
        let mut head = format!("{} {target} {}\r\n", self.method, self.version);
        for (name, value) in &self.headers {
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("proxy-") || lower == "connection" || lower == "keep-alive" {
                continue;
            }
            let _ = write!(head, "{name}: {value}\r\n");
        }
        head.push_str("Connection: close\r\n\r\n");
        head.into_bytes()
    }
}

/// Reads one request from `client` and relays it.
async fn relay(mut client: TcpStream, check: &AddressCheck, expected: &str) -> io::Result<()> {
    let (head, rest) = read_head(&mut client).await?;
    let Some(head) = Head::parse(&head) else {
        return answer(&mut client, "400 Bad Request").await;
    };
    if head.header("proxy-authorization") != Some(expected) {
        return client
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"goodfetch\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
    }
    let Some(url) = head.destination() else {
        return answer(&mut client, "400 Bad Request").await;
    };
    let addresses = match check.connectable(&url).await {
        Ok(addresses) => addresses,
        Err(Some(refused)) => {
            tracing::debug!(%refused, "the light-browser bridge refused a connection");
            return answer(&mut client, "403 Forbidden").await;
        }
        Err(None) => return answer(&mut client, "502 Bad Gateway").await,
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let Some(mut upstream) = connect(&addresses, port).await else {
        return answer(&mut client, "502 Bad Gateway").await;
    };
    if head.method == "CONNECT" {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
    } else {
        upstream.write_all(&head.forwarded(&url)).await?;
    }
    upstream.write_all(&rest).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// The first of `addresses` that accepts a connection on `port`.
async fn connect(addresses: &[IpAddr], port: u16) -> Option<TcpStream> {
    for address in addresses {
        if let Ok(stream) = TcpStream::connect(SocketAddr::new(*address, port)).await {
            return Some(stream);
        }
    }
    None
}

/// Answers `status` with no body and closes.
async fn answer(client: &mut TcpStream, status: &str) -> io::Result<()> {
    client
        .write_all(
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
}

/// The request head `client` sent, up to its blank line, and what it sent
/// after it.
async fn read_head(client: &mut TcpStream) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut read = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        if let Some(end) = read.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = read.split_off(end + 4);
            return Ok((read, rest));
        }
        if read.len() > MAX_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a request head past its limit",
            ));
        }
        let count = client.read(&mut buffer).await?;
        if count == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        read.extend_from_slice(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests;
