//! Every byte stream the client opens to an engine, taken from a network
//! value (ADR 0060): the OS's, or the in-memory one through which tests and
//! each Product's SDK crate reach an engine in the same process. On the OS's
//! network an Attached engine is a child process the client starts and
//! talks to over its stdio; a Daemon is the Unix socket or loopback TCP
//! address its endpoint names (ADR 0024), never a named pipe. This is the
//! one module on the client side that opens a socket or starts a process
//! (spec #606).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use goodfetch_protocol::cli::ATTACHED_ENGINE;
use goodfetch_protocol::connection::DaemonEndpoint;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, BufReader};
use tokio::process::{Child, ChildStderr};

/// One Connection's bytes, both ways.
pub struct Link {
    reader: Box<dyn AsyncRead + Send + Unpin>,
    writer: Box<dyn AsyncWrite + Send + Unpin>,
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
        }
    }

    /// Its two directions.
    pub(crate) fn split(
        self,
    ) -> (
        Box<dyn AsyncRead + Send + Unpin>,
        Box<dyn AsyncWrite + Send + Unpin>,
    ) {
        (self.reader, self.writer)
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
/// does not start, a Daemon's endpoint where nothing listens, an in-memory
/// network whose engine is gone, or the in-memory endpoint on the OS's
/// network.
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
    Ok(Opened {
        link: Link::new(stdout, stdin),
        process: Some(Process { child, stderr }),
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
        DaemonEndpoint::Tcp(address) => {
            #[expect(
                clippy::disallowed_types,
                reason = "ADR 0060: the client's transport module is the one place on the client side that opens sockets"
            )]
            let stream = tokio::net::TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let (reader, writer) = stream.into_split();
            Ok(Link::new(reader, writer))
        }
    }
}

/// An Attached engine's process.
pub(crate) struct Process {
    child: Child,
    stderr: Option<ChildStderr>,
}

impl Process {
    /// Logs each line the engine writes to its stderr, then its exit, for
    /// as long as it runs.
    pub(crate) async fn watch(mut self) {
        if let Some(stderr) = self.stderr.take() {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "goodfetch::client", %line, "the Attached engine wrote to its stderr");
            }
        }
        match self.child.wait().await {
            Ok(status) => {
                tracing::debug!(target: "goodfetch::client", %status, "the Attached engine exited");
            }
            Err(error) => {
                tracing::warn!(target: "goodfetch::client", %error, "the Attached engine's exit could not be read");
            }
        }
    }
}
