//! A Chromium NetLog read back for the quiet-browser test (ADR 0074): a
//! launch run with `--log-net-log=<file>` writes every event of its network
//! stack to that file, and [`NetLog::read`] lists every socket the launch
//! connected, so the test can hold that a launch, an idle minute and a
//! Proving ground navigation reach the Proving ground alone.
//!
//! A socket is each TCP connection attempt (`TCP_CONNECT_ATTEMPT`) and each
//! UDP socket connected (`UDP_CONNECT`), with the address it went to,
//! whatever came of it: an attempt that failed still sent its packets. The
//! file must be whole, as the browser writes it when it exits on its own or
//! on an interrupt; one cut short, as a killed browser leaves it, is
//! refused, since its last sockets would be missing.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the browser's NetLog file in the test's thread, never on an async worker"
)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The event a TCP connection attempt begins with, its address in its
/// parameters.
const TCP_ATTEMPT: &str = "TCP_CONNECT_ATTEMPT";

/// The event a UDP socket's connection begins with, its address in its
/// parameters.
const UDP_CONNECT: &str = "UDP_CONNECT";

/// The phase an event that begins something has.
const PHASE_BEGIN: &str = "PHASE_BEGIN";

/// Every socket a browser launch connected, from its NetLog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetLog {
    /// Each socket, in the order the launch connected them.
    sockets: Vec<NetLogSocket>,
}

/// One socket a launch connected: a TCP connection attempt or a connected
/// UDP socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetLogSocket {
    /// Over TCP or UDP.
    pub transport: SocketTransport,
    /// The address it went to.
    pub address: SocketAddr,
    /// The NetLog source that opened it: the socket, which a test can look
    /// up in the file. Chromium numbers some sources below zero, such as
    /// those another process logged.
    pub source: i64,
}

/// The transport of a [`NetLogSocket`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketTransport {
    /// A TCP connection attempt.
    Tcp,
    /// A connected UDP socket, such as QUIC's or a DNS query's.
    Udp,
}

/// Why a NetLog could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NetLogError {
    /// The file could not be opened or read.
    #[error("the NetLog at {path} cannot be read: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What reading it gave.
        source: io::Error,
    },
    /// The file ends before the NetLog does, as a killed browser leaves it.
    #[error(
        "the NetLog at {0} is cut short: the browser did not finish writing it, so its last sockets may be missing"
    )]
    Truncated(PathBuf),
    /// The file is whole but no NetLog, or a connection's event names no
    /// address.
    #[error("the NetLog at {path} cannot be read: {reason}")]
    Malformed {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
}

/// A NetLog file as Chromium writes it: the constants that name each event
/// type and phase by number, then the events.
#[derive(Deserialize)]
struct NetLogFile {
    constants: Constants,
    events: Vec<Event>,
}

/// The numbers the events use for their types and phases.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Constants {
    log_event_types: BTreeMap<String, u64>,
    log_event_phase: BTreeMap<String, u64>,
}

/// One event of the file.
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: u64,
    source: Source,
    phase: u64,
    #[serde(default)]
    params: Option<Params>,
}

/// The source an event belongs to.
#[derive(Deserialize)]
struct Source {
    id: i64,
}

/// The parameters of an event, of which a connection's give its address.
#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    address: Option<String>,
}

impl NetLog {
    /// Reads the NetLog file at `path` and lists every socket it holds.
    ///
    /// # Errors
    ///
    /// `Io` when the file cannot be read, `Truncated` when it ends before
    /// the NetLog does, and `Malformed` when it is no NetLog or a
    /// connection's event gives no address.
    pub fn read(path: &Path) -> Result<Self, NetLogError> {
        let malformed = |reason: String| NetLogError::Malformed {
            path: path.to_owned(),
            reason,
        };
        let file = File::open(path).map_err(|source| NetLogError::Io {
            path: path.to_owned(),
            source,
        })?;
        let log: NetLogFile = serde_json::from_reader(BufReader::new(file)).map_err(|error| {
            if error.is_eof() {
                NetLogError::Truncated(path.to_owned())
            } else if error.is_io() {
                NetLogError::Io {
                    path: path.to_owned(),
                    source: error.into(),
                }
            } else {
                malformed(error.to_string())
            }
        })?;
        let number = |table: &BTreeMap<String, u64>, name: &str| {
            table
                .get(name)
                .copied()
                .ok_or_else(|| malformed(format!("its constants name no {name}")))
        };
        let tcp = number(&log.constants.log_event_types, TCP_ATTEMPT)?;
        let udp = number(&log.constants.log_event_types, UDP_CONNECT)?;
        let begin = number(&log.constants.log_event_phase, PHASE_BEGIN)?;
        let mut sockets = Vec::new();
        for event in &log.events {
            let transport = match event.kind {
                kind if kind == tcp => SocketTransport::Tcp,
                kind if kind == udp => SocketTransport::Udp,
                _ => continue,
            };
            if event.phase != begin {
                continue;
            }
            let name = if transport == SocketTransport::Tcp {
                TCP_ATTEMPT
            } else {
                UDP_CONNECT
            };
            let address = event
                .params
                .as_ref()
                .and_then(|params| params.address.as_deref())
                .ok_or_else(|| {
                    malformed(format!(
                        "the {name} of source {} gives no address",
                        event.source.id
                    ))
                })?;
            let address = address.parse().map_err(|_| {
                malformed(format!(
                    "the {name} of source {} gives the address {address:?}, which is no IP address and port",
                    event.source.id
                ))
            })?;
            sockets.push(NetLogSocket {
                transport,
                address,
                source: event.source.id,
            });
        }
        Ok(Self { sockets })
    }

    /// Each socket the launch connected, in order.
    pub fn sockets(&self) -> &[NetLogSocket] {
        &self.sockets
    }
}
