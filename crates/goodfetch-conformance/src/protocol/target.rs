//! A target that speaks Goodfetch's JSON-RPC protocol, started from its
//! executable taken by path (ADR 0096) the way its Clients start it: as an
//! Attached engine, `<executable> attached-engine` over its stdio, or as a
//! Daemon, `<executable> daemon launched`, whose runtime file it prints once
//! ready and whose socket the harness then connects to. Every read waits at
//! most [`DEADLINE`], and its first answer, which a process just started
//! gives, at most [`START_DEADLINE`], so a target that stays silent fails
//! its case instead of holding the harness.
//!
//! A gateway (ADR 0096) is reached by address instead: its `http` or `https`
//! origin, whose WebSocket at `/protocol` (`ws` or `wss`, TLS checked
//! against Mozilla's roots and any certificate the target trusts) carries
//! one JSON-RPC message a frame, the gateway's token presented in `hello`
//! (spec #586's duty 8). Its connect, TLS and WebSocket handshakes each wait
//! at most [`START_DEADLINE`].
//!
//! The harness is a Client of the target and runs outside any engine, so the
//! Simulation never runs this module: it opens the Daemon's socket itself and
//! reads each stream on a thread of its own, which ends when the stream does
//! (ADR 0060's note for the conformance harness).
#![expect(
    clippy::disallowed_types,
    reason = "the conformance harness connects to a Daemon's socket from outside the engine, never inside the Simulation (ADR 0060's note for the harness)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness starts its target's process and reads each of its streams on a thread that ends with the stream, outside any async worker (ADR 0060's note for the harness)"
)]

use std::collections::BTreeMap;
use std::io::{self, BufRead as _, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs as _};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use secrecy::ExposeSecret as _;
use tungstenite::client::IntoClientRequest as _;
use tungstenite::{Message, WebSocket};

use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonRuntimeFile};

use super::gateway::GatewayTarget;

/// How long the harness waits for each line the target writes, for the
/// Daemon to be ready and for the target to exit once the suite ends.
pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

/// How long the harness waits for a process it just started to be ready
/// and to answer `hello`, which a loaded machine's first start of a fresh
/// build can take longer than any later answer.
pub(crate) const START_DEADLINE: Duration = Duration::from_secs(30);

/// How long the harness waits for a call that launches or closes a browser:
/// a launch checks the build's digest and starts the process, and a Run's
/// end waits for its browser to exit.
pub(crate) const BROWSER_DEADLINE: Duration = Duration::from_secs(60);

/// A target that speaks the protocol: an executable taken by path, started
/// in a mode with exactly this environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineTarget {
    /// The executable: either Product's, or one that is started as they are,
    /// such as the Scripted engine (ADR 0069).
    pub executable: PathBuf,
    /// How the harness reaches it.
    pub mode: EngineMode,
    /// The whole environment it runs with, its Goodfetch home among it.
    pub environment: BTreeMap<String, String>,
    /// The folder it runs in.
    pub folder: PathBuf,
}

/// How the harness reaches a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineMode {
    /// An Attached engine over the target's stdio.
    Attached,
    /// A Daemon over the socket its runtime file names, with its token.
    Daemon,
}

impl EngineMode {
    /// The mode as the result document's case entries name it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attached => "attached",
            Self::Daemon => "daemon",
        }
    }
}

/// Why a target could not be reached.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StartError {
    /// Its process did not start.
    #[error("{} did not start: {source}", executable.display())]
    Spawn {
        /// The executable.
        executable: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The Daemon printed no runtime file before it exited or the deadline.
    #[error("the Daemon printed no runtime file once ready: {reason}")]
    NotReady {
        /// What it did instead.
        reason: String,
    },
    /// The gateway refused the sign-in or the WebSocket.
    #[error("the gateway at {address} took no Connection: {reason}")]
    Gateway {
        /// Its address.
        address: String,
        /// Why.
        reason: String,
    },
    /// The Daemon's endpoint took no connection.
    #[error("the Daemon at {endpoint} took no connection: {source}")]
    Connect {
        /// Its endpoint.
        endpoint: String,
        /// Why.
        source: io::Error,
    },
}

