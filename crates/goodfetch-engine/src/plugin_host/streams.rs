//! Credit-controlled opaque `net.connect` streams carried as `net.stream.*`
//! JSON-RPC notifications over a Plugin's stdio channel (ADR 0033, issue 833).
//!
//! Every stream is bound to the `call_id` (and therefore the lease) it was
//! opened under: when that call finishes, is cancelled, or its lease ends, the
//! stream's child cancellation token fires, closing the TCP socket, emitting
//! `net.stream.close`, and accounting total bytes and duration once through
//! [`StreamAccountant`].
#![expect(
    clippy::disallowed_types,
    reason = "`net.connect` bridges validated TCP streams opened behind `PluginFetch::connect` (ADRs 0033 and 0060)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::net::{
    NET_STREAM_CLOSE, NET_STREAM_CREDITS, NET_STREAM_DATA, NetFetchBody, NetStreamCloseParams,
    NetStreamCreditsParams, NetStreamDataParams,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::plugin_gate::proxy::{
    StreamAccountant, StreamRecord, StreamTransport, format_destination,
};
use crate::tasks::TaskOwner;
use crate::wire;

/// Initial credit window in bytes granted in each direction when a
/// `net.connect` stream opens.
pub(super) const INITIAL_CREDITS: u64 = 64 * 1024;

/// Maximum raw payload bytes sent in a single `net.stream.data` frame
/// (Base64-encoded stays comfortably below `MAX_MESSAGE_BYTES`).
const CHUNK_BYTES: usize = 16 * 1024;

/// Bounded per-stream channel capacity for Plugin -> Engine write chunks.
const WRITE_QUEUE: usize = 16;

/// Bounded channel capacity for outbound stream events to the instance loop.
pub(super) const STREAM_EVENTS: usize = 64;

/// Events delivered from spawned stream tasks back to the instance loop.
pub(super) enum StreamEvent {
    /// A `net.connect` request completed (connected TCP stream or refusal).
    Connected {
        reply_id: RequestId,
        call_id: RequestId,
        host: String,
        port: u16,
        outcome: Option<Result<TcpStream, CallError>>,
    },
    /// Outbound `net.stream.data` line to write to the Plugin's stdin.
    Frame(Vec<u8>),
    /// Outbound `net.stream.credits` line after the writer flushed `bytes`.
    CreditsReturned {
        stream_id: String,
        bytes: u64,
        frame: Vec<u8>,
    },
    /// Outbound `net.stream.close` line after the stream ended.
    Closed { stream_id: String, frame: Vec<u8> },
}

/// One active `net.connect` stream owned by a Plugin instance.
struct ActiveStream {
    /// Writes opaque chunks from the Plugin to the TCP socket's write task.
    writes: mpsc::Sender<Vec<u8>>,
    /// Remaining write credits the Plugin is permitted to send to the engine.
    plugin_send_credits: u64,
    /// Shared credit counter granted by the Plugin for Engine -> Plugin reads.
    engine_send_credits: Arc<AtomicU64>,
    /// Wakes the TCP reader task when the Plugin grants more read credits.
    read_credits_notify: Arc<Notify>,
    /// Cancels both reader and writer tasks when the stream or lease closes.
    cancel: CancellationToken,
}

/// Active `net.connect` streams for one Plugin instance.
pub(super) struct StreamRegistry {
    next_id: u64,
    streams: BTreeMap<String, ActiveStream>,
    outbound: mpsc::Sender<StreamEvent>,
}

impl StreamRegistry {
    /// Creates an empty registry whose stream events go to `outbound`.
    pub(super) fn new(outbound: mpsc::Sender<StreamEvent>) -> Self {
        Self {
            next_id: 0,
            streams: BTreeMap::new(),
            outbound,
        }
    }

    /// Sender handle for spawned `net.connect` tasks.
    pub(super) fn sender(&self) -> mpsc::Sender<StreamEvent> {
        self.outbound.clone()
    }

