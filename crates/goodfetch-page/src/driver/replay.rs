//! A browser played in process for the driver's tests: the other end of an
//! in-process pipe to a real `goodfetch_cdp::Connection`, which reads the
//! client's commands and writes the browser's messages in the order its
//! pipe delivers them, several in one write when the order under test is
//! the one the connection's driver reads them in.

use goodfetch_cdp::{Connection, Driver, Framer};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, ReadHalf, WriteHalf};

/// The page's session.
pub(super) const SESSION: &str = "page";

/// The page's main frame.
pub(super) const FRAME: &str = "main";

/// The client's connection, and its driver, whose browser is played.
pub(super) type Client = (
    Connection,
    Driver<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>,
);

/// The played browser's end of the pipe.
pub(super) struct Played {
    pipe: DuplexStream,
    framer: Framer,
}

/// A connection to a browser that `Played` plays.
pub(super) fn connected() -> (Client, Played) {
    let (client, browser) = tokio::io::duplex(1 << 16);
    let (reader, writer) = tokio::io::split(client);
    (
        Connection::new(
            reader,
            writer,
            goodfetch_counted_work::CountedWork::default(),
        ),
        Played {
            pipe: browser,
            framer: Framer::default(),
        },
    )
}

/// One message as the browser's pipe frames it: JSON, then a NUL byte.
fn framed(message: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
    bytes.push(0);
    bytes
}

/// The page's event `method`, with its `params`.
pub(super) fn event(method: &str, params: Value) -> Value {
    let mut event = json!({ "method": method, "sessionId": SESSION });
    event["params"] = params;
    event
}

/// The answer to `command`, its `result`.
pub(super) fn answer(command: &Value, result: Value) -> Value {
    let mut answer = json!({ "id": command["id"], "sessionId": SESSION });
    answer["result"] = result;
    answer
}

impl Played {
    /// The next command the client wrote.
    pub(super) async fn command(&mut self) -> Value {
        loop {
            if let Some(bytes) = self
                .framer
                .next_frame()
                .expect("invariant: the client's frames fit")
            {
                return serde_json::from_slice(&bytes).expect("invariant: the client writes JSON");
            }
            let mut buffer = vec![0; 4096];
            let count = self
                .pipe
                .read(&mut buffer)
                .await
                .expect("invariant: the pipe reads");
            assert_ne!(count, 0, "the client closed its end before a command");
            self.framer.push(&buffer[..count]);
        }
    }

    /// Writes `messages` in one write, so that the client's driver reads
    /// them together; `false` once the client closed its end.
    pub(super) async fn send(&mut self, messages: &[Value]) -> bool {
        let bytes: Vec<u8> = messages.iter().flat_map(framed).collect();
        self.pipe.write_all(&bytes).await.is_ok()
    }

    /// Holds the pipe open until the client closed its end, as a browser
    /// keeps it, leaving what the client writes meanwhile unanswered.
    pub(super) async fn hold(mut self) {
        let mut buffer = vec![0; 4096];
        while self
            .pipe
            .read(&mut buffer)
            .await
            .is_ok_and(|count| count > 0)
        {}
    }
}