/// One line the target wrote, or why its stream ended in a failure.
type Line = io::Result<String>;

/// A started target and one Connection the harness holds to it: the
/// process it started, or, for a further Connection to a Daemon it
/// started, none.
pub(crate) struct Started {
    child: Option<Child>,
    link: Link,
    /// The lines of the Connection.
    lines: Receiver<Line>,
    /// For a Daemon, the lines of its stdout after the ready one, which end
    /// when its process does; an Attached engine's end shows on `lines`.
    exited: Option<Receiver<Line>>,
    /// The token the Daemon's runtime file names.
    pub(crate) token: Option<String>,
    /// The Daemon's endpoint, which a further Connection reaches.
    endpoint: Option<DaemonEndpoint>,
}

/// Where the harness writes.
enum Link {
    Stdin(Option<std::process::ChildStdin>),
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
    /// A gateway's WebSocket, written by the thread that holds it.
    WebSocket(Option<Sender<String>>),
}

/// What waiting for a line gave.
pub(crate) enum Next {
    /// A line.
    Line(String),
    /// The target closed the Connection.
    Closed,
    /// Nothing came before the deadline.
    Silent,
    /// The stream failed.
    Failed(io::Error),
}

impl Started {
    /// Starts `target` and opens its Connection, before any `hello`.
    pub(crate) fn start(target: &EngineTarget) -> Result<Self, StartError> {
        let mut command = Command::new(&target.executable);
        command
            .env_clear()
            .envs(&target.environment)
            .current_dir(&target.folder)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        match target.mode {
            EngineMode::Attached => command.arg("attached-engine").stdin(Stdio::piped()),
            EngineMode::Daemon => command.args(["daemon", "launched"]).stdin(Stdio::null()),
        };
        let mut child = command.spawn().map_err(|source| StartError::Spawn {
            executable: target.executable.clone(),
            source,
        })?;
        let stdout = child
            .stdout
            .take()
            .expect("invariant: the target's stdout is piped");
        match target.mode {
            EngineMode::Attached => {
                let stdin = child.stdin.take();
                Ok(Self {
                    child: Some(child),
                    link: Link::Stdin(stdin),
                    lines: read_lines(stdout),
                    exited: None,
                    token: None,
                    endpoint: None,
                })
            }
            EngineMode::Daemon => Self::connect_daemon(child, stdout),
        }
    }

    /// Reads the runtime file the Daemon prints once ready and connects to
    /// its endpoint.
    fn connect_daemon(child: Child, stdout: ChildStdout) -> Result<Self, StartError> {
        let mut started = Self {
            child: Some(child),
            link: Link::Stdin(None),
            lines: mpsc::channel().1,
            exited: Some(read_lines(stdout)),
            token: None,
            endpoint: None,
        };
        let ready = started
            .exited
            .as_ref()
            .expect("invariant: a Daemon's stdout is read")
            .recv_timeout(START_DEADLINE);
        let runtime: DaemonRuntimeFile = match ready {
            Ok(Ok(line)) => serde_json::from_str(&line).map_err(|error| StartError::NotReady {
                reason: format!("its first line is no runtime file ({error}): {line}"),
            })?,
            Ok(Err(error)) => {
                return Err(StartError::NotReady {
                    reason: format!("its stdout failed: {error}"),
                });
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(StartError::NotReady {
                    reason: format!("nothing came within {} s", START_DEADLINE.as_secs()),
                });
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(StartError::NotReady {
                    reason: "it exited first".to_owned(),
                });
            }
        };
        let (link, reader) = connect(&runtime.endpoint)?;
        started.link = link;
        started.lines = read_lines(reader);
        started.token = Some(runtime.token.expose().to_owned());
        started.endpoint = Some(runtime.endpoint);
        Ok(started)
    }

    /// Opens a Connection to the gateway `target`, before any `hello`, which
    /// presents the target's token.
    pub(crate) fn gateway(target: &GatewayTarget) -> Result<Self, StartError> {
        Self::gateway_within(target, START_DEADLINE)
    }

