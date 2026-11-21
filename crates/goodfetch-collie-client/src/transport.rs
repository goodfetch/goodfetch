//! Every byte stream the client opens to an engine, taken from a network
//! value (ADR 0060): the OS's, or the in-memory one through which tests and
//! each Product's SDK crate reach an engine in the same process. On the OS's
//! network an Attached engine is a child process the client starts and
//! talks to over its stdio; a Daemon is the Unix socket or loopback TCP
//! address its endpoint names (ADR 0024), never a named pipe; an Exposed
//! Daemon is a WebSocket on loopback TCP (ADR 0049). This is the
//! one module on the client side that opens a socket or starts a process
//! (spec #606). An Attached engine's stderr is never the program's: each
//! line it writes is logged at warning and its last 64 KiB kept, which
//! `engine_exited` carries with how it ended (ADR 0078).

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use goodfetch_protocol::cli::ATTACHED_ENGINE;
use goodfetch_protocol::connection::{DaemonEndpoint, EngineExitedDetails};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, BufReader};
use tokio::process::{Child, ChildStderr};
use tokio::sync::oneshot;

mod websocket;

/// The most of an Attached engine's stderr `engine_exited` carries (ADR
/// 0078).
const STDERR_TAIL_BYTES: usize = 64 * 1024;

/// How a Connection's engine ended, once it has.
pub(crate) type Exit = Pin<Box<dyn Future<Output = EngineExitedDetails> + Send>>;

/// One Connection's bytes, both ways.
pub struct Link {
    reader: Box<dyn AsyncRead + Send + Unpin>,
    writer: Box<dyn AsyncWrite + Send + Unpin>,
    exit: Option<Exit>,
}

impl Link {
    /// The Connection that reads from `reader` and writes to `writer`.
    pub fn new(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            exit: None,
        }
    }

    /// The same Connection to an engine that ending ends it, as an Attached
    /// or a linked engine does, and `exit` tells how once it has: when the
    /// engine ends the Connection before the Client closes it, each call
    /// still unanswered, and every later one, fails with `engine_exited`
    /// and what `exit` yields (ADR 0078).
    ///
    /// ```
    /// use goodfetch_collie_client::transport::Link;
    /// use goodfetch_protocol::connection::EngineExitedDetails;
    ///
    /// let (client_end, _engine_end) = tokio::io::duplex(1024);
    /// let (reader, writer) = tokio::io::split(client_end);
    /// let ended = std::future::ready(EngineExitedDetails::without_evidence());
    /// let _link = Link::new(reader, writer).ending_with(ended);
    /// ```
    #[must_use]
    pub fn ending_with(
        mut self,
        exit: impl Future<Output = EngineExitedDetails> + Send + 'static,
    ) -> Self {
        self.exit = Some(Box::pin(exit));
        self
    }

    /// Its two directions, and how its engine ended when it is one that
    /// ending ends the Connection.
    pub(crate) fn split(
        self,
    ) -> (
        Box<dyn AsyncRead + Send + Unpin>,
        Box<dyn AsyncWrite + Send + Unpin>,
        Option<Exit>,
    ) {
        (self.reader, self.writer, self.exit)
    }
}

impl fmt::Debug for Link {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Link")
    }
}

/// What opens a Connection on an in-memory network: area 3's dialer, which
/// the engine's `transport` module hands out with the engine's end.
pub trait Dial: Send + Sync + 'static {
    /// Opens a Connection to the network's engine.
    ///
    /// # Errors
    ///
    /// When the engine no longer takes Connections.
    fn dial(&self) -> Pin<Box<dyn Future<Output = io::Result<Link>> + Send + '_>>;
}

/// The network a Connection reaches its engine over.
#[derive(Clone)]
#[non_exhaustive]
pub enum Network {
    /// The OS's: processes, Unix sockets and loopback TCP.
    Os,
    /// An in-memory one, whose dialer reaches the one engine on it,
    /// whatever endpoint the Connection names.
    InMemory(Arc<dyn Dial>),
}

impl Network {
    /// The in-memory network `dial` opens Connections on.
    pub fn in_memory(dial: impl Dial) -> Self {
        Self::InMemory(Arc::new(dial))
    }
}

impl fmt::Debug for Network {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Os => formatter.write_str("Os"),
            Self::InMemory(_) => formatter.write_str("InMemory"),
        }
    }
}

