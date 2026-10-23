//! One page's pipeline: each request its target pauses goes through the
//! steps the interception lists, in order, as a task of the page's Run.
//! Steps 3 to 8 for what the last route sends on are [`steps`]'s.
#![expect(
    clippy::disallowed_types,
    reason = "routes are armed and answers delivered as their call arrives, before any `.await`: each lock guards one table or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use data_encoding::BASE64;
use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{fetch, network};
use goodfetch_engine::CallError;
use goodfetch_engine::client::{ClientRequestError, Sent};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::route::{
    AbortReason, PAGE_HANDLER_FAILED, PageHandler, PageHandlerFailedDetails, ROUTE_HANDLE,
    RouteFetched, RouteHandleParams, RouteHandled, RouteId, RouteRequest,
};
use goodfetch_protocol::page::{PageId, RequestFailed};
use tokio::sync::oneshot;

use super::answers::{Answer, Answers, Fulfillment, Reply, already_handled};
use super::routes::Route;
use super::steps::{self, Overrides, entries};
use super::{PageState, lock};
use crate::driver::PageDriver;

/// What the pipeline of one page holds.
pub(super) struct PageContext {
    driver: PageDriver,
    page_id: PageId,
    state: Arc<PageState>,
    answers: Answers,
    policy: Policy,
    log: RequestLog,
    run: Run,
    /// The requests a `fetch` sent on, waiting for their response to be
    /// paused, by the interception's ID.
    responses: Mutex<BTreeMap<fetch::RequestId, oneshot::Sender<fetch::RequestPausedEvent>>>,
}

impl PageContext {
    pub(super) fn new(
        driver: PageDriver,
        page_id: PageId,
        state: Arc<PageState>,
        answers: Answers,
        policy: Policy,
        log: RequestLog,
        run: Run,
    ) -> Self {
        Self {
            driver,
            page_id,
            state,
            answers,
            policy,
            log,
            run,
            responses: Mutex::new(BTreeMap::new()),
        }
    }
}

/// Reads the page's paused requests until its target goes, each through the
/// pipeline as a task of its own.
pub(super) async fn dispatch(mut events: Subscription, context: Arc<PageContext>) {
    while let Some(event) = events.next().await {
        let Some(paused) = event.read::<fetch::RequestPausedEvent>() else {
            continue;
        };
        let at_response =
            paused.response_status_code.is_some() || paused.response_error_reason.is_some();
        if at_response && let Some(waiting) = lock(&context.responses).remove(&paused.request_id) {
            let _ = waiting.send(paused);
            continue;
        }
        let context = Arc::clone(&context);
        let run = context.run.clone();
        run.spawn(async move {
            if at_response {
                // A response no `fetch` waits for goes on as it came.
                context.continue_response(&paused.request_id).await;
            } else {
                context.intercept(paused).await;
            }
        });
    }
}

/// A paused request as it goes: as the page sent it, with the overrides of
/// the routes it passed.
struct Held {
    paused: fetch::RequestPausedEvent,
    overrides: Overrides,
    /// Whether it loads the document of the page's main frame, so the
    /// navigation waits on it.
    navigation: bool,
}

impl Held {
    fn url(&self) -> &str {
        self.overrides
            .url
            .as_deref()
            .unwrap_or(&self.paused.request.url)
    }

    fn method(&self) -> &str {
        self.overrides
            .method
            .as_deref()
            .unwrap_or(&self.paused.request.method)
    }

    /// Takes `overrides` over its own, part by part.
    fn apply(&mut self, overrides: Overrides) {
        self.overrides = std::mem::take(&mut self.overrides).then(overrides);
    }

    /// The request as a route sees it.
    /// The request as a route sees it: without the Session's cookies.
    fn view(&self) -> RouteRequest {
        let headers = self.overrides.headers.clone().unwrap_or_else(|| {
            self.paused
                .request
                .headers
                .iter()
                .filter(|(name, _)| !name.eq_ignore_ascii_case(COOKIE))
                .map(|(name, value)| (name.clone(), header_value(value)))
                .collect()
        });
        RouteRequest {
            url: self.url().to_owned(),
            method: self.method().to_owned(),
            headers,
            resource_type: resource_type(self.paused.resource_type),
            is_navigation_request: self.navigation,
        }
    }
}

/// The header of the Session's cookies.
const COOKIE: &str = "cookie";

