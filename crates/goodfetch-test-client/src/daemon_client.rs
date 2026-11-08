//! A test's Client Connection to a Daemon, or to an Attached engine's
//! private endpoint (ADRs 0024 and 0031): a Unix socket on macOS and Linux,
//! loopback TCP on Windows, as the endpoint's runtime file names it, one
//! JSON-RPC message per line.
#![expect(
    clippy::disallowed_types,
    reason = "a test's own Client reaches the Daemon's socket from outside the engine (ADR 0060)"
)]

use std::io::{BufRead as _, BufReader, ErrorKind, Read, Write};
use std::time::Duration;

use goodfetch_test_support::DaemonEndpoint;
use serde_json::{Value, json};

use crate::handler_request::is_handler_request;
use crate::hello::Hello;

/// One Client's Connection.
pub struct DaemonClient {
    reader: BufReader<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    /// How long a read waits for the next message, when bounded.
    deadline: Option<Duration>,
}

impl DaemonClient {
    /// Connects to `endpoint`, without a word; each read waits as long as
    /// it takes.
    ///
    /// # Panics
    ///
    /// When nothing listens there.
    pub fn connect(endpoint: &DaemonEndpoint) -> Self {
        Self::connected(endpoint, None)
    }

    /// Connects as [`DaemonClient::connect`] does, each read waiting at
    /// most `deadline`.
    pub fn connect_within(endpoint: &DaemonEndpoint, deadline: Duration) -> Self {
        Self::connected(endpoint, Some(deadline))
    }

    /// Connects to `endpoint` and says `hello`, which the engine must
    /// answer with a result.
    ///
    /// # Panics
    ///
    /// When the engine refuses the `hello` or closes the Connection.
    pub fn opened(endpoint: &DaemonEndpoint, hello: &Hello) -> Self {
        let mut client = Self::connect(endpoint);
        let answer = client.open(hello);
        assert!(answer.get("result").is_some(), "{answer}");
        client
    }

    /// Whether the engine at `endpoint` closes, unanswered, a Connection
    /// whose `hello` is `hello`, such as one carrying another launch's
    /// token.
    pub fn refuses(endpoint: &DaemonEndpoint, hello: &Hello) -> bool {
        let mut client = Self::connect(endpoint);
        client.send(&hello.message());
        client.next().is_none()
    }

    fn connected(endpoint: &DaemonEndpoint, deadline: Option<Duration>) -> Self {
        let (reader, writer): (Box<dyn Read + Send>, Box<dyn Write + Send>) = match endpoint {
            #[cfg(unix)]
            DaemonEndpoint::Unix(path) => {
                let stream =
                    std::os::unix::net::UnixStream::connect(path).unwrap_or_else(|error| {
                        panic!("{} cannot be reached: {error}", path.display())
                    });
                stream
                    .set_read_timeout(deadline)
                    .expect("a socket takes a read timeout");
                (
                    Box::new(stream.try_clone().expect("a socket clones")),
                    Box::new(stream),
                )
            }
            #[cfg(not(unix))]
            DaemonEndpoint::Unix(_) => unreachable!("a Daemon listens on loopback TCP here"),
            DaemonEndpoint::Tcp(address) => {
                let stream = std::net::TcpStream::connect(address)
                    .unwrap_or_else(|error| panic!("{address} cannot be reached: {error}"));
                stream
                    .set_read_timeout(deadline)
                    .expect("a socket takes a read timeout");
                (
                    Box::new(stream.try_clone().expect("a socket clones")),
                    Box::new(stream),
                )
            }
        };
        Self {
            reader: BufReader::new(reader),
            writer,
            deadline,
        }
    }

    /// Says `hello` and answers the engine's reply to it.
    ///
    /// # Panics
    ///
    /// When the engine closes the Connection first.
    pub fn open(&mut self, hello: &Hello) -> Value {
        let message = hello.message();
        self.send(&message);
        self.reply_to(&message["id"])
    }

    /// Sends `message`.
    ///
    /// # Panics
    ///
    /// When the Connection is closed.
    pub fn send(&mut self, message: &Value) {
        writeln!(self.writer, "{message}")
            .and_then(|()| self.writer.flush())
            .expect("the Connection takes the message");
    }

    /// Sends `message` unless the engine closed the Connection already.
    pub fn try_send(&mut self, message: &Value) {
        // A Connection the engine closed refuses it, which is what the
        // caller is asking about.
        let _ = writeln!(self.writer, "{message}").and_then(|()| self.writer.flush());
    }

    /// The next message, or `None` once the engine closed the Connection.
    ///
    /// # Panics
    ///
    /// When nothing comes within the deadline the Connection was made
    /// with: a read past the socket's timeout fails with `WouldBlock` on
    /// Unix and `TimedOut` on Windows.
    #[expect(
        clippy::should_implement_trait,
        reason = "each read waits up to the deadline and panics past it, which an Iterator's `next` would hide"
    )]
    pub fn next(&mut self) -> Option<Value> {
        let mut line = String::new();
        let read = match self.reader.read_line(&mut line) {
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                panic!("the engine sent nothing for {:?}", self.deadline)
            }
            // The engine dropped the Connection, whether its close reached
            // the read or reset it. Anything else, bytes that are not text
            // among them, is no close.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::BrokenPipe
                ) =>
            {
                0
            }
            Err(error) => panic!("reading from the engine failed: {error}"),
            Ok(read) => read,
        };
        (read > 0)
            .then(|| serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: {line}")))
    }

    /// Sends the call `id` of `method` with `params` and answers its reply,
    /// skipping every other message.
    pub fn call(&mut self, id: impl Into<Value>, method: &str, params: &Value) -> Value {
        self.call_handling(id, method, params, &mut |_| None)
    }

    /// Sends the call `id`, answers each `crawl_request.handle` request
    /// with what `handler` makes of its params (none when it gives none),
    /// skips every other message, and answers the call's reply.
    ///
    /// # Panics
    ///
    /// When the engine closes the Connection before replying.
    pub fn call_handling(
        &mut self,
        id: impl Into<Value>,
        method: &str,
        params: &Value,
        handler: &mut impl FnMut(&Value) -> Option<Value>,
    ) -> Value {
        let id = id.into();
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next().expect("the engine closed before replying");
            if is_handler_request(&message) {
                if let Some(mut reply) = handler(&message["params"]) {
                    reply["jsonrpc"] = json!("2.0");
                    reply["id"] = message["id"].clone();
                    self.send(&reply);
                }
            } else if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        }
    }

    /// Reads on until the reply to `id`.
    fn reply_to(&mut self, id: &Value) -> Value {
        loop {
            let message = self.next().expect("the engine closed before replying");
            if message["id"] == *id && message.get("method").is_none() {
                return message;
            }
        }
    }
}
