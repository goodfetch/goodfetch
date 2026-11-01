//! The requests route handlers are answering, each by its `route_id`
//! (`route-<n>`, numbered from 1 by the engine): an answer is delivered to
//! its request's slot when its call arrives, and answered once the pipeline
//! acted on it. A request's slot closes once it is answered or its handler
//! ended, so any later answer raises `route_already_handled`; a slot holds
//! one answer at a time (STYLE.md: every channel is bounded), so one that
//! comes while the one before is still unread raises it too.
#![expect(
    clippy::disallowed_types,
    reason = "routes are armed and answers delivered as their call arrives, before any `.await`: each lock guards one table or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use goodfetch_engine::CallError;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::route::{
    AbortReason, ROUTE_ALREADY_HANDLED, RouteAlreadyHandledDetails, RouteFetched, RouteId,
};
use tokio::sync::{mpsc, oneshot, watch};

use super::lock;
pub(crate) use super::steps::Overrides;

/// A response of the program's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fulfillment {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    /// In Base64.
    pub(crate) body: String,
}

/// An answer to a route's request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Send it on past every other route.
    Continue(Overrides),
    /// Pass it to the route armed before.
    Fallback(Overrides),
    /// Answer it with this response.
    Fulfill(Fulfillment),
    /// Fail it with this network error.
    Abort(AbortReason),
    /// Send it on past every other route and answer its response.
    Fetch(Overrides),
}

/// What the pipeline answers an answer with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// It acted on it.
    Answered,
    /// The response `fetch` got.
    Fetched(RouteFetched),
}

/// An answer on its way to its request, with where its reply goes.
pub(super) type Delivered = (Answer, oneshot::Sender<Result<Reply, CallError>>);

/// Every open slot, by its number. Clones share them; a clone made for a
/// page also counts how many of its requests have a slot open, which is
/// while a route handler holds them.
#[derive(Clone, Default)]
pub(super) struct Answers {
    shared: Arc<Shared>,
    waiting: Option<Arc<watch::Sender<usize>>>,
}

#[derive(Default)]
struct Shared {
    /// The last number given.
    given: AtomicU64,
    slots: Mutex<BTreeMap<u64, mpsc::Sender<Delivered>>>,
}

/// One request's slot, which the pipeline reads its answers from; dropped,
/// it closes.
pub(super) struct Slot {
    number: u64,
    answers: mpsc::Receiver<Delivered>,
    shared: Arc<Shared>,
    waiting: Option<Arc<watch::Sender<usize>>>,
}

impl Answers {
    /// The same slots, counting the requests that have one open in
    /// `waiting`.
    pub(super) fn counting(&self, waiting: Arc<watch::Sender<usize>>) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            waiting: Some(waiting),
        }
    }

    /// Opens the slot of a new request, and names it.
    pub(super) fn open(&self) -> (RouteId, Slot) {
        let number = self.shared.given.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, answers) = mpsc::channel(1);
        lock(&self.shared.slots).insert(number, sender);
        if let Some(waiting) = &self.waiting {
            waiting.send_modify(|waiting| *waiting += 1);
        }
        (
            RouteId(format!("route-{number}")),
            Slot {
                number,
                answers,
                shared: Arc::clone(&self.shared),
                waiting: self.waiting.clone(),
            },
        )
    }

    /// Delivers `answer` to the request `route_id` names, and answers its
    /// reply.
    pub(super) fn deliver(
        &self,
        route_id: &RouteId,
        answer: Answer,
    ) -> Result<impl Future<Output = Result<Reply, CallError>> + use<>, CallError> {
        let number = route_id
            .0
            .strip_prefix("route-")
            .and_then(|number| number.parse::<u64>().ok())
            .filter(|number| (1..=self.shared.given.load(Ordering::Relaxed)).contains(number))
            .ok_or_else(|| {
                CallError::new(
                    &CLOSED,
                    format!("no route's request has the ID `{}`", route_id.0),
                    &RouteAlreadyHandledDetails {
                        route_id: route_id.clone(),
                    },
                )
            })?;
        let (reply, replied) = oneshot::channel();
        let delivered = lock(&self.shared.slots)
            .get(&number)
            .is_some_and(|slot| slot.try_send((answer, reply)).is_ok());
        if !delivered {
            return Err(already_handled(route_id));
        }
        let route_id = route_id.clone();
        Ok(async move {
            replied
                .await
                .unwrap_or_else(|_| Err(already_handled(&route_id)))
        })
    }
}

impl Slot {
    /// The next answer delivered; `None` never comes while the slot is
    /// open, so a caller reads it beside something else.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no answer.
    pub(super) async fn next(&mut self) -> Option<Delivered> {
        self.answers.recv().await
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        lock(&self.shared.slots).remove(&self.number);
        if let Some(waiting) = &self.waiting {
            waiting.send_modify(|waiting| *waiting = waiting.saturating_sub(1));
        }
    }
}

/// `route_already_handled` for `route_id`.
pub(super) fn already_handled(route_id: &RouteId) -> CallError {
    CallError::new(
        &ROUTE_ALREADY_HANDLED,
        format!(
            "the route's request `{}` was answered already, or its handler ended",
            route_id.0
        ),
        &RouteAlreadyHandledDetails {
            route_id: route_id.clone(),
        },
    )
}