/// A header's value as the browser gives it.
fn header_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// The name the browser gives `resource_type`, such as `Document`.
fn resource_type(resource_type: network::ResourceType) -> String {
    serde_json::to_value(resource_type)
        .ok()
        .and_then(|name| name.as_str().map(str::to_owned))
        .unwrap_or_else(|| "Other".to_owned())
}

/// What a route did with a request.
enum Routed {
    /// It passed it to the route before, with these overrides.
    Fallback(Overrides),
    /// It ended there, answered or sent on.
    Ended,
}

/// Where a request a route answered stands.
enum Stage {
    /// Waiting for its first answer.
    Unanswered,
    /// Sent on by `fetch`, its response paused at this interception ID.
    Fetched,
}

impl PageContext {
    /// Runs one paused request through the pipeline.
    async fn intercept(&self, paused: fetch::RequestPausedEvent) {
        let navigation = paused.resource_type == network::ResourceType::Document
            && &paused.frame_id == self.driver.main_frame();
        let mut held = Held {
            paused,
            overrides: Overrides::default(),
            navigation,
        };
        // Step 1: a crawl request's own document fetch has its Hooks
        // applied already (ADR 0104); nothing here runs a Hook.
        // Step 2: page routes, the latest first, then Session routes.
        let routes = self
            .state
            .routes
            .latest_first()
            .into_iter()
            .chain(self.state.session.latest_first());
        for route in routes {
            if !route.matches(held.url()) {
                continue;
            }
            match self.ask(&route, &mut held).await {
                Routed::Fallback(overrides) => held.apply(overrides),
                Routed::Ended => return,
            }
        }
        let _ = self.send_on(&held, false).await;
    }

    /// Asks `route`'s handler about `held`, and acts on its first answer.
    async fn ask(&self, route: &Arc<Route>, held: &mut Held) -> Routed {
        let (route_id, mut slot) = self.answers.open();
        let params = RouteHandleParams {
            id: route.call.clone(),
            route_id: route_id.clone(),
            page_id: self.page_id.clone(),
            request: held.view(),
        };
        let Ok(mut sent) = route
            .client
            .send::<RouteHandled>(&ROUTE_HANDLE, &params)
            .await
        else {
            // Its Connection ended: no handler answers it any more.
            self.remove(route);
            return Routed::Fallback(Overrides::default());
        };
        let mut stage = Stage::Unanswered;
        loop {
            tokio::select! {
                biased;
                Some((answer, reply)) = slot.next() => {
                    let (replied, routed) = self.act(held, &route_id, &mut stage, answer).await;
                    let _ = reply.send(replied);
                    if let Some(routed) = routed {
                        drop(slot);
                        self.watch(route, held, sent);
                        return routed;
                    }
                }
                ended = sent.answer() => {
                    drop(slot);
                    return self.ended(route, held, &stage, ended).await;
                }
            }
        }
    }

    /// Acts on `answer` to `held` at `stage`: the reply its call gets, and
    /// what the route did when the answer settles the request.
    async fn act(
        &self,
        held: &mut Held,
        route_id: &RouteId,
        stage: &mut Stage,
        answer: Answer,
    ) -> (Result<Reply, CallError>, Option<Routed>) {
        let id = held.paused.request_id.clone();
        match (answer, &*stage) {
            (Answer::Fallback(overrides), Stage::Unanswered) => {
                (Ok(Reply::Answered), Some(Routed::Fallback(overrides)))
            }
            (Answer::Continue(overrides), Stage::Unanswered) => {
                held.apply(overrides);
                let _ = self.send_on(held, false).await;
                (Ok(Reply::Answered), Some(Routed::Ended))
            }
            (Answer::Fetch(overrides), Stage::Unanswered) => {
                held.apply(overrides);
                let (response, responded) = oneshot::channel();
                lock(&self.responses).insert(id.clone(), response);
                if let Err(refused) = self.send_on(held, true).await {
                    lock(&self.responses).remove(&id);
                    return (Err(refused), Some(Routed::Ended));
                }
                match responded.await {
                    Ok(paused) => {
                        *stage = Stage::Fetched;
                        (self.fetched(&paused).await, None)
                    }
                    Err(_) => (Err(gone(held.url())), Some(Routed::Ended)),
                }
            }
            (Answer::Fulfill(fulfillment), _) => {
                self.fulfill(&id, fulfillment).await;
                (Ok(Reply::Answered), Some(Routed::Ended))
            }
            (Answer::Abort(reason), _) => {
                self.fail(&id, abort_reason(reason)).await;
                (Ok(Reply::Answered), Some(Routed::Ended))
            }
            // After `fetch`, sending on passes the response it got.
            (Answer::Continue(_) | Answer::Fallback(_), Stage::Fetched) => {
                self.continue_response(&id).await;
                (Ok(Reply::Answered), Some(Routed::Ended))
            }
            (Answer::Fetch(_), Stage::Fetched) => (Err(already_handled(route_id)), None),
        }
    }