    /// [`Started::gateway`], each handshake waiting at most `deadline`.
    fn gateway_within(target: &GatewayTarget, deadline: Duration) -> Result<Self, StartError> {
        let address = &target.address;
        let refused = |reason: String| StartError::Gateway {
            address: address.to_string(),
            reason,
        };
        let stream = connect_within(&address.host(), address.port(), deadline)
            .map_err(|error| refused(error.to_string()))?;
        // The handshakes that follow wait at most the deadline too.
        for set in [TcpStream::set_read_timeout, TcpStream::set_write_timeout] {
            set(&stream, Some(deadline)).map_err(|error| refused(error.to_string()))?;
        }
        let stream = if address.is_tls() {
            GatewayStream::Tls(Box::new(
                tls(stream, &address.host(), &target.trusted_certificates).map_err(refused)?,
            ))
        } else {
            GatewayStream::Plain(stream)
        };
        let request = address
            .websocket()
            .as_str()
            .into_client_request()
            .map_err(|error| refused(error.to_string()))?;
        let (socket, _) =
            tungstenite::client(request, stream).map_err(|error| refused(error.to_string()))?;
        socket
            .get_ref()
            .tcp()
            .set_read_timeout(Some(POLL))
            .map_err(|error| refused(error.to_string()))?;
        let (outgoing, sending) = mpsc::channel();
        Ok(Self {
            child: None,
            link: Link::WebSocket(Some(outgoing)),
            lines: relay_socket(socket, sending),
            exited: None,
            token: Some(target.token.expose_secret().to_owned()),
            endpoint: None,
        })
    }

    /// Opens a further Connection to the same target, before any `hello`:
    /// a process of its own for an Attached engine, and the same Daemon's
    /// endpoint for a Daemon.
    pub(crate) fn another(&self, target: &EngineTarget) -> Result<Self, StartError> {
        let Some(endpoint) = &self.endpoint else {
            return Self::start(target);
        };
        let (link, reader) = connect(endpoint)?;
        Ok(Self {
            child: None,
            link,
            lines: read_lines(reader),
            exited: None,
            token: self.token.clone(),
            endpoint: None,
        })
    }

    /// Writes one line.
    pub(crate) fn write_line(&mut self, line: &[u8]) -> io::Result<()> {
        let mut framed = line.to_vec();
        framed.push(b'\n');
        let write = |writer: &mut dyn Write| {
            writer.write_all(&framed)?;
            writer.flush()
        };
        match &mut self.link {
            Link::Stdin(Some(stdin)) => write(stdin),
            Link::Stdin(None) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            Link::Tcp(stream) => write(stream),
            #[cfg(unix)]
            Link::Unix(stream) => write(stream),
            Link::WebSocket(Some(socket)) => socket
                .send(String::from_utf8_lossy(line).into_owned())
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe)),
            Link::WebSocket(None) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    /// The next line the target writes on the Connection, within
    /// `deadline`.
    pub(crate) fn next_line(&self, deadline: Duration) -> Next {
        match self.lines.recv_timeout(deadline) {
            Ok(Ok(line)) => Next::Line(line),
            Ok(Err(error)) => Next::Failed(error),
            Err(RecvTimeoutError::Timeout) => Next::Silent,
            Err(RecvTimeoutError::Disconnected) => Next::Closed,
        }
    }

    /// Closes the Connection and waits for the target to exit, stopping it
    /// when it has not by the deadline.
    pub(crate) fn close(mut self) {
        self.close_link();
        let Some(mut child) = self.child.take() else {
            return;
        };
        let ends = self.exited.as_ref().unwrap_or(&self.lines);
        let exited = loop {
            match ends.recv_timeout(DEADLINE) {
                Ok(_) => {}
                Err(RecvTimeoutError::Disconnected) => break true,
                Err(RecvTimeoutError::Timeout) => break false,
            }
        };
        if !exited {
            // A target that outlives the deadline is stopped, never left.
            let _ = child.kill();
        }
        let _ = child.wait();
    }

