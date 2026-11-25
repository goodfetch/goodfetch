//! A page's or a Session's routes, in the order they were armed: each a URL
//! pattern, the Client whose handler answers its requests and the call whose
//! subscription it is. A route leaves its table when its subscription ends,
//! and its subscription ends when its handler fails, its table closes, or
//! its page's request part, which a page's routes belong to, is replaced
//! (ADR 0146).
#![expect(
    clippy::disallowed_types,
    reason = "routes are armed and answers delivered as their call arrives, before any `.await`: each lock guards one table or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::sync::{Arc, Mutex};

use goodfetch_engine::CallError;
use goodfetch_engine::client::Client;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::route::RouteEnded;
use tokio::sync::{oneshot, watch};

use super::lock;
use crate::url_pattern::UrlPattern;

/// One route.
pub(super) struct Route {
    pattern: UrlPattern,
    /// The Client whose handler answers its requests.
    pub(super) client: Client,
    /// The id of the call whose subscription it is.
    pub(super) call: RequestId,
    /// Where its handler's failure goes, once: its subscription's end.
    failed: Mutex<Failed>,
    ended: Mutex<Option<oneshot::Receiver<CallError>>>,
}

/// Where a route's handler's failure goes.
enum Failed {
    /// To its subscription, which it ends.
    Live(oneshot::Sender<CallError>),
    /// Its subscription ended with an earlier failure.
    Ended,
    /// Its page's request part, which armed it, was replaced (ADR 0146):
    /// its subscription ended, and its failures belong to no call.
    Retired,
}

impl Route {
    pub(super) fn new(pattern: UrlPattern, client: Client, call: RequestId) -> Self {
        let (failed, ended) = oneshot::channel();
        Self {
            pattern,
            client,
            call,
            failed: Mutex::new(Failed::Live(failed)),
            ended: Mutex::new(Some(ended)),
        }
    }

    /// Whether it answers a request to `url`.
    pub(super) fn matches(&self, url: &str) -> bool {
        self.pattern.matches(url)
    }

    /// Ends its subscription with `error`, its handler's failure, unless
    /// it ended already: whether the failure is left for its page's next
    /// call, which it is not once its page's request part was replaced.
    pub(super) fn fail(&self, error: CallError) -> bool {
        let mut failed = lock(&self.failed);
        match std::mem::replace(&mut *failed, Failed::Ended) {
            Failed::Live(subscription) => {
                let _ = subscription.send(error);
                true
            }
            Failed::Ended => true,
            Failed::Retired => {
                *failed = Failed::Retired;
                false
            }
        }
    }

    /// Ends its subscription without a failure, as its page's request part
    /// is replaced.
    fn retire(&self) {
        let retired = std::mem::replace(&mut *lock(&self.failed), Failed::Retired);
        drop(retired);
    }
}

/// The routes of one page or Session.
pub(super) struct RouteTable {
    routes: Mutex<Vec<Arc<Route>>>,
    closed: watch::Sender<bool>,
}

impl Default for RouteTable {
    fn default() -> Self {
        Self {
            routes: Mutex::new(Vec::new()),
            closed: watch::Sender::new(false),
        }
    }
}

impl RouteTable {
    /// Adds `route` as the latest.
    pub(super) fn arm(self: &Arc<Self>, route: Route) -> Armed {
        let route = Arc::new(route);
        lock(&self.routes).push(Arc::clone(&route));
        let ended = lock(&route.ended)
            .take()
            .expect("invariant: a route is armed once");
        Armed {
            table: Arc::clone(self),
            route,
            ended,
            closed: self.closed.subscribe(),
        }
    }

    /// Its routes, the latest first.
    pub(super) fn latest_first(&self) -> Vec<Arc<Route>> {
        lock(&self.routes).iter().rev().cloned().collect()
    }

    /// Whether any of its routes answers a request to `url`.
    pub(super) fn matches_any(&self, url: &str) -> bool {
        lock(&self.routes).iter().any(|route| route.matches(url))
    }

    /// Takes `route` out, so no later request reaches it.
    pub(super) fn remove(&self, route: &Arc<Route>) {
        lock(&self.routes).retain(|armed| !Arc::ptr_eq(armed, route));
    }

    /// Closes it, as its page or Session closes: every route's subscription
    /// ends.
    pub(super) fn close(&self) {
        lock(&self.routes).clear();
        self.closed.send_replace(true);
    }

    /// Ends the routes its page's earlier request armed, as its request
    /// part is replaced (ADR 0146): each subscription ends, and the next
    /// request arms its own.
    pub(super) fn replace_request(&self) {
        let earlier = std::mem::take(&mut *lock(&self.routes));
        for route in earlier {
            route.retire();
        }
    }

    /// Whether it closed.
    pub(super) fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }
}

/// An armed route, as its subscription holds it: dropped, it takes the
/// route out of its table.
pub(crate) struct Armed {
    table: Arc<RouteTable>,
    route: Arc<Route>,
    ended: oneshot::Receiver<CallError>,
    closed: watch::Receiver<bool>,
}

impl Armed {
    /// Waits for the subscription's end: its handler's failure, its page
    /// or Session closing, or its page's request part being replaced.
    ///
    /// # Errors
    ///
    /// The handler's failure, `page_handler_failed`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future, as a `$/cancel` does, removes the route.
    pub(crate) async fn ended(mut self) -> Result<RouteEnded, CallError> {
        tokio::select! {
            failed = &mut self.ended => match failed {
                Ok(error) => Err(error),
                Err(_) => Ok(RouteEnded {}),
            },
            _ = self.closed.wait_for(|closed| *closed) => Ok(RouteEnded {}),
        }
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        self.table.remove(&self.route);
    }
}