    /// The response a `fetch` got, its body read.
    async fn fetched(&self, paused: &fetch::RequestPausedEvent) -> Result<Reply, CallError> {
        let body = self
            .driver
            .command(&fetch::GetResponseBodyParams {
                request_id: paused.request_id.clone(),
            })
            .await
            .map(|read| {
                if read.base64_encoded {
                    read.body
                } else {
                    BASE64.encode(read.body.as_bytes())
                }
            })
            .unwrap_or_default();
        let headers = paused
            .response_headers
            .iter()
            .flatten()
            .map(|header| (header.name.to_ascii_lowercase(), header.value.clone()))
            .collect();
        Ok(Reply::Fetched(RouteFetched {
            status: paused
                .response_status_code
                .and_then(|status| u16::try_from(status).ok())
                .unwrap_or_default(),
            status_text: paused.response_status_text.clone().unwrap_or_default(),
            headers,
            body,
        }))
    }

    /// The handler of `route` replied `ended` before it answered `held`.
    async fn ended(
        &self,
        route: &Arc<Route>,
        held: &Held,
        stage: &Stage,
        ended: Result<RouteHandled, ClientRequestError>,
    ) -> Routed {
        let id = &held.paused.request_id;
        match (ended, stage) {
            (Err(ClientRequestError::Failed { message, .. }), _) => {
                // Its failure waits before the request fails, so the call
                // waiting on it raises it; never sent half-changed.
                self.handler_failed(route, held, &message);
                self.fail(id, network::ErrorReason::Failed).await;
                Routed::Ended
            }
            (Err(ClientRequestError::Gone), Stage::Unanswered) => {
                self.remove(route);
                Routed::Fallback(Overrides::default())
            }
            // Ended without an answer: it falls back.
            (_, Stage::Unanswered) => Routed::Fallback(Overrides::default()),
            // Ended after `fetch`: the page gets the response it got.
            (_, Stage::Fetched) => {
                self.continue_response(id).await;
                Routed::Ended
            }
        }
    }

    /// Watches the handler that answered `held` until it ends: one that
    /// fails then still loses its route and leaves its failure.
    fn watch(&self, route: &Arc<Route>, held: &Held, mut sent: Sent<RouteHandled>) {
        let (route, state, url) = (Arc::clone(route), Arc::clone(&self.state), held.view().url);
        self.run.spawn(async move {
            if let Err(ClientRequestError::Failed { message, .. }) = sent.answer().await {
                route.fail(handler_failed(&url, &message));
                state.routes.remove(&route);
                state.session.remove(&route);
                state.leave(handler_failed(&url, &message));
            }
        });
    }

    /// `route`'s handler failed on `held`: its route goes, its subscription
    /// ends, and its failure waits for the page's next call.
    fn handler_failed(&self, route: &Arc<Route>, held: &Held, message: &str) {
        let url = held.view().url;
        route.fail(handler_failed(&url, message));
        self.remove(route);
        self.state.leave(handler_failed(&url, message));
    }

    /// Takes `route` out of the page's and its Session's tables.
    fn remove(&self, route: &Arc<Route>) {
        self.state.routes.remove(route);
        self.state.session.remove(route);
    }

