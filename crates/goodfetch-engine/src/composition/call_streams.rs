//! A call's ways to and from its Client (ADR 0024): the notifications that
//! name the call, a streaming call's sends under the credits its Client
//! grants, and the input a call takes, bounded by the room its handler
//! declares.
#![expect(
    clippy::disallowed_types,
    reason = "a poll cannot await: one Mutex guards a call's input receiver, held only while one poll of it runs, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::sync::{Mutex, PoisonError};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use goodfetch_protocol::page::{CHUNK, CHUNK_BYTES, Chunk};
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncReadExt as _;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::call_error::CallError;
use crate::events::Credits;

/// The way back to a call's Client: the notifications that name the call,
/// written as lines on its Connection's writer.
#[derive(Clone)]
pub(crate) struct CallNotifier {
    id: RequestId,
    outgoing: mpsc::Sender<Vec<u8>>,
    line: fn(&str, Value) -> Vec<u8>,
    work: CountedWork,
}

impl CallNotifier {
    /// The notifier of the call `id`, on the Connection's writer
    /// `outgoing`, which writes the line `line` makes of a notification's
    /// method and params, each encode counted on `work`.
    pub(crate) fn new(
        id: RequestId,
        outgoing: mpsc::Sender<Vec<u8>>,
        line: fn(&str, Value) -> Vec<u8>,
        work: CountedWork,
    ) -> Self {
        Self {
            id,
            outgoing,
            line,
            work,
        }
    }

    /// The notification `method` with `params` to the Client; `false` once
    /// the Connection's writer is gone. Dropping the future sends nothing
    /// unless the writer took the line.
    async fn notify(&self, method: &str, params: &impl Serialize) -> bool {
        self.work.count(Row::MessageEncodes);
        let params =
            serde_json::to_value(params).expect("invariant: a notification's params encode");
        self.outgoing
            .send((self.line)(method, params))
            .await
            .is_ok()
    }
}

/// What a streaming call sends its notifications with: the credits its
/// Client grants it, and the way back to its Client.
pub struct CallStream {
    notifier: CallNotifier,
    pub(crate) credits: Credits,
}

impl CallStream {
    /// The stream of the call `notifier` answers, under `credits`.
    pub(crate) fn new(notifier: CallNotifier, credits: Credits) -> Self {
        Self { notifier, credits }
    }

    /// The call's id, which its notifications name.
    pub fn id(&self) -> &RequestId {
        &self.notifier.id
    }

    /// Waits for at least one credit its Client grants, and spends at most
    /// `most`: how many notifications it may send now.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future spends nothing.
    pub async fn spend(&self, most: usize) -> usize {
        self.credits.spend(most).await
    }

    /// Gives back `credits` it spent on nothing.
    pub fn refund(&self, credits: usize) {
        self.credits.refund(credits);
    }

    /// Sends the Client the notification `method` with `params`; `false`
    /// once the Connection's writer is gone.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends nothing, unless the writer already took
    /// the line.
    pub async fn notify(&self, method: &str, params: &impl Serialize) -> bool {
        self.notifier.notify(method, params).await
    }

    /// Sends `file`, from where it is to its end, in `$/chunk`
    /// notifications of at most [`CHUNK_BYTES`], one per credit, each with
    /// its offset; answers how many bytes it sent, fewer than the file's
    /// rest once the Connection's writer is gone.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future stops the chunks; those sent stay sent.
    pub async fn send_file(&self, file: &mut tokio::fs::File) -> std::io::Result<u64> {
        let mut offset: u64 = 0;
        let mut chunk = vec![0; CHUNK_BYTES];
        loop {
            let read = read_chunk(file, &mut chunk).await?;
            if read == 0 {
                return Ok(offset);
            }
            let _ = self.spend(1).await;
            let sent = self
                .notify(
                    CHUNK,
                    &Chunk {
                        id: self.id().clone(),
                        offset,
                        data: data_encoding::BASE64.encode(&chunk[..read]),
                    },
                )
                .await;
            if !sent {
                return Ok(offset);
            }
            offset += u64::try_from(read).unwrap_or(u64::MAX);
        }
    }
}

/// Fills `chunk` from `file` as far as it goes, and answers how many bytes
/// it read: fewer than its length only at the file's end.
async fn read_chunk(file: &mut tokio::fs::File, chunk: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < chunk.len() {
        let read = file.read(&mut chunk[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// The stream of input a call takes from its Client (ADR 0024): each
/// notification of its handler's input method that names the call's id, in
/// the order they came, at most its handler's room of them unread, and the
/// way back to its Client for what it answers, such as the credits that
/// let the Client send more.
pub struct CallInput {
    notifier: CallNotifier,
    incoming: Mutex<mpsc::Receiver<Value>>,
    /// Cancelled once the Client sent more than `room` notifications the
    /// call had not read.
    overflowed: CancellationToken,
    room: usize,
}

/// The Connection's end of a call's input, which hands each input
/// notification over until the call's room is full.
pub(crate) struct InputSender {
    sender: Option<mpsc::Sender<Value>>,
    overflowed: CancellationToken,
}

impl InputSender {
    /// Hands `params` to the call; when the call has `room` notifications
    /// unread already, refuses its input instead (the call's next read
    /// fails with `invalid_params`) and ignores what comes after.
    pub(crate) fn hand_over(&mut self, params: Value) {
        let Some(sender) = &self.sender else {
            return;
        };
        if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(params) {
            self.overflowed.cancel();
            self.sender = None;
        }
    }
}

impl CallInput {
    /// The input of the call `notifier` answers, which leaves at most
    /// `room` of its notifications unread, and the Connection's end that
    /// hands them over.
    pub(crate) fn new(notifier: CallNotifier, room: usize) -> (InputSender, Self) {
        let (sender, incoming) = mpsc::channel(room.max(1));
        let overflowed = CancellationToken::new();
        (
            InputSender {
                sender: Some(sender),
                overflowed: overflowed.clone(),
            },
            Self {
                notifier,
                incoming: Mutex::new(incoming),
                overflowed,
                room,
            },
        )
    }

    /// The call's id, which its notifications name.
    pub fn id(&self) -> &RequestId {
        &self.notifier.id
    }

    /// The params of the next input notification; `None` once the
    /// Connection ended. Read by the call's handler alone, one read at a
    /// time.
    ///
    /// # Errors
    ///
    /// `invalid_params` once the Client sent more notifications than the
    /// call's room while the call had not read them, which a Client
    /// honouring the call's credits never does.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no notification.
    pub async fn next(&self) -> Result<Option<Value>, CallError> {
        if self.overflowed.is_cancelled() {
            return Err(self.overflow());
        }
        let next = std::future::poll_fn(|context| {
            self.incoming
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .poll_recv(context)
        })
        .await;
        match next {
            Some(params) => Ok(Some(params)),
            None if self.overflowed.is_cancelled() => Err(self.overflow()),
            None => Ok(None),
        }
    }

    /// `invalid_params` for a Client that sent more input than the call's
    /// room.
    fn overflow(&self) -> CallError {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "The Client sent more than {} input notifications the call had not read: a Client sends no more input than the call's credits allow.",
                self.room
            ),
        )
    }

    /// Sends the Client the notification `method` with `params`; `false`
    /// once the Connection's writer is gone.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future sends nothing, unless the writer already took
    /// the line.
    pub async fn notify(&self, method: &str, params: &impl Serialize) -> bool {
        self.notifier.notify(method, params).await
    }
}
