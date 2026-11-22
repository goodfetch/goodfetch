//! A Run's progress as a stream (ADR 0102): `run.progress`, a followed
//! read. Each read is sent once the program asks for the next value, with
//! the last value's `position` as `after`, and the engine answers it with
//! the latest value once a newer one comes, so values the program did not
//! read in time coalesce. The value whose `last` is true ends it.

use std::future::IntoFuture as _;
use std::pin::Pin;
use std::sync::PoisonError;
use std::task::{Context, Poll, ready};

use goodfetch_protocol::run::{RUN_PROGRESS_READ, RunId, RunProgress, RunProgressParams};

use crate::answer::Answer;
use crate::call::Reply;
use crate::error::Error;
use crate::host::Host;
use crate::sync::Mutex;

/// The reads of a Run's progress, which an [`crate::EventStream`] hands.
pub(super) struct Following {
    host: Host<RunId>,
    /// The position of the value read last, the next read's `after`.
    after: Option<u64>,
    /// The read in flight, once one was asked for. The lock only makes the
    /// stream `Sync`: the read is polled through `get_mut`, never locked.
    read: Option<Mutex<Reply<Answer<RunProgress>>>>,
    ended: bool,
}

impl Following {
    /// The reads of the Run `host` names, from its progress now.
    pub(super) fn new(host: Host<RunId>) -> Self {
        Self {
            host,
            after: None,
            read: None,
            ended: false,
        }
    }
}

impl futures_core::Stream for Following {
    type Item = Result<Answer<RunProgress>, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let following = self.get_mut();
        if following.ended {
            return Poll::Ready(None);
        }
        let read = following.read.get_or_insert_with(|| {
            let params = RunProgressParams {
                run_id: following.host.id().clone(),
                after: following.after,
            };
            Mutex::new(
                following
                    .host
                    .call(RUN_PROGRESS_READ.name, params)
                    .into_future(),
            )
        });
        let answer = ready!(
            read.get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .as_mut()
                .poll(context)
        );
        following.read = None;
        match &answer {
            Ok(value) if !value.last => following.after = Some(value.position),
            // The last value, or a failed read, which ends the stream.
            _ => following.ended = true,
        }
        Poll::Ready(Some(answer))
    }
}