    /// Steps 3 to 8 for what the last route sends on; `fetch` pauses its
    /// response.
    ///
    /// # Errors
    ///
    /// `egress_refused` when the domain policy refuses it: the request is
    /// failed, and the navigation waiting on it raises the refusal, or else
    /// the Run trace records it.
    async fn send_on(&self, held: &Held, fetch: bool) -> Result<(), CallError> {
        let id = &held.paused.request_id;
        let command = match steps::send_on(id, held.url(), &held.overrides, &self.policy, fetch) {
            Ok(command) => command,
            Err(refused) => {
                if held.navigation {
                    // It waits before the navigation fails, so the
                    // navigation raises it.
                    self.state.leave(refused.error());
                    self.fail(id, network::ErrorReason::BlockedByClient).await;
                } else {
                    self.fail(id, network::ErrorReason::BlockedByClient).await;
                    let failed = RequestFailed {
                        url: held.url().to_owned(),
                        method: held.method().to_owned(),
                        failure: refused.failure(),
                    };
                    if let Err(error) = self.log.request_failed(&failed).await {
                        tracing::warn!(%error, "the Run trace did not take a refused request");
                    }
                }
                return Err(refused.error());
            }
        };
        if let Err(failure) = self.driver.command(&command).await {
            tracing::debug!(?failure, "a paused request went before it was sent on");
        }
        Ok(())
    }

    /// Answers the request `id` with `fulfillment`.
    async fn fulfill(&self, id: &fetch::RequestId, fulfillment: Fulfillment) {
        let fulfilled = self
            .driver
            .command(&fetch::FulfillRequestParams {
                request_id: id.clone(),
                response_code: i64::from(fulfillment.status),
                response_headers: Some(entries(&fulfillment.headers)),
                binary_response_headers: None,
                body: Some(fulfillment.body),
                response_phrase: None,
            })
            .await;
        if let Err(failure) = fulfilled {
            tracing::debug!(?failure, "a paused request went before it was fulfilled");
        }
    }

    /// Fails the request `id` with `reason`.
    async fn fail(&self, id: &fetch::RequestId, reason: network::ErrorReason) {
        let failed = self
            .driver
            .command(&fetch::FailRequestParams {
                request_id: id.clone(),
                error_reason: reason,
            })
            .await;
        if let Err(failure) = failed {
            tracing::debug!(?failure, "a paused request went before it was failed");
        }
    }

    /// Lets the response paused at `id` go on as it came.
    async fn continue_response(&self, id: &fetch::RequestId) {
        let continued = self
            .driver
            .command(&fetch::ContinueResponseParams {
                request_id: id.clone(),
                response_code: None,
                response_phrase: None,
                response_headers: None,
                binary_response_headers: None,
            })
            .await;
        if let Err(failure) = continued {
            tracing::debug!(?failure, "a paused response went before it was sent on");
        }
    }
}

/// The browser's network error for `reason`.
fn abort_reason(reason: AbortReason) -> network::ErrorReason {
    match reason {
        AbortReason::Aborted => network::ErrorReason::Aborted,
        AbortReason::AccessDenied => network::ErrorReason::AccessDenied,
        AbortReason::AddressUnreachable => network::ErrorReason::AddressUnreachable,
        AbortReason::BlockedByClient => network::ErrorReason::BlockedByClient,
        AbortReason::BlockedByResponse => network::ErrorReason::BlockedByResponse,
        AbortReason::ConnectionAborted => network::ErrorReason::ConnectionAborted,
        AbortReason::ConnectionClosed => network::ErrorReason::ConnectionClosed,
        AbortReason::ConnectionFailed => network::ErrorReason::ConnectionFailed,
        AbortReason::ConnectionRefused => network::ErrorReason::ConnectionRefused,
        AbortReason::ConnectionReset => network::ErrorReason::ConnectionReset,
        AbortReason::InternetDisconnected => network::ErrorReason::InternetDisconnected,
        AbortReason::NameNotResolved => network::ErrorReason::NameNotResolved,
        AbortReason::TimedOut => network::ErrorReason::TimedOut,
        AbortReason::Failed => network::ErrorReason::Failed,
    }
}

/// `closed` for a fetched request whose response never came: its page
/// went first.
fn gone(url: &str) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the page closed before the response to {url} came"),
        &serde_json::json!({ "url": url }),
    )
}

/// `page_handler_failed` for a route's handler that failed on a request to
/// `url`, saying `message`.
fn handler_failed(url: &str, message: &str) -> CallError {
    CallError::new(
        &PAGE_HANDLER_FAILED,
        format!("a route's handler failed on the request to {url}: {message}"),
        &PageHandlerFailedDetails {
            handler: PageHandler::Route,
            url: url.to_owned(),
            failure: message.to_owned(),
        },
    )
}