/// Where an engine takes its Connection.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Endpoint {
    /// The engine on an in-memory network, which only that network reaches.
    InMemory,
    /// An Attached engine the Connection starts and owns.
    Attached(AttachedEngine),
    /// A running Daemon.
    Daemon(DaemonEndpoint),
    /// An Exposed Daemon, reached over WebSocket.
    Exposed(ExposedDaemon),
}

/// An Exposed Daemon's address, reached over WebSocket with the
/// subprotocol `goodfetch.v<major>` of the protocol version the client
/// speaks (ADR 0049; spec #606's Transports), its handshake carrying the
/// Daemon's own `Host` and, over `http`, `Origin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposedDaemon {
    /// Where it listens.
    pub address: SocketAddr,
    /// The `host:port` its URL names, which its handshake carries.
    pub authority: String,
}

/// A Product's executable, started as an Attached engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedEngine {
    /// The executable.
    pub executable: PathBuf,
    /// Its whole environment: it inherits none of the program's.
    pub environment: BTreeMap<OsString, OsString>,
}

/// An opened Connection, and the Attached engine's process when the
/// Connection started one.
pub(crate) struct Opened {
    pub(crate) link: Link,
    pub(crate) process: Option<Process>,
}

/// Opens a Connection to the engine at `endpoint` over `network`. An
/// in-memory network reaches its one engine whatever the endpoint, so a
/// test stands it in for the OS's under the endpoint the code names.
///
/// # Errors
///
/// When the engine cannot be reached: an Attached engine's executable that
/// does not start, a Daemon's endpoint where nothing listens, an Exposed
/// Daemon's WebSocket whose handshake fails, an in-memory network whose
/// engine is gone, or the in-memory endpoint on the OS's network.
///
/// # Cancel safety
///
/// Dropping the future closes what it was opening, and kills an Attached
/// engine it started.
pub(crate) async fn open(network: &Network, endpoint: &Endpoint) -> io::Result<Opened> {
    match (network, endpoint) {
        (Network::InMemory(dial), _) => Ok(Opened {
            link: dial.dial().await?,
            process: None,
        }),
        (Network::Os, Endpoint::Attached(engine)) => attached(engine),
        (Network::Os, Endpoint::Daemon(endpoint)) => Ok(Opened {
            link: daemon(endpoint).await?,
            process: None,
        }),
        (Network::Os, Endpoint::Exposed(exposed)) => Ok(Opened {
            link: websocket::open(tcp(exposed.address).await?, exposed).await?,
            process: None,
        }),
        (Network::Os, Endpoint::InMemory) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the in-memory endpoint is on no OS network",
        )),
    }
}

/// Starts `engine` with exactly its environment, in a process group of its
/// own so a signal sent to the program's group never reaches it, killed if
/// the Connection is dropped with it still running (spec #606), its stderr
/// piped and never inherited.
fn attached(engine: &AttachedEngine) -> io::Result<Opened> {
    let mut command = tokio::process::Command::new(&engine.executable);
    command
        .arg(ATTACHED_ENGINE)
        .env_clear()
        .envs(&engine.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        /// `CREATE_NEW_PROCESS_GROUP`: Ctrl+C sent to the program's group
        /// never reaches the engine.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0060: the client's transport module starts the Attached engine it talks to"
    )]
    let mut child = command.spawn()?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(io::Error::other("the Attached engine's stdio is not piped"));
    };
    let stderr = child.stderr.take();
    let (exited, exit) = oneshot::channel();
    // A watch that ends without telling, its runtime shut down, leaves no
    // evidence.
    let exit = async {
        exit.await
            .unwrap_or_else(|_| EngineExitedDetails::without_evidence())
    };
    Ok(Opened {
        link: Link::new(stdout, stdin).ending_with(exit),
        process: Some(Process {
            child,
            stderr,
            exited,
        }),
    })
}

/// A Connection to the Daemon at `endpoint`.
async fn daemon(endpoint: &DaemonEndpoint) -> io::Result<Link> {
    match endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            #[expect(
                clippy::disallowed_types,
                reason = "ADR 0060: the client's transport module is the one place on the client side that opens sockets"
            )]
            let stream = tokio::net::UnixStream::connect(path).await?;
            let (reader, writer) = stream.into_split();
            Ok(Link::new(reader, writer))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(path) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{} is a Unix socket, which this OS's Daemon never listens on",
                path.display()
            ),
        )),
        DaemonEndpoint::Tcp(address) => tcp(*address).await,
    }
}