    /// Registers a newly connected TCP stream bound to `lease_token` (child of
    /// the call's serving token), spawns its reader and writer tasks on
    /// `owner`, and returns its unique `stream_id`.
    #[expect(
        clippy::too_many_arguments,
        reason = "binds all stream resources (owner, plugin name, destination, socket, lease token, clock, accountant) once at open"
    )]
    pub(super) fn open(
        &mut self,
        owner: &TaskOwner,
        plugin: String,
        host: &str,
        port: u16,
        tcp: TcpStream,
        lease_token: CancellationToken,
        clock: Clock,
        accountant: Option<Arc<dyn StreamAccountant>>,
    ) -> String {
        self.next_id = self.next_id.saturating_add(1);
        let stream_id = format!("s-{}", self.next_id);
        let destination = format_destination(host, port);
        let cancel = lease_token.child_token();
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(WRITE_QUEUE);
        let engine_send_credits = Arc::new(AtomicU64::new(INITIAL_CREDITS));
        let read_credits_notify = Arc::new(Notify::new());

        let (reader, writer) = tcp.into_split();
        let total_bytes = Arc::new(AtomicU64::new(0));
        let started = clock.now();

        let write_cancel = cancel.clone();
        let write_outbound = self.outbound.clone();
        let write_id = stream_id.clone();
        let write_bytes = Arc::clone(&total_bytes);
        owner.spawn(async move {
            run_writer(
                writer,
                write_rx,
                write_cancel,
                write_outbound,
                write_id,
                write_bytes,
            )
            .await;
        });

        let read_cancel = cancel.clone();
        let read_outbound = self.outbound.clone();
        let read_id = stream_id.clone();
        let read_credits = Arc::clone(&engine_send_credits);
        let read_notify = Arc::clone(&read_credits_notify);
        let read_bytes = Arc::clone(&total_bytes);
        owner.spawn(async move {
            run_reader(
                reader,
                read_cancel,
                lease_token,
                read_outbound,
                read_id,
                read_credits,
                read_notify,
                read_bytes,
                StreamCloseContext {
                    plugin,
                    destination,
                    clock,
                    started,
                    accountant,
                },
            )
            .await;
        });

        self.streams.insert(
            stream_id.clone(),
            ActiveStream {
                writes: write_tx,
                plugin_send_credits: INITIAL_CREDITS,
                engine_send_credits,
                read_credits_notify,
                cancel,
            },
        );
        stream_id
    }

    /// Handles a `net.stream.data` notification from the Plugin.
    pub(super) async fn on_data(&mut self, params: NetStreamDataParams) {
        let Some(stream) = self.streams.get_mut(&params.stream_id) else {
            return;
        };
        let bytes = params.data.into_bytes();
        let len = bytes.len() as u64;
        if len > stream.plugin_send_credits {
            stream.cancel.cancel();
            self.streams.remove(&params.stream_id);
            let frame = close_frame(&params.stream_id, Some("credits_exceeded"));
            let _ = self.outbound.try_send(StreamEvent::Closed {
                stream_id: params.stream_id,
                frame,
            });
            return;
        }
        stream.plugin_send_credits -= len;
        let writes = stream.writes.clone();
        let cancel = stream.cancel.clone();
        let sent = cancel.run_until_cancelled(writes.send(bytes)).await;
        if matches!(sent, Some(Err(_))) {
            cancel.cancel();
            self.streams.remove(&params.stream_id);
        }
    }

    /// Handles a `net.stream.credits` notification from the Plugin, granting
    /// additional bytes the engine may send on `net.stream.data`.
    pub(super) fn on_credits(&mut self, params: &NetStreamCreditsParams) {
        if let Some(stream) = self.streams.get(&params.stream_id) {
            stream
                .engine_send_credits
                .fetch_add(params.bytes, Ordering::Relaxed);
            stream.read_credits_notify.notify_one();
        }
    }

    /// Handles a `net.stream.close` notification from the Plugin.
    pub(super) fn on_close(&mut self, params: &NetStreamCloseParams) {
        if let Some(stream) = self.streams.remove(&params.stream_id) {
            stream.cancel.cancel();
        }
    }

    /// Replenishes `plugin_send_credits` after the writer task flushed bytes.
    pub(super) fn credit_returned(&mut self, stream_id: &str, bytes: u64) {
        if let Some(stream) = self.streams.get_mut(stream_id) {
            stream.plugin_send_credits = stream.plugin_send_credits.saturating_add(bytes);
        }
    }

    /// Removes a closed stream from the registry.
    pub(super) fn remove(&mut self, stream_id: &str) {
        self.streams.remove(stream_id);
    }

    /// Cancels all active streams when the instance stops.
    pub(super) fn close_all(&mut self) {
        for (_, stream) in std::mem::take(&mut self.streams) {
            stream.cancel.cancel();
        }
    }
}

