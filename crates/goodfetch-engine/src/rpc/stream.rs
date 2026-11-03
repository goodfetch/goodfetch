//! The streaming calls (ADRs 0024 and 0031): `events.subscribe`, which
//! sends a subscription's events in `$/events` batches, and `page.body`,
//! which sends a body's file in `$/chunk` notifications. Each sends no more
//! than the credits its Client granted it, and waits for more. A streaming
//! call of another module that sends its own events, such as `page.on`,
//! sends them with [`send_events`].

use goodfetch_protocol::events::{
    EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventEnvelope, EventsSubscribeParams, SubscriptionEnded,
};
use goodfetch_protocol::page::{
    BODY_NOT_FOUND, BodyNotFoundDetails, PAGE_BODY, PageBodyParams, PageBodyRead,
};

use super::record_event;
use crate::body;
use crate::composition::{CallContext, CallError, CallStream, Handler};

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
pub async fn send_events(stream: &CallStream, events: Vec<EventEnvelope>) -> bool {
    for event in &events {
        record_event(&event.event);
    }
    let batch = EventBatch {
        id: stream.id().clone(),
        events,
    };
    stream.notify(EVENTS, &batch).await
}

#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
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
    let mut file = tokio::fs::File::open(&file)
        .await
        .map_err(|_| not_found())?;
    let size = context
        .stream()
        .send_file(&mut file)
        .await
        .map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("the body could not be read: {error}"),
            )
        })?;
    Ok(PageBodyRead { size })
}
