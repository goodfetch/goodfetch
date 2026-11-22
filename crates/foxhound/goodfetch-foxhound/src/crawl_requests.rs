//! The engine's Crawl requests to one connection (ADRs 0093 and 0112):
//! the connection's one stream of `crawl_request.handle`, opened before
//! its first crawl's start, which the serve of each of its crawl Runs reads
//! its own Run's requests from. The serve that reads the stream keeps
//! another Run's requests aside for that Run's serve; none of them spawns
//! a task.

use std::collections::{HashMap, VecDeque};
use std::mem;
use std::pin::pin;
use std::sync::PoisonError;

use goodfetch_collie_client::{ClientConnection, Error, HandlerCall, HandlerRequests};
use goodfetch_protocol::foxhound::crawl::CRAWL_REQUEST_HANDLE;
use goodfetch_protocol::run::RunId;
use serde_json::Value;
use tokio::sync::Notify;

use crate::sync::{Mutex, MutexGuard};

/// The Crawl requests of one connection.
#[derive(Debug, Default)]
pub(crate) struct CrawlRequests {
    state: Mutex<State>,
    /// Told when the stream is given back or ends, and when a request is
    /// kept aside for a Run.
    changed: Notify,
}

#[derive(Debug, Default)]
struct State {
    stream: Stream,
    /// The requests read for a Run whose serve was not reading, by Run ID.
    kept: HashMap<String, VecDeque<HandlerCall>>,
}

/// Where the connection's stream of Crawl requests is.
#[derive(Debug, Default)]
enum Stream {
    /// Not opened yet: no crawl was started on the connection.
    #[default]
    Unopened,
    /// Waiting for a serve to read it.
    Idle(HandlerRequests),
    /// A serve is reading it.
    Reading,
    /// The connection ended.
    Ended,
}

impl CrawlRequests {
    /// Opens the stream once, before the connection's first crawl starts,
    /// so none of its requests is answered with the method error.
    ///
    /// # Errors
    ///
    /// `closed` once the connection has ended.
    pub(crate) fn open(&self, connection: &ClientConnection) -> Result<(), Error> {
        let mut state = self.state();
        if matches!(state.stream, Stream::Unopened) {
            state.stream = Stream::Idle(connection.handler_requests(CRAWL_REQUEST_HANDLE.name)?);
        }
        Ok(())
    }

    /// The next Crawl request of the Run `run_id`, or `None` once the
    /// connection has ended and none is kept for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no request: the stream it read is given
    /// back, and the requests it read are kept for their Runs.
    pub(crate) async fn next(&self, run_id: &RunId) -> Option<HandlerCall> {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            let taken = {
                let mut state = self.state();
                if let Some(call) = state
                    .kept
                    .get_mut(run_id.as_str())
                    .and_then(VecDeque::pop_front)
                {
                    return Some(call);
                }
                match mem::replace(&mut state.stream, Stream::Reading) {
                    Stream::Idle(requests) => Some(requests),
                    Stream::Reading => None,
                    ended @ (Stream::Unopened | Stream::Ended) => {
                        state.stream = ended;
                        return None;
                    }
                }
            };
            match taken {
                Some(requests) => {
                    let reader = Reader {
                        requests: Some(requests),
                        owner: self,
                    };
                    if let Some(call) = reader.read_until(run_id).await {
                        return Some(call);
                    }
                }
                None => changed.await,
            }
        }
    }

    /// Drops the requests kept for the Run `run_id`, whose serve ended:
    /// each goes back to the engine as dropped by the program.
    pub(crate) fn forget(&self, run_id: &RunId) {
        let kept = self.state().kept.remove(run_id.as_str());
        drop(kept);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A serve's hold on the stream, which it gives back when dropped.
struct Reader<'a> {
    requests: Option<HandlerRequests>,
    owner: &'a CrawlRequests,
}

impl Reader<'_> {
    /// Reads until a request of `run_id`, keeping the others aside for
    /// their Runs: `None` once the stream ended, or when this future gave
    /// the stream back to another serve's kept request.
    async fn read_until(mut self, run_id: &RunId) -> Option<HandlerCall> {
        loop {
            let requests = self.requests.as_mut()?;
            let Some(call) = requests.next().await else {
                self.requests = None;
                self.owner.state().stream = Stream::Ended;
                return None;
            };
            match run_of(&call) {
                Some(id) if id == run_id.as_str() => return Some(call),
                Some(id) => {
                    let id = id.to_owned();
                    self.owner
                        .state()
                        .kept
                        .entry(id)
                        .or_default()
                        .push_back(call);
                    self.owner.changed.notify_waiters();
                }
                // A request with no Run ID is no Crawl request of a Run:
                // dropping it gives it back to the engine.
                None => drop(call),
            }
        }
    }
}

impl Drop for Reader<'_> {
    fn drop(&mut self) {
        let mut state = self.owner.state();
        if let Some(requests) = self.requests.take()
            && matches!(state.stream, Stream::Reading)
        {
            state.stream = Stream::Idle(requests);
        }
        drop(state);
        self.owner.changed.notify_waiters();
    }
}

/// The Run ID a Crawl request names.
fn run_of(call: &HandlerCall) -> Option<&str> {
    call.params().get("run_id").and_then(Value::as_str)
}
