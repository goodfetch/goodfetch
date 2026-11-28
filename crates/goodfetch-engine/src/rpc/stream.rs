//! The streaming calls (ADRs 0024 and 0031): `events.subscribe`, which
//! sends a subscription's events in `$/events` batches, and `page.body`,
//! which sends a body's file in `$/chunk` notifications. Each sends no more
//! than the credits its Client granted it, and waits for more. A streaming
//! call of another module that sends its own events, such as `page.on`,
//! sends them with [`send_events`].

use std::convert::Infallible;
use std::io::Write as _;
use std::pin::Pin;
use std::task::{Context, Poll};

use goodfetch_protocol::events::{
    EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventsSubscribeParams, LastRead, StreamRedirect,
    SubscriptionEnded,
};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::{
    BODY_NOT_FOUND, BodyNotFoundDetails, PAGE_BODY, PageBodyParams, PageBodyRead,
};
use goodfetch_protocol::run::RunId;
use hyper::body::{Body, Bytes, Frame};
use serde::Serialize;
use tokio::sync::mpsc;

use super::record_event;
use crate::body;
use crate::call_error::CallError;
use crate::composition::{CallContext, CallStream, Handler};
use crate::engine_context::EventsRoute;
use crate::events::{Credits, Envelope, Events};
use crate::mask::{Masks, Scope};
use crate::tasks::TaskOwner;

/// The most events one `$/events` batch carries.
const MAX_BATCH: usize = 256;

/// Credit window for one Server-Sent Events reader (`GET /v1/events`, ADRs
/// 0024 and 0027): each frame polled by the HTTP response refunds one credit,
/// and dropping the response closes the channel and ends the subscription.
const SSE_CREDITS: u32 = 16;

/// The streaming calls' handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&EVENTS_SUBSCRIBE, subscribe).streaming(),
        Handler::new(&PAGE_BODY, read_body).streaming(),
    ]
}

async fn subscribe(
    params: EventsSubscribeParams,
    context: CallContext,
) -> Result<SubscriptionEnded, CallError> {
    let stream = context.stream();
    if let Some(ref run_id) = params.run_id
        && let Some(router) = context.engine().group_router()
    {
        match router.route_events(run_id.clone()).await? {
            EventsRoute::Local => {}
            EventsRoute::Redirect { url, owner } => {
                return Ok(SubscriptionEnded {
                    redirect: Some(StreamRedirect { url, owner }),
                });
            }
            EventsRoute::Forward {
                owner,
                peer_address,
            } => {
                let mut rx = router.forward_events(owner, peer_address, params).await?;
                let mut pending = std::collections::VecDeque::new();
                loop {
                    let credits = stream.credits.spend(MAX_BATCH).await;
                    if pending.is_empty() {
                        let Some(batch) = rx.recv().await else {
                            stream.credits.refund(credits);
                            return Ok(SubscriptionEnded::default());
                        };
                        pending.extend(batch);
                    }
                    let mut out = Vec::with_capacity(credits.min(pending.len()));
                    while out.len() < credits {
                        let Some(ev) = pending.pop_front() else {
                            break;
                        };
                        record_event(&ev.event);
                        out.push(ev);
                    }
                    stream.credits.refund(credits.saturating_sub(out.len()));
                    if out.is_empty() {
                        continue;
                    }
                    let batch = EventBatch {
                        id: stream.id().clone(),
                        events: out,
                    };
                    if !stream.notify_events(EVENTS, &batch).await {
                        return Ok(SubscriptionEnded::default());
                    }
                }
            }
        }
    }
    let Some(mut subscription) = context
        .events()
        .subscribe(params.after, params.system_info)
        .await
    else {
        return Ok(SubscriptionEnded::default());
    };
    loop {
        let credits = stream.credits.spend(MAX_BATCH).await;
        let Some(events) = subscription.next(credits).await else {
            return Ok(SubscriptionEnded::default());
        };
        stream.credits.refund(credits.saturating_sub(events.len()));
        if !send_events(stream, events).await {
            return Ok(SubscriptionEnded::default());
        }
    }
}

/// Sends `events`, the streaming call `stream`'s own, in one `$/events`
/// batch naming the call, each recorded as sent (ADR 0072); `false` once
/// the Connection's writer is gone. The caller spent a credit for each.
///
/// # Cancel safety
///
/// As [`CallStream::notify`]'s.
pub async fn send_events(stream: &CallStream, events: Vec<Envelope>) -> bool {
    for event in &events {
        record_event(event.event);
    }
    let batch = Batch {
        id: stream.id(),
        events: &events,
    };
    stream.notify_events(EVENTS, &batch).await
}

/// What `$/events` carries, an `EventBatch` on the wire, each event's
/// payload as it was encoded once.
#[derive(Serialize)]
struct Batch<'a> {
    id: &'a RequestId,
    events: &'a [Envelope],
}