    fn close_link(&mut self) {
        // A target that already went leaves nothing to shut down.
        match &mut self.link {
            Link::Stdin(stdin) => drop(stdin.take()),
            Link::Tcp(stream) => drop(stream.shutdown(Shutdown::Both)),
            #[cfg(unix)]
            Link::Unix(stream) => drop(stream.shutdown(Shutdown::Both)),
            Link::WebSocket(socket) => drop(socket.take()),
        }
    }
}

impl Drop for Started {
    /// A target the harness gives up on early is stopped, never left.
    fn drop(&mut self) {
        self.close_link();
        if let Some(child) = &mut self.child
            && matches!(child.try_wait(), Ok(None))
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Connects to a Daemon's endpoint.
fn connect(endpoint: &DaemonEndpoint) -> Result<(Link, Box<dyn Read + Send>), StartError> {
    let connect_error = |source| StartError::Connect {
        endpoint: endpoint.to_string(),
        source,
    };
    match endpoint {
        DaemonEndpoint::Tcp(address) => {
            let stream = TcpStream::connect_timeout(address, DEADLINE).map_err(connect_error)?;
            let reader = stream.try_clone().map_err(connect_error)?;
            Ok((Link::Tcp(stream), Box::new(reader)))
        }
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            let stream = UnixStream::connect(path).map_err(connect_error)?;
            let reader = stream.try_clone().map_err(connect_error)?;
            Ok((Link::Unix(stream), Box::new(reader)))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => Err(connect_error(io::Error::from(io::ErrorKind::Unsupported))),
    }
}

/// How long the thread that holds a gateway's WebSocket waits for a frame
/// before it sends what the harness wrote meanwhile.
const POLL: Duration = Duration::from_millis(20);

/// A TCP connection to `host`'s first address that takes one within
/// `deadline`.
fn connect_within(host: &str, port: u16, deadline: Duration) -> io::Result<TcpStream> {
    let mut failed = None;
    for address in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, deadline) {
            Ok(stream) => return Ok(stream),
            Err(error) => failed = Some(error),
        }
    }
    Err(failed.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("`{host}` names no address"),
        )
    }))
}

/// `stream` under TLS to `host`, whose certificate Mozilla's roots or one
/// of `trusted` (DER) signs.
fn tls(
    stream: TcpStream,
    host: &str,
    trusted: &[Vec<u8>],
) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    for certificate in trusted {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .map_err(|error| format!("a trusted certificate is unusable: {error}"))?;
    }
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| error.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).map_err(|error| error.to_string())?;
    let connection =
        ClientConnection::new(Arc::new(config), name).map_err(|error| error.to_string())?;
    Ok(StreamOwned::new(connection, stream))
}

/// A gateway's connection: plain, or under TLS.
enum GatewayStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl GatewayStream {
    /// The TCP connection under it.
    fn tcp(&self) -> &TcpStream {
        match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => &stream.sock,
        }
    }
}

impl Read for GatewayStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for GatewayStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// Starts the thread that holds a gateway's WebSocket: it sends each message
/// `sending` gives and hands on each text frame it reads as a line, until
/// either side ends.
fn relay_socket(mut socket: WebSocket<GatewayStream>, sending: Receiver<String>) -> Receiver<Line> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            match sending.try_recv() {
                Ok(text) => {
                    if let Err(error) = socket.send(Message::text(text)) {
                        let _ = sender.send(Err(io::Error::other(error.to_string())));
                        return;
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    return;
                }
                Err(TryRecvError::Empty) => {}
            }
            match socket.read() {
                Ok(Message::Text(text)) => {
                    if sender.send(Ok(text.to_string())).is_err() {
                        return;
                    }
                }
                Ok(Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return;
                }
                Err(error) => {
                    let _ = sender.send(Err(io::Error::other(error.to_string())));
                    return;
                }
            }
        }
    });
    receiver
}

/// Starts the thread that reads `stream` a line at a time, until it ends
/// or fails.
fn read_lines(stream: impl Read + Send + 'static) -> Receiver<Line> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stream = BufReader::new(stream);
        loop {
            let mut line = String::new();
            let read = match stream.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => Ok(line.trim_end_matches(['\n', '\r']).to_owned()),
                Err(error) => Err(error),
            };
            let failed = read.is_err();
            if sender.send(read).is_err() || failed {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests;
