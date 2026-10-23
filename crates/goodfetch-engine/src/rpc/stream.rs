//! The streaming calls (ADRs 0024 and 0031): `events.subscribe`, which
//! sends a subscription's events in `$/events` batches, and `page.body`,
//! which sends a body's file in `$/chunk` notifications. Each sends no more
//! than the credits its Client granted it, and waits for more.

use goodfetch_protocol::events::{
    EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventsSubscribeParams, SubscriptionEnded,
};
use goodfetch_protocol::page::{
    BODY_NOT_FOUND, BodyNotFoundDetails, CHUNK, CHUNK_BYTES, Chunk, PAGE_BODY, PageBodyParams,
    PageBodyRead,
};
use tokio::io::AsyncReadExt as _;

use super::{notification_line, record_event};
use crate::body;
use crate::composition::{CallContext, CallError, Handler};

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
    let Some(mut subscription) = context.events().subscribe(params.after).await else {
        return Ok(SubscriptionEnded {});
    };
    loop {
        let credits = stream.credits.spend(MAX_BATCH).await;
        let Some(events) = subscription.next(credits).await else {
            return Ok(SubscriptionEnded {});
        };
        stream.credits.refund(credits.saturating_sub(events.len()));
        for event in &events {
            record_event(&event.event);
        }
        let batch = EventBatch {
            id: stream.id.clone(),
            events,
        };
        if stream
            .outgoing
            .send(notification_line(EVENTS, &batch))
            .await
            .is_err()
        {
            return Ok(SubscriptionEnded {});
        }
    }
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
    let mut file = tokio::fs::File::open(&file)
        .await
        .map_err(|_| not_found())?;
    let stream = context.stream();
    let mut offset: u64 = 0;
    let mut chunk = vec![0; CHUNK_BYTES];
    loop {
        let read = read_chunk(&mut file, &mut chunk).await.map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("the body could not be read: {error}"),
            )
        })?;
        if read == 0 {
            return Ok(PageBodyRead { size: offset });
        }
        let _ = stream.credits.spend(1).await;
        let line = notification_line(
            CHUNK,
            &Chunk {
                id: stream.id.clone(),
                offset,
                data: data_encoding::BASE64.encode(&chunk[..read]),
            },
        );
        if stream.outgoing.send(line).await.is_err() {
            return Ok(PageBodyRead { size: offset });
        }
        offset += u64::try_from(read).unwrap_or(u64::MAX);
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
