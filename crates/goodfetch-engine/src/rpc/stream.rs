//! The streaming calls (ADRs 0024 and 0031): `events.subscribe`, which
//! sends a subscription's events in `$/events` batches, and `page.body`,
//! which sends a body's file in `$/chunk` notifications. Each sends no more
//! than the credits its Client granted it, and waits for more. A streaming
//! call of another module that sends its own events, such as `page.on`,
//! sends them with [`send_events`].

use goodfetch_protocol::events::{
    EVENTS, EVENTS_SUBSCRIBE, EventsSubscribeParams, SubscriptionEnded,
};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::{
    BODY_NOT_FOUND, BodyNotFoundDetails, PAGE_BODY, PageBodyParams, PageBodyRead,
};
use serde::Serialize;

use super::record_event;
use crate::body;
use crate::call_error::CallError;
use crate::composition::{CallContext, CallStream, Handler};
use crate::events::Envelope;

/// The most events one `$/events` batch carries.
const MAX_BATCH: usize = 256;

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
    let Some(mut subscription) = context
        .events()
        .subscribe(params.after, params.system_info)
        .await
    else {
        return Ok(SubscriptionEnded {});
    };
    loop {
        let credits = stream.credits.spend(MAX_BATCH).await;
        let Some(events) = subscription.next(credits).await else {
            return Ok(SubscriptionEnded {});
        };
        stream.credits.refund(credits.saturating_sub(events.len()));
        if !send_events(stream, events).await {
            return Ok(SubscriptionEnded {});
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