struct StreamCloseContext {
    plugin: String,
    destination: String,
    clock: Clock,
    started: tokio::time::Instant,
    accountant: Option<Arc<dyn StreamAccountant>>,
}

async fn run_writer(
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    mut rx: mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
    outbound: mpsc::Sender<StreamEvent>,
    stream_id: String,
    total_bytes: Arc<AtomicU64>,
) {
    loop {
        let next = cancel.run_until_cancelled(rx.recv()).await;
        let Some(Some(chunk)) = next else {
            break;
        };
        let len = chunk.len() as u64;
        let written = cancel.run_until_cancelled(writer.write_all(&chunk)).await;
        if let Some(Ok(())) = written {
            total_bytes.fetch_add(len, Ordering::Relaxed);
            let frame = credits_frame(&stream_id, len);
            if outbound
                .send(StreamEvent::CreditsReturned {
                    stream_id: stream_id.clone(),
                    bytes: len,
                    frame,
                })
                .await
                .is_err()
            {
                break;
            }
        } else {
            cancel.cancel();
            break;
        }
    }
    let _ = writer.shutdown().await;
}

#[expect(
    clippy::too_many_arguments,
    reason = "reader task owns its stream halves, flow-control primitives, and close accounting context"
)]
async fn run_reader(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    cancel: CancellationToken,
    lease_token: CancellationToken,
    outbound: mpsc::Sender<StreamEvent>,
    stream_id: String,
    credits: Arc<AtomicU64>,
    notify: Arc<Notify>,
    total_bytes: Arc<AtomicU64>,
    ctx: StreamCloseContext,
) {
    let mut buf = vec![0_u8; CHUNK_BYTES];
    let reason: Option<&'static str> = loop {
        let available = loop {
            if cancel.is_cancelled() {
                break None;
            }
            let current = credits.load(Ordering::Relaxed);
            if current > 0 {
                break Some(
                    usize::try_from(current)
                        .unwrap_or(CHUNK_BYTES)
                        .min(CHUNK_BYTES),
                );
            }
            if cancel
                .run_until_cancelled(notify.notified())
                .await
                .is_none()
            {
                break None;
            }
        };
        let Some(limit) = available else {
            break if lease_token.is_cancelled() {
                Some("lease_ended")
            } else {
                None
            };
        };
        let read = cancel
            .run_until_cancelled(reader.read(&mut buf[..limit]))
            .await;
        match read {
            None => {
                break if lease_token.is_cancelled() {
                    Some("lease_ended")
                } else {
                    None
                };
            }
            Some(Ok(0)) => break None,
            Some(Ok(n)) => {
                let n_u64 = n as u64;
                credits.fetch_sub(n_u64, Ordering::Relaxed);
                total_bytes.fetch_add(n_u64, Ordering::Relaxed);
                let frame = data_frame(&stream_id, buf[..n].to_vec());
                if outbound.send(StreamEvent::Frame(frame)).await.is_err() {
                    break None;
                }
            }
            Some(Err(_)) => break Some("io_error"),
        }
    };

    cancel.cancel();
    let frame = close_frame(&stream_id, reason);
    let _ = outbound
        .send(StreamEvent::Closed {
            stream_id: stream_id.clone(),
            frame,
        })
        .await;

    if let Some(accountant) = ctx.accountant {
        let elapsed = ctx.clock.now().saturating_duration_since(ctx.started);
        let duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        accountant
            .record(StreamRecord {
                plugin: ctx.plugin,
                destination: ctx.destination,
                transport: StreamTransport::NetConnect,
                bytes: total_bytes.load(Ordering::Relaxed),
                duration_ms,
            })
            .await;
    }
}

fn data_frame(stream_id: &str, bytes: Vec<u8>) -> Vec<u8> {
    wire::notification_line(
        NET_STREAM_DATA,
        &NetStreamDataParams {
            stream_id: stream_id.to_owned(),
            data: NetFetchBody::new(bytes),
        },
    )
}

fn credits_frame(stream_id: &str, bytes: u64) -> Vec<u8> {
    wire::notification_line(
        NET_STREAM_CREDITS,
        &NetStreamCreditsParams {
            stream_id: stream_id.to_owned(),
            bytes,
        },
    )
}

fn close_frame(stream_id: &str, reason: Option<&str>) -> Vec<u8> {
    wire::notification_line(
        NET_STREAM_CLOSE,
        &NetStreamCloseParams {
            stream_id: stream_id.to_owned(),
            reason: reason.map(str::to_owned),
        },
    )
}