/// A Connection's bytes over loopback TCP to `address`.
async fn tcp(address: SocketAddr) -> io::Result<Link> {
    #[expect(
        clippy::disallowed_types,
        reason = "ADR 0060: the client's transport module is the one place on the client side that opens sockets"
    )]
    let stream = tokio::net::TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    let (reader, writer) = stream.into_split();
    Ok(Link::new(reader, writer))
}

/// An Attached engine's process.
pub(crate) struct Process {
    child: Child,
    stderr: Option<ChildStderr>,
    exited: oneshot::Sender<EngineExitedDetails>,
}

impl Process {
    /// Logs each line the engine writes to its stderr at warning, never on
    /// the program's own stderr, for as long as it runs, then tells its
    /// Connection how it exited, with its stderr's last 64 KiB.
    pub(crate) async fn watch(mut self) {
        let tail = match self.stderr.take() {
            Some(stderr) => read_stderr(stderr).await,
            None => Tail::default(),
        };
        let (exit_code, signal) = match self.child.wait().await {
            Ok(status) => {
                tracing::debug!(target: "goodfetch::client", %status, "the Attached engine exited");
                (status.code(), signal_of(status))
            }
            Err(error) => {
                tracing::warn!(target: "goodfetch::client", %error, "the Attached engine's exit could not be read");
                (None, None)
            }
        };
        let _unheard = self.exited.send(EngineExitedDetails {
            exit_code,
            signal,
            stderr_tail: tail.text(),
        });
    }
}

/// Reads `stderr` until the engine closes it, logging each line at
/// warning, and a line longer than the tail in pieces as long, and keeps
/// its last bytes.
async fn read_stderr(stderr: ChildStderr) -> Tail {
    let mut stderr = BufReader::new(stderr);
    let mut tail = Tail::default();
    let mut line = Vec::new();
    loop {
        let read = match stderr.fill_buf().await {
            Ok(read) => read,
            Err(error) => {
                tracing::warn!(target: "goodfetch::client", %error, "the Attached engine's stderr could not be read");
                break;
            }
        };
        if read.is_empty() {
            break;
        }
        let ends = read.iter().position(|byte| *byte == b'\n');
        let taken = ends.map_or(read.len(), |at| at + 1);
        line.extend_from_slice(&read[..taken]);
        tail.push(&read[..taken]);
        stderr.consume(taken);
        if ends.is_some() || line.len() >= STDERR_TAIL_BYTES {
            log_stderr(&line);
            line.clear();
        }
    }
    if !line.is_empty() {
        log_stderr(&line);
    }
    tail
}

/// Logs the line `line` the Attached engine wrote to its stderr.
fn log_stderr(line: &[u8]) {
    let line = String::from_utf8_lossy(line.strip_suffix(b"\n").unwrap_or(line));
    tracing::warn!(target: "goodfetch::client", %line, "the Attached engine wrote to its stderr");
}

/// The last bytes an Attached engine wrote to its stderr.
#[derive(Default)]
struct Tail(VecDeque<u8>);

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        self.0.extend(bytes);
        let over = self.0.len().saturating_sub(STDERR_TAIL_BYTES);
        self.0.drain(..over);
    }

    /// Its text, from the first character that starts in it: a cut may
    /// fall inside one, whose continuation bytes are `0b10xx_xxxx`.
    fn text(mut self) -> String {
        let bytes = self.0.make_contiguous();
        let start = bytes
            .iter()
            .position(|byte| byte & 0xC0 != 0x80)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[start..]).into_owned()
    }
}

/// The name of the signal that ended the process, such as `SIGKILL`, or
/// its number past the ones named here.
#[cfg(unix)]
fn signal_of(status: std::process::ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt as _;
    let signal = status.signal()?;
    let name = match signal {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGILL => "SIGILL",
        libc::SIGTRAP => "SIGTRAP",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGKILL => "SIGKILL",
        libc::SIGUSR1 => "SIGUSR1",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGUSR2 => "SIGUSR2",
        libc::SIGPIPE => "SIGPIPE",
        libc::SIGALRM => "SIGALRM",
        libc::SIGTERM => "SIGTERM",
        libc::SIGXCPU => "SIGXCPU",
        libc::SIGXFSZ => "SIGXFSZ",
        libc::SIGSYS => "SIGSYS",
        other => return Some(format!("signal {other}")),
    };
    Some(name.to_owned())
}

/// No signal ends a process on this OS.
#[cfg(not(unix))]
fn signal_of(_status: std::process::ExitStatus) -> Option<String> {
    None
}