/// One Server-Sent Events HTTP body streamed through `rpc`'s event-stream
/// writer and `Credits` (ADRs 0024, 0027 and 0140's E1).
pub(crate) struct SseStream {
    frames: mpsc::Receiver<Bytes>,
    credits: Credits,
}

impl Body for SseStream {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.frames.poll_recv(cx) {
            Poll::Ready(Some(bytes)) => {
                self.credits.grant(1);
                Poll::Ready(Some(Ok(Frame::data(bytes))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Opens a Server-Sent Events stream backed by `events` and `Credits`,
/// optionally filtered to `run` and resuming after `after`. Each event's
/// pre-encoded [`EventData`](crate::events::EventData) is spliced verbatim
/// without re-encoding (ADR 0140's E1).
///
/// # Cancel safety
///
/// Dropping the returned [`SseStream`] ends the pump task and unsubscribes.
pub(crate) async fn open_sse_stream(
    events: &Events,
    after: Option<LastRead>,
    run: Option<RunId>,
    masks: Masks,
    owner: &TaskOwner,
) -> Option<SseStream> {
    let mut subscription = events.subscribe_for_run(after, run, false).await?;
    let credits = Credits::new();
    credits.grant(SSE_CREDITS);
    let (sender, frames) = mpsc::channel(SSE_CREDITS as usize);
    let pump_credits = credits.clone();
    let stopping = owner.token().clone();
    owner.spawn(async move {
        loop {
            let granted = tokio::select! {
                biased;
                () = stopping.cancelled() => return,
                () = sender.closed() => return,
                granted = pump_credits.spend(MAX_BATCH) => granted,
            };
            let batch = tokio::select! {
                biased;
                () = stopping.cancelled() => return,
                () = sender.closed() => return,
                batch = subscription.next(granted) => match batch {
                    Some(batch) => batch,
                    None => return,
                },
            };
            pump_credits.refund(granted.saturating_sub(batch.len()));
            for envelope in batch {
                record_event(envelope.event);
                let frame = format_sse_frame(&envelope, &masks);
                let sent = tokio::select! {
                    biased;
                    () = stopping.cancelled() => false,
                    outcome = sender.send(frame) => outcome.is_ok(),
                };
                if !sent {
                    return;
                }
            }
        }
    });
    Some(SseStream { frames, credits })
}

/// Formats one [`Envelope`] as a Server-Sent Event frame (`id`, `event`,
/// `data`), splicing the pre-encoded `envelope.data.json()` verbatim so an
/// event is encoded once whichever transport carries it (ADR 0140's E1).
pub(crate) fn format_sse_frame(envelope: &Envelope, masks: &Masks) -> Bytes {
    let payload = envelope.data.json();
    let mut json_line = Vec::with_capacity(payload.len() + envelope.event.len() + 64);
    let _ = write!(
        json_line,
        "{{\"position\":{},\"event\":\"{}\",\"data\":{}",
        envelope.position.0, envelope.event, payload
    );
    if envelope.dropped > 0 {
        let _ = write!(json_line, ",\"dropped\":{}", envelope.dropped);
    }
    json_line.push(b'}');
    let masked = masks.apply_json_line(json_line, Scope::Returned);
    let mut wire = Vec::with_capacity(masked.len() + envelope.event.len() + 40);
    let _ = write!(
        wire,
        "id: {}\nevent: {}\ndata: ",
        envelope.position.0, envelope.event
    );
    wire.extend_from_slice(&masked);
    wire.extend_from_slice(b"\n\n");
    Bytes::from(wire)
}

async fn read_body(
    params: PageBodyParams,
    context: CallContext,
) -> Result<PageBodyRead, CallError> {
    let not_found = || {
        CallError::new(
            &BODY_NOT_FOUND,
            format!(
                "The Run `{}` holds no body `{}`.",
                params.run_id, params.body
            ),
            &BodyNotFoundDetails {
                run_id: params.run_id.clone(),
                body: params.body.clone(),
            },
        )
    };
    let folder = context
        .runs()
        .folder(params.run_id.clone())
        .await
        .map_err(|_| not_found())?;
    let file = body::file(&folder, &params.body).ok_or_else(not_found)?;
    let owner = context.engine().owner();
    let file = owner
        .blocking(move || {
            #[expect(
                clippy::disallowed_methods,
                reason = "ADR 0140: a file opened on no counted row yet, on the tasks module's blocking policy (issue 2106); the Shared paths audit moves it onto its counted function"
            )]
            std::fs::File::open(file)
        })
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => not_found(),
            // The Run holds the body: a failure to open it is the engine's.
            _ => CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("the body could not be opened: {error}"),
            ),
        })?;
    let size = context
        .stream()
        .send_file(file, owner)
        .await
        .map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("the body could not be read: {error}"),
            )
        })?;
    Ok(PageBodyRead { size })
}
