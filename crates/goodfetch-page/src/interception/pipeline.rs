//! One page's pipeline: each request its target pauses goes through the
//! steps the interception lists, in order, as a task of the page's Run.
//! Steps 3 to 8 for what the last route sends on are [`steps`]'s; step 6,
//! the page's navigation gate, waits, so it is asked here. A document's
//! response pauses too, for the gate to judge.
//!
//! Size: one paused request's way through the steps, each of which reads
//! and answers the same held request on the same page; the steps' pure
//! judgement, the routes, the answers and the pending navigations are in
//! their own modules.
#![expect(
    clippy::disallowed_types,
    reason = "routes are armed and answers delivered as their call arrives, before any `.await`: each lock guards one table or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use data_encoding::BASE64;
use goodfetch_cdp::generated::{fetch, network, page};
use goodfetch_cdp::{NetError, Subscription};
use goodfetch_counted_work::Row;
use goodfetch_engine::CallError;
use goodfetch_engine::client::{ClientRequestError, Sent};
use goodfetch_engine::egress::{self, Gate, Policy, Refused, RequestKind};
use goodfetch_engine::http_client;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::runs::{Run, run_trace};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::page::events::RequestFailure;
use goodfetch_protocol::page::route::{
    AbortReason, PAGE_HANDLER_FAILED, PageHandler, PageHandlerFailedDetails, ROUTE_HANDLE,
    RouteFetched, RouteHandleParams, RouteHandled, RouteId, RouteRequest,
};
use goodfetch_protocol::page::{
    NAVIGATION_FAILED, NavigationFailedDetails, PageId, RequestFailed, ServiceWorkerRequest,
    SessionId,
};
use goodfetch_protocol::secrets::{
    SECRET_INPUT_REFUSED, SecretInputRefusedDetails, SecretInputRefusedReason,
};
use tokio::sync::oneshot;

use super::answers::{Answer, Answers, Fulfillment, Reply, already_handled};
use super::authentication;
use super::hops;
use super::lookups::Lookups;
use super::routes::Route;
use super::steps::{self, Blocking, Overrides, Stopped, entries};
use super::{PageState, lock};
use crate::driver::{PageDriver, cdp_response};
use crate::errors::{gate_failure, gate_refused};
use crate::page_events::{Refusals, is_navigation, post_data, resource_type};
use crate::page_value::{
    DocumentResponse, Navigation, NavigationKind, Permit, ReadDocument, Reader, Reading, Refusal,
    SiteResponse,
};
use crate::paused_body;
use crate::proxy::{self, PROXY_STATUS, ProxyPermit};

/// Whose requests a pipeline holds.
pub(crate) enum Sender {
    /// A page's.
    Page(PageId),
    /// An out-of-process frame's of the page `page`, whose target is not
    /// the page's: they pass the page's routes, and its page's events never
    /// see them, so each one refused is recorded in the Run trace at once.
    Frame(PageId),
    /// A site's service worker's, which belongs to no page, of the Session
    /// `session`: each one is recorded in the Run trace's request `log`
    /// (ADR 0121).
    ServiceWorker {
        session: SessionId,
        log: Arc<RequestLog>,
    },
    /// A shared worker's, which belongs to no page: each one refused is
    /// recorded in the Run trace at once.
    SharedWorker,
}

impl Sender {
    /// The page, when a page or one of its frames sends the requests.
    fn page_id(&self) -> Option<&PageId> {
        match self {
            Self::Page(page_id) | Self::Frame(page_id) => Some(page_id),
            Self::ServiceWorker { .. } | Self::SharedWorker => None,
        }
    }

    /// Whether a refusal is recorded in the Run trace at once, since no
    /// page's events report the sender's requests: a frame's or a shared
    /// worker's.
    fn records_at_once(&self) -> bool {
        matches!(self, Self::Frame(_) | Self::SharedWorker)
    }
}

/// What the pipeline of one page, or of one service worker, holds.
pub(super) struct PageContext {
    driver: PageDriver,
    sender: Sender,
    state: Arc<PageState>,
    answers: Answers,
    policy: Policy,
    blocking: Blocking,
    /// The refusals the page's requests then carry.
    refusals: Refusals,
    run: Run,
    /// The requests a `fetch` sent on, waiting for their response to be
    /// paused, by the interception's ID, each with its network ID: a
    /// request that fails first, or a target that goes, drops its waiter.
    responses: Mutex<BTreeMap<fetch::RequestId, Fetching>>,
    /// A service worker's requests that carry a page's navigation, sent on
    /// with their response paused for the gate to judge, by the
    /// interception's ID: the page each carries a navigation of.
    carried: Mutex<BTreeMap<fetch::RequestId, Weak<PageState>>>,
    /// The URL of the main frame's document last sent on: the source the
    /// Filter lists judge a request without a `Referer` by.
    document: Mutex<Arc<str>>,
    /// The requests the Session's proxy credentials were given for.
    authenticated: Mutex<Authenticated>,
    /// The permit the page's navigation gate gave the main frame's
    /// navigation for its latest hop, and that navigation's network ID,
    /// held until the next hop, the navigation's failure or the page's
    /// `load` event.
    permit: Mutex<Option<(Option<network::RequestId>, Permit)>>,
    /// The Permit the Session's proxy's limits gave the main frame's
    /// navigation for its latest hop, a program's own included, and that
    /// navigation's network ID, held as `permit` is (ADRs 0053 and 0088).
    proxy_permit: Mutex<Option<(Option<network::RequestId>, ProxyPermit)>>,
    /// On a Borrowed browser, or a launched one with no relay, its lookups,
    /// through which the engine resolves each request's host itself, the
    /// Run's `network.resolve` mapping first, and judges its answers, since
    /// no relay is under that browser (ADR 0056; #2435).
    lookups: Option<Arc<Lookups>>,
    /// The Run trace's request log, where a refusal no page's event
    /// reports is recorded.
    log: Arc<RequestLog>,
}

/// A request a `fetch` sent on: its network ID, and where its paused
/// response goes.
struct Fetching {
    network_id: Option<network::RequestId>,
    response: oneshot::Sender<fetch::RequestPausedEvent>,
}

/// The latest requests the Session's proxy credentials were given for, so a
/// second 407 for one of them is a refusal (STYLE.md: every buffer is
/// bounded).
#[derive(Default)]
struct Authenticated {
    requests: BTreeSet<fetch::RequestId>,
    order: VecDeque<fetch::RequestId>,
}

/// The most requests [`Authenticated`] keeps.
const AUTHENTICATED_LIMIT: usize = 1024;

impl Authenticated {
    /// Records `request`; whether it was recorded already.
    fn seen(&mut self, request: &fetch::RequestId) -> bool {
        if self.requests.contains(request) {
            return true;
        }
        if self.order.len() == AUTHENTICATED_LIMIT
            && let Some(oldest) = self.order.pop_front()
        {
            self.requests.remove(&oldest);
        }
        self.requests.insert(request.clone());
        self.order.push_back(request.clone());
        false
    }
}

impl PageContext {
    #[expect(
        clippy::too_many_arguments,
        reason = "one value per part of a page's pipeline, each its own owner's"
    )]
    pub(super) fn new(
        driver: PageDriver,
        sender: Sender,
        state: Arc<PageState>,
        answers: Answers,
        policy: Policy,
        blocking: Blocking,
        refusals: Refusals,
        run: Run,
    ) -> Self {
        Self {
            driver,
            sender,
            state,
            answers,
            policy,
            blocking,
            refusals,
            responses: Mutex::new(BTreeMap::new()),
            carried: Mutex::new(BTreeMap::new()),
            document: Mutex::new(Arc::from("")),
            authenticated: Mutex::new(Authenticated::default()),
            permit: Mutex::new(None),
            proxy_permit: Mutex::new(None),
            lookups: None,
            log: Arc::new(RequestLog::new(run_trace(&run))),
            run,
        }
    }

    /// The same, on an out-of-process frame whose document the browser
    /// named `url` when it attached it: the source the Filter lists judge
    /// its requests without a `Referer` by.
    pub(super) fn with_document(self, url: &str) -> Self {
        Self {
            document: Mutex::new(Arc::from(url)),
            ..self
        }
    }

    /// The same, on a browser no relay is under, whose requests' hosts the
    /// engine resolves through `lookups`, when it is one.
    pub(super) fn with_lookups(self, lookups: Option<Arc<Lookups>>) -> Self {
        Self { lookups, ..self }
    }
}

/// Reads the page's paused requests until its target goes, each through the
/// pipeline as a task of its own.
pub(super) async fn dispatch(mut events: Subscription, context: Arc<PageContext>) {
    while let Some(event) = events.next().await {
        if matches!(context.sender, Sender::Page(_))
            && event.read::<page::LoadEventFiredEvent>().is_some()
        {
            // The navigation's last hop ends with the page's `load`.
            drop(lock(&context.permit).take());
            drop(lock(&context.proxy_permit).take());
            if let Some(page_id) = context.sender.page_id() {
                context.state.pending.ended(page_id);
            }
            continue;
        }
        if let Some(started) = event.read::<page::FrameStartedNavigatingEvent>() {
            if let Some(page_id) = context.sender.page_id()
                && &started.frame_id == context.driver.main_frame()
                && started.navigation_type
                    != page::FrameStartedNavigatingNavigationType::SameDocument
            {
                context.state.pending.started(
                    page_id,
                    &started.url,
                    &Arc::downgrade(&context.state),
                );
            }
            continue;
        }
        // A navigation the page schedules to a scheme no request may use
        // sends no request the interception pauses: it is refused here.
        let scheduled = event
            .read::<page::FrameScheduledNavigationEvent>()
            .map(|scheduled| (scheduled.frame_id, scheduled.url))
            .or_else(|| {
                event
                    .read::<page::FrameRequestedNavigationEvent>()
                    .map(|requested| (requested.frame_id, requested.url))
            });
        if let Some((frame_id, url)) = scheduled {
            context.scheduled(&frame_id, &url);
            continue;
        }
        if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
            // A request a `fetch` sent on that fails gets no response.
            lock(&context.responses)
                .retain(|_, fetching| fetching.network_id.as_ref() != Some(&failed.request_id));
            let mut permit = lock(&context.permit);
            if permit
                .as_ref()
                .is_some_and(|(network_id, _)| network_id.as_ref() == Some(&failed.request_id))
            {
                drop(permit.take());
            }
            drop(permit);
            context.proxy_hop_failed(&failed);
            context.refused_by_relay(&failed);
            continue;
        }
        if let Some(created) = event.read::<network::WebSocketCreatedEvent>() {
            context.web_socket(&created.url);
            continue;
        }
        if let Some(required) = event.read::<fetch::AuthRequiredEvent>() {
            let context = Arc::clone(&context);
            let run = context.run.clone();
            run.spawn(async move { context.authenticate(required).await });
            continue;
        }
        let Some(paused) = event.read::<fetch::RequestPausedEvent>() else {
            continue;
        };
        let at_response =
            paused.response_status_code.is_some() || paused.response_error_reason.is_some();
        context.received(&paused);
        if at_response && let Some(waiting) = lock(&context.responses).remove(&paused.request_id) {
            let _ = waiting.response.send(paused);
            continue;
        }
        let context = Arc::clone(&context);
        let run = context.run.clone();
        run.spawn(async move {
            if at_response {
                // A response no `fetch` waits for goes on as it came, once
                // the navigation gate judged a document's.
                context.responded(&paused).await;
            } else {
                context.intercept(paused).await;
            }
        });
    }
    // The target went: no `fetch` it sent on gets a response now.
    lock(&context.responses).clear();
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

    /// The request as a route sees it, with Playwright's values (ADR
    /// 0085's #703 note): without the Session's cookies, its body masked
    /// with `masks` as the page's events mask it.
    fn view(&self, masks: &Masks) -> RouteRequest {
        let headers = self.overrides.headers.clone().unwrap_or_else(|| {
            let mut headers = cdp_response::headers(&self.paused.request.headers);
            headers.remove(COOKIE);
            headers
        });
        RouteRequest {
            url: self.url().to_owned(),
            method: self.method().to_owned(),
            headers,
            resource_type: resource_type(self.paused.resource_type),
            is_navigation_request: is_navigation(Some(self.paused.resource_type)),
            post_data: self
                .overrides
                .post_data
                .clone()
                .or_else(|| post_data(&self.paused.request, masks)),
        }
    }
}

/// The header of the Session's cookies.
const COOKIE: &str = "cookie";

/// What `secret_input_refused` names the Session's cookies by: the SDK's
/// entry that reads them.
const SESSION_COOKIES: &str = "session.cookies";

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
    /// Sent on by `fetch`, its response paused at this interception ID,
    /// and what `fetch` took of it.
    Fetched(Taken),
}

/// What a route's `fetch` took of the response it got.
enum Taken {
    /// Nothing: it was refused before its body was read, so the response
    /// still goes on as it came.
    Nothing,
    /// Its body, read whole from the browser's stream: the response goes on
    /// as this, its body after any content coding.
    Body {
        status: u16,
        headers: Vec<fetch::HeaderEntry>,
        body: Vec<u8>,
    },
    /// Part of its body, until a limit refused it: the rest is gone, so the
    /// request fails rather than go on.
    Refused,
}

impl PageContext {
    /// Answers a 407 or 401 the browser reports as [`authentication`]
    /// says, once per request; a proxy's refusal on the page's own document
    /// leaves `proxy_auth_failed` for the navigation (ADR 0053).
    async fn authenticate(&self, required: fetch::AuthRequiredEvent) {
        let navigation = required.resource_type == network::ResourceType::Document
            && &required.frame_id == self.driver.main_frame();
        let challenged = authentication::answer(&self.driver, &required, || {
            lock(&self.authenticated).seen(&required.request_id)
        });
        if navigation && let Some(refused) = challenged.refused {
            self.state.leave(refused);
        }
        let credentials_given = challenged.credentials_given;
        let answered = self
            .driver
            .command(&fetch::ContinueWithAuthParams {
                request_id: required.request_id,
                auth_challenge_response: challenged.response,
            })
            .await;
        match answered {
            Err(failure) => {
                tracing::debug!(%failure, "the request that asked for credentials was gone");
            }
            // The navigation leaves only now, with the credentials: its
            // gate hears that send too (ADR 0086).
            Ok(_) if credentials_given && navigation => {
                self.driver.sent(&required.request.url).await;
            }
            Ok(_) => {}
        }
    }

    /// Hands the document whose response is paused at `paused` to the page
    /// whose main frame receives it, this pipeline's own or the one whose
    /// navigation a service worker's request carried, when a program opened
    /// it (ADR 0086): a response the site answered, whatever started the
    /// navigation and whatever a route then does with it, a redirect's
    /// excepted, whose last hop is the document.
    fn received(&self, paused: &fetch::RequestPausedEvent) {
        if paused.response_status_code.is_none()
            || cdp_response::redirects(cdp_response::paused_status(paused))
        {
            return;
        }
        let state = if self.main_document(paused) {
            Some(Arc::clone(&self.state))
        } else {
            lock(&self.carried)
                .get(&paused.request_id)
                .and_then(Weak::upgrade)
        };
        if let Some(state) = state
            && let Some(on_document) = &state.on_document
        {
            on_document.received(&paused.request.url, &state.engine_requests);
        }
    }

    /// Whether `paused` is a request, or a response, of the document of
    /// the page's main frame: a service worker has none.
    fn main_document(&self, paused: &fetch::RequestPausedEvent) -> bool {
        self.sender.page_id().is_some()
            && paused.resource_type == network::ResourceType::Document
            && &paused.frame_id == self.driver.main_frame()
    }

    /// Runs one paused request through the pipeline.
    async fn intercept(&self, paused: fetch::RequestPausedEvent) {
        let navigation = self.main_document(&paused);
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
            page_id: self.sender.page_id().cloned(),
            request: held.view(self.run.secrets().mask().masks()),
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
                let network_id = held.paused.network_id.clone();
                lock(&self.responses).insert(
                    id.clone(),
                    Fetching {
                        network_id,
                        response,
                    },
                );
                if let Err(refused) = self.send_on(held, true).await {
                    lock(&self.responses).remove(&id);
                    return (Err(refused), Some(Routed::Ended));
                }
                // Its response pauses, or its waiter goes with the request
                // or the target; the Run's end stops the wait too.
                let paused = tokio::select! {
                    paused = responded => paused.ok(),
                    () = self.run.cancellation().cancelled() => None,
                };
                lock(&self.responses).remove(&id);
                match paused {
                    Some(paused) => {
                        // The gate judges a main-frame document before the
                        // route gets it.
                        let status = cdp_response::paused_status(&paused);
                        // A document the gate judges is read whole, as the
                        // gate reads it, and goes on as it came; one whose
                        // gate reads no body is read as the route reads it.
                        let judged = held.navigation
                            && self
                                .driver
                                .value()
                                .gate()
                                .is_some_and(|gate| gate.reads_document_body())
                            && !cdp_response::redirects(status);
                        let read = if judged {
                            Some(self.body(&paused.request_id).await)
                        } else {
                            None
                        };
                        if held.navigation
                            && let Some(refused) = self
                                .refused_document(&paused, status, None, read.as_deref())
                                .await
                        {
                            return (Err(refused), Some(Routed::Ended));
                        }
                        let (reply, taken) = self.fetched(&paused, read).await;
                        *stage = Stage::Fetched(taken);
                        (reply, None)
                    }
                    None => (Err(gone(held.url())), Some(Routed::Ended)),
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
            (Answer::Continue(_) | Answer::Fallback(_), Stage::Fetched(taken)) => {
                self.send_on_fetched(&id, taken).await;
                (Ok(Reply::Answered), Some(Routed::Ended))
            }
            (Answer::Fetch(_), Stage::Fetched(_)) => (Err(already_handled(route_id)), None),
        }
    }

    /// The response a `fetch` got, its body read under the Run's limits
    /// (ticket #644), and what was taken of it: a `Content-Length` past
    /// `network.max_body_size`, with no content coding, is refused before
    /// the body is read; any other body is read from the browser's stream,
    /// judged as it arrives and stopped at the first limit it passes, so
    /// neither the browser nor the engine holds more. `read` is the body
    /// when the navigation gate read it whole already.
    ///
    /// # Errors
    ///
    /// `response_limit_exceeded` naming the limit the body passed.
    async fn fetched(
        &self,
        paused: &fetch::RequestPausedEvent,
        read: Option<Vec<u8>>,
    ) -> (Result<Reply, CallError>, Taken) {
        let headers =
            cdp_response::paused_headers(paused.response_headers.as_deref().unwrap_or_default());
        let limits = Limits::of_run(self.run.settings());
        let url = &paused.request.url;
        if let Err(refused) = paused_body::declared_within_limits(url, &headers, &limits) {
            return (Err(refused), Taken::Nothing);
        }
        let declared = cdp_response::declared_length(&headers);
        let status = cdp_response::paused_status(paused);
        let (read, streamed) = match read {
            Some(read) => {
                let decoded = u64::try_from(read.len()).unwrap_or(u64::MAX);
                if let Err(exceeded) = limits.judge_decoded(url, decoded, declared) {
                    return (Err(exceeded.error()), Taken::Nothing);
                }
                (read, false)
            }
            None => {
                match paused_body::streamed(
                    &self.driver,
                    &paused.request_id,
                    url,
                    declared,
                    &limits,
                )
                .await
                {
                    Ok(read) => (read, true),
                    Err(refused) => return (Err(refused), Taken::Refused),
                }
            }
        };
        let reply = Reply::Fetched(RouteFetched {
            status,
            status_text: paused.response_status_text.clone().unwrap_or_default(),
            headers,
            body: BASE64.encode(&read),
        });
        let taken = if streamed {
            Taken::Body {
                status,
                headers: decoded_headers(paused),
                body: read,
            }
        } else {
            Taken::Nothing
        };
        (Ok(reply), taken)
    }

    /// Sends on the response paused at `id` after a route's `fetch` took
    /// `taken` of it: as it came, as the body read, or failed when a limit
    /// left only part of its body.
    async fn send_on_fetched(&self, id: &fetch::RequestId, taken: &Taken) {
        match taken {
            Taken::Nothing => self.continue_response(id).await,
            Taken::Body {
                status,
                headers,
                body,
            } => {
                self.fulfill_whole(id, *status, headers.clone(), body).await;
            }
            Taken::Refused => self.fail(id, network::ErrorReason::Failed).await,
        }
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
            (_, Stage::Fetched(taken)) => {
                self.send_on_fetched(id, taken).await;
                Routed::Ended
            }
        }
    }

    /// Watches the handler that answered `held` until it ends: one that
    /// fails then still loses its route and leaves its failure.
    fn watch(&self, route: &Arc<Route>, held: &Held, mut sent: Sent<RouteHandled>) {
        let (route, state, url) = (
            Arc::clone(route),
            Arc::clone(&self.state),
            held.url().to_owned(),
        );
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
        let url = held.url().to_owned();
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
    /// failed carrying the refusal, and the navigation waiting on it raises
    /// it, or else the Run trace records it; for a `fetch`, `closed` when
    /// the request went before it was sent on.
    async fn send_on(&self, held: &Held, fetch: bool) -> Result<(), CallError> {
        let id = &held.paused.request_id;
        // Borrowed from the request where it can be, so no request
        // allocates its source.
        let document;
        let source: &str = if held.navigation {
            held.url()
        } else if let Some(referer) = referer(&held.paused.request.headers) {
            referer
        } else {
            document = Arc::clone(&*lock(&self.document));
            &document
        };
        let paused = steps::Paused {
            id,
            url: held.url(),
            method: held.method(),
            overrides: &held.overrides,
            kind: steps::kind(held.paused.resource_type, held.navigation),
            source_url: source,
            egress: self.egress_kind(held),
        };
        // Step 4 without the Filter lists while the page loads once more
        // with blocking off, or for a document on a domain its Run learned
        // blocking off for (ADR 0047).
        let without_lists;
        let blocking = if self.state.lists_off(source) {
            without_lists = self.blocking.without_lists();
            &without_lists
        } else {
            &self.blocking
        };
        let stepped = match steps::send_on(&paused, &self.policy, blocking, fetch) {
            Ok(steps::Sent::Continue(command)) => {
                match self.judged_addresses(held.url(), paused.egress).await {
                    Ok(()) => Ok(steps::Sent::Continue(command)),
                    Err(refused) => Err(Stopped::Refused(refused)),
                }
            }
            other => other,
        };
        // Step 7's guard of the Session's cookies: none goes to a host the
        // egress relay terminates for a certificate exception (ADR 0042).
        // The browser adds them itself as the request leaves, so a request
        // that would carry one is failed instead.
        if matches!(stepped, Ok(steps::Sent::Continue(_)))
            && let Some(withheld) = self.withhold_cookies(held).await
        {
            return Err(withheld);
        }
        // The page whose navigation a service worker's request carries.
        let mut gated: Option<Arc<PageState>> = None;
        let command = match stepped {
            Ok(steps::Sent::Continue(command)) => {
                // Step 6: the page's navigation gate; its lease check is
                // asked before the command that sends the request on.
                let admitted = if held.navigation {
                    self.admit(held)
                        .await
                        .map(|()| None)
                        .map_err(|refusal| (refusal, None))
                } else {
                    self.admit_for_worker(held).await
                };
                let mut command = command;
                match admitted {
                    Err((refusal, carried)) => {
                        return Err(self.gate_refused(held, &refusal, carried.as_deref()).await);
                    }
                    // The document it brings back is judged when it arrives.
                    Ok(Some(carried)) => {
                        lock(&self.carried).insert(id.clone(), Arc::downgrade(&carried));
                        command.intercept_response = Some(true);
                        gated = Some(carried);
                    }
                    Ok(None) => {}
                }
                self.trace_service_worker(held, None);
                command
            }
            Ok(steps::Sent::StandIn(stand_in)) => {
                self.trace_service_worker(held, None);
                if let Err(failure) = self.driver.command(&stand_in).await {
                    tracing::debug!(
                        ?failure,
                        "a paused request went before its stand-in answered it"
                    );
                }
                return Ok(());
            }
            Err(Stopped::Blocked { by_list }) => {
                if by_list {
                    self.state.blocked_by_lists();
                }
                return Err(self.blocked(held).await);
            }
            Err(Stopped::Refused(refused)) => {
                // The refusal waits before the request fails, so its
                // failure carries it, and the navigation waiting on it
                // raises it; no call waits on any other request, which the
                // Run trace then records.
                self.leave_refusal(held, refused.failure());
                if held.navigation {
                    self.state.leave(refused.error());
                }
                self.fail(id, network::ErrorReason::BlockedByClient).await;
                return Err(refused.error());
            }
        };
        if held.navigation {
            *lock(&self.document) = Arc::from(held.url());
        }
        self.sent_on(held, &command, fetch, gated.as_deref()).await
    }

    /// Sends `held` on with `command`, then tells the gate that admitted it
    /// that the browser took the command, so a delay counts from its send
    /// (ADR 0086): the page's own for a hop of its main frame's
    /// navigation, the page's whose navigation it carries, `carried`, for a
    /// service worker's.
    ///
    /// # Errors
    ///
    /// For a `fetch`, `closed` when the request went before it was sent on.
    async fn sent_on(
        &self,
        held: &Held,
        command: &fetch::ContinueRequestParams,
        fetch: bool,
        carried: Option<&PageState>,
    ) -> Result<(), CallError> {
        if let Err(failure) = self.driver.command(command).await {
            tracing::debug!(?failure, "a paused request went before it was sent on");
            // A `fetch` whose request was never sent on gets no response.
            if fetch {
                return Err(gone(held.url()));
            }
            return Ok(());
        }
        if held.navigation {
            self.driver.sent(held.url()).await;
        } else if let Some(carried) = carried {
            carried
                .navigating
                .as_ref()
                .map_or(&self.driver, |navigating| &navigating.driver)
                .sent(held.url())
                .await;
        }
        Ok(())
    }

    /// Fails `held` with [`Self::cookies_withheld`]'s refusal, which the
    /// navigation waiting on it raises.
    async fn withhold_cookies(&self, held: &Held) -> Option<CallError> {
        let withheld = self.cookies_withheld(held.url()).await?;
        if held.navigation {
            self.state.leave(withheld.clone());
        }
        self.run.record_outcome(withheld.code());
        self.fail(
            &held.paused.request_id,
            network::ErrorReason::BlockedByClient,
        )
        .await;
        Some(withheld)
    }

    /// `secret_input_refused` when the browser would send `url`'s request,
    /// over TLS, any cookie of the Session's to a host its egress relay
    /// terminates for an invalid-certificate exception, the host's own
    /// host-only cookies included: the engine cannot tell one the host set
    /// through the exception from one the Session already held, so none
    /// goes, as the engine's own client sends none (ADR 0042). `closed`
    /// when the browser does not say which cookies it would send, so no
    /// request leaves unchecked.
    async fn cookies_withheld(&self, url: &str) -> Option<CallError> {
        let relay = self.driver.relay()?;
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_ascii_lowercase();
        if !matches!(parsed.scheme(), "https" | "wss") || !relay.terminates(&host) {
            return None;
        }
        let cookies = match self
            .driver
            .command(&network::GetCookiesParams {
                urls: Some(vec![url.to_owned()]),
            })
            .await
        {
            Ok(found) => found.cookies,
            Err(failure) => {
                tracing::debug!(
                    ?failure,
                    "the browser named no cookies, so the request to a terminated host is failed"
                );
                return Some(gone(url));
            }
        };
        let session = cookies.into_iter().next()?;
        Some(CallError::new(
            &SECRET_INPUT_REFUSED,
            format!(
                "The Session's cookie `{}` was not sent to {host}: the host is reached through an invalid-certificate exception, so whoever holds its key may not be the site (ADR 0042).",
                session.name
            ),
            &SecretInputRefusedDetails {
                placeholder: session.name,
                reference: SESSION_COOKIES.to_owned(),
                domain: host,
                reason: SecretInputRefusedReason::InvalidCertificateException,
            },
        ))
    }

    /// On a browser no relay is under, Borrowed or launched with none,
    /// judges the addresses the engine resolves the host of `url` to, as
    /// the browser would connect to them (ADR 0056; #2435): the browser
    /// resolves names itself, so an answer that changes after this lookup
    /// reaches it, the rebinding gap Provenance records. A name the engine
    /// cannot resolve is left to a browser on the engine's machine, which
    /// resolves it as the engine would, and refused on one elsewhere, or
    /// behind a proxy, whose network may resolve it to an intranet host or
    /// a cloud's metadata service (O9 of the deep review).
    ///
    /// # Errors
    ///
    /// [`Refused`] naming the first address a page may not reach, or
    /// `network.resolve` for a name the engine cannot resolve.
    async fn judged_addresses(&self, url: &str, kind: RequestKind) -> Result<(), Refused> {
        let Some(lookups) = &self.lookups else {
            return Ok(());
        };
        let judged = Gate::Session {
            policy: &self.policy,
            kind,
        }
        .judge(url)?;
        let Some(port) = judged.port().filter(|_| !judged.host().is_empty()) else {
            return Ok(());
        };
        match http_client::lookup(judged.lookup_name(), port, lookups.resolve()).await {
            Ok(addresses) => judged.addresses(&addresses),
            Err(failure) => {
                let location = lookups.location().await;
                tracing::debug!(
                    ?failure,
                    ?location,
                    "the engine resolved no address for a request no relay is under"
                );
                egress::judge_unresolved(url, &self.policy, location)
            }
        }
    }

    /// Admits `held`, a hop of the main frame's navigation: by the page's
    /// navigation gate, then by its Session's proxy's limits.
    async fn admit(&self, held: &Held) -> Result<(), Refusal> {
        self.admit_by_gate(held).await?;
        self.admit_through_proxy(held).await;
        Ok(())
    }

    /// Takes a Permit of the Session's proxy's limits for `held`, a hop of
    /// the main frame's navigation, a program's own included, keeping it
    /// until the hop ends, once the limits admit it (ADRs 0053 and 0088):
    /// the hop before gives its Permit back first. The wait counts against
    /// no deadline of the page. A page without a proxy, and a hop
    /// `proxy.bypass` sends around it, go at once.
    async fn admit_through_proxy(&self, held: &Held) {
        if self.driver.proxy().is_none() {
            return;
        }
        let network_id = held.paused.network_id.clone();
        drop(lock(&self.proxy_permit).take());
        if let Some(permit) = self.driver.proxy_permit(held.url()).await {
            *lock(&self.proxy_permit) = Some((network_id, permit));
        }
    }

    /// The Permit of the Session's proxy's limits the main frame's
    /// navigation holds for the hop the network ID `network_id` names, or
    /// for its latest hop when `network_id` is `None`, taken off the page.
    fn proxy_hop(&self, network_id: Option<&network::RequestId>) -> Option<ProxyPermit> {
        let mut held = lock(&self.proxy_permit);
        let matches = held.as_ref().is_some_and(|(id, _)| {
            network_id.is_none_or(|network_id| id.as_ref() == Some(network_id))
        });
        if matches {
            held.take().map(|(_, permit)| permit)
        } else {
            None
        }
    }

    /// Leaves for the waiting navigation, or the page's next call, the
    /// egress relay's refusal of the document load that `failed` failed: a
    /// hop the relay refused at the address it connects to, which the
    /// browser reports only as a network error.
    fn refused_by_relay(&self, failed: &network::LoadingFailedEvent) {
        let (Some(relay), Some(_)) = (self.driver.relay(), self.sender.page_id()) else {
            return;
        };
        if failed.r#type != network::ResourceType::Document {
            return;
        }
        let document = Arc::clone(&*lock(&self.document));
        if let Some(refused) = relay.take_refusal(&document, None) {
            self.state.leave(refused.error());
        }
    }

    /// Ends the hop of the main frame's navigation that `failed` failed:
    /// its proxy's Permit goes back as the proxy's refusal when the proxy
    /// refused the tunnel, lowering the proxy's ceiling, and as a request
    /// that ended otherwise.
    fn proxy_hop_failed(&self, failed: &network::LoadingFailedEvent) {
        let Some(permit) = self.proxy_hop(Some(&failed.request_id)) else {
            return;
        };
        let document = Arc::clone(&*lock(&self.document));
        let refused = self.driver.proxy().is_some_and(|proxy| {
            proxy.refusal(
                &document,
                proxy::Answer::Failure {
                    error: &NetError::of(&failed.error_text),
                },
                None,
            )
        });
        if refused {
            self.run.spawn(permit.refused());
        }
    }

    /// Gives back, as the proxy's refusal, the proxy's Permit of the hop
    /// whose document's response, paused at `paused` with `status`, the
    /// Session's proxy gave itself with a 429 or a 502 (the proxy-or-site
    /// rule), before the response goes on, so the proxy's ceiling is
    /// lowered by the time the navigation's call answers. A site's
    /// response, of any status, leaves the Permit to its hop's end.
    async fn proxy_answered(&self, paused: &fetch::RequestPausedEvent, status: u16) {
        let Some(proxy) = self.driver.proxy() else {
            return;
        };
        let proxy_status = paused
            .response_headers
            .iter()
            .flatten()
            .find(|header| header.name.eq_ignore_ascii_case(PROXY_STATUS))
            .map(|header| header.value.as_str());
        let answer = proxy.response(&paused.request.url, proxy_status);
        if !proxy.refusal(&paused.request.url, answer, Some(status)) {
            return;
        }
        if let Some(permit) = self.proxy_hop(paused.network_id.as_ref()) {
            permit.refused().await;
        }
    }

    /// Asks the page's navigation gate to admit `held`, a hop of the main
    /// frame's navigation, keeping its permit until the hop ends: the
    /// permit of the hop before is released first, since its 3xx or its
    /// refresh stub ended it. The wait counts against no deadline of the
    /// page, and its lease check holds the lease through it. A page with no
    /// gate admits it at once.
    async fn admit_by_gate(&self, held: &Held) -> Result<(), Refusal> {
        if self.driver.value().gate().is_none() {
            return Ok(());
        }
        let network_id = held.paused.network_id.clone();
        let before = lock(&self.permit).take();
        let redirect = before
            .as_ref()
            .is_some_and(|(id, _)| id.is_some() && *id == network_id);
        drop(before);
        let navigation = Navigation {
            url: held.url().to_owned(),
            method: held.method().to_owned(),
            kind: if redirect {
                NavigationKind::Redirect
            } else {
                NavigationKind::Document
            },
        };
        if let Some(permit) = self.driver.admit(&navigation).await? {
            *lock(&self.permit) = Some((network_id, permit));
        }
        Ok(())
    }

    /// Asks the navigation gate to admit `held`, a service worker's request,
    /// when it carries a navigation pending on a page of its Session: one to
    /// that navigation's URL, fragment removed, in `navigate` mode or any
    /// other. Its permit is the navigation's, held until the page's `load`,
    /// and the navigation stays its page's: the page's gate admits it, the
    /// wait counting against none of the page's deadlines. Any other
    /// request is admitted at once. Answers the page whose navigation it
    /// carries, whose gate judges the document it brings back.
    ///
    /// # Errors
    ///
    /// The gate's refusal, with the page whose navigation it carries.
    async fn admit_for_worker(
        &self,
        held: &Held,
    ) -> Result<Option<Arc<PageState>>, (Refusal, Option<Arc<PageState>>)> {
        if !matches!(self.sender, Sender::ServiceWorker { .. })
            || self.driver.value().gate().is_none()
        {
            return Ok(None);
        }
        let Some((page, carried)) = self.state.pending.take(held.url()) else {
            return Ok(None);
        };
        let carried = carried.upgrade();
        let driver = carried
            .as_ref()
            .and_then(|state| state.navigating.as_ref())
            .map_or(&self.driver, |navigating| &navigating.driver);
        let navigation = Navigation {
            url: held.url().to_owned(),
            method: held.method().to_owned(),
            kind: NavigationKind::ServiceWorker,
        };
        match driver.admit(&navigation).await {
            Ok(Some(permit)) => self.state.pending.hold(&page, permit),
            Ok(None) => {}
            Err(refusal) => return Err((refusal, carried)),
        }
        Ok(carried)
    }

    /// Aborts `held`, which the page's navigation gate refused, and answers
    /// `site_preferences_refused`: a navigation call waiting on the page
    /// raises it; with none waiting, its `page.request_failed` carries it,
    /// the Run trace records it and the waiters armed for it raise it.
    /// Aborted, the page keeps its document. A service worker's request
    /// that carried the navigation of the page `carried` raises it from
    /// that page's navigation call or waiters.
    async fn gate_refused(
        &self,
        held: &Held,
        refusal: &Refusal,
        carried: Option<&PageState>,
    ) -> CallError {
        let url = held.url();
        let page = carried.and_then(|state| state.navigating.as_ref().map(|page| (state, page)));
        let waiting = match page {
            Some((_, page)) => page.driver.navigation_waits(),
            None => self.driver.navigation_waits(),
        };
        if !matches!(self.sender, Sender::Page(_)) {
            // A worker's or a frame's request, which no page's event
            // reports, is recorded refused at once.
            self.leave_refusal(held, gate_failure(url, refusal));
        } else if let Some(network_id) = &held.paused.network_id {
            self.refusals
                .leave(network_id, gate_failure(url, refusal), !waiting);
        }
        match page {
            Some((state, _)) if waiting => state.leave(gate_refused(url, refusal)),
            // No call waits on it: the waiters armed for it raise it.
            Some((_, page)) => page
                .refusals
                .refuse_navigation(url, gate_refused(url, refusal)),
            None if waiting => self.state.leave(gate_refused(url, refusal)),
            None if held.navigation => self
                .refusals
                .refuse_navigation(url, gate_refused(url, refusal)),
            None => {}
        }
        self.fail(&held.paused.request_id, network::ErrorReason::Aborted)
            .await;
        gate_refused(url, refusal)
    }

    /// How step 3 judges `held`: a hop of the main frame's navigation while
    /// a navigation call that may load a `file:` URL goes on as that call's,
    /// any other request as the page's own.
    fn egress_kind(&self, held: &Held) -> RequestKind {
        match *lock(&self.state.files) {
            Some(navigator) if held.navigation => RequestKind::Navigation(navigator),
            _ => RequestKind::Page,
        }
    }

    /// Refuses the navigation the page scheduled for its frame `frame_id`
    /// to `url`, when its scheme is one no request may use: the browser
    /// sends no request for it, and the call waiting on the page's
    /// navigation, or else its next call, raises the refusal.
    fn scheduled(&self, frame_id: &page::FrameId, url: &str) {
        if self.sender.page_id().is_none() || frame_id != self.driver.main_frame() {
            return;
        }
        if let Some(refused) = hops::scheduled(url, &self.policy) {
            tracing::debug!(%refused, "the page scheduled a navigation the egress policy refuses");
            self.state.leave(refused.error());
        }
    }

    /// Refuses the redirect paused at `paused`, whose `Location` names a
    /// scheme no request may use, as step 3 refuses a request: it fails
    /// carrying the refusal, which the navigation waiting on it raises, or
    /// else the Run trace records.
    async fn refuse_redirect(&self, paused: &fetch::RequestPausedEvent, refused: &Refused) {
        let navigation = self.main_document(paused);
        if self.sender.records_at_once() {
            self.record_refusal(
                &paused.request.url,
                &paused.request.method,
                refused.failure(),
            );
        } else if let Some(network_id) = &paused.network_id {
            self.refusals
                .leave(network_id, refused.failure(), !navigation);
        }
        if navigation {
            self.state.leave(refused.error());
        }
        self.fail(&paused.request_id, network::ErrorReason::BlockedByClient)
            .await;
    }

    /// Acts on the response paused at `paused`, which no `fetch` waits for:
    /// a redirect to a scheme no request may use is refused; a main-frame
    /// document's, or the one a service worker's request carrying a page's
    /// navigation brought back, goes on once the page's navigation gate
    /// judged it, and on its refusal the page is left on a blank document,
    /// the navigation call waiting on it raising the refusal; any other
    /// response goes on as it came.
    async fn responded(&self, paused: &fetch::RequestPausedEvent) {
        let document = self.main_document(paused);
        let carried = lock(&self.carried)
            .remove(&paused.request_id)
            .and_then(|page| page.upgrade());
        let status = cdp_response::paused_status(paused);
        if document {
            self.proxy_answered(paused, status).await;
        }
        if let Some(refused) = hops::redirect(
            &paused.request.url,
            status,
            paused.response_headers.as_deref().unwrap_or_default(),
            &self.policy,
        ) {
            self.refuse_redirect(paused, &refused).await;
            return;
        }
        // A handler's page reads its main frame's documents through its
        // document reader (ADR 0119), a redirect's excepted.
        if document
            && paused.response_status_code.is_some()
            && !cdp_response::redirects(status)
            && let Some(reader) = self.driver.reader()
        {
            self.read_document(reader, paused, status).await;
            return;
        }
        // A response that never came (an error-stage pause) has no document
        // to judge.
        if (document || carried.is_some())
            && paused.response_status_code.is_some()
            && self
                .refused_document(paused, status, carried.as_deref(), None)
                .await
                .is_some()
        {
            return;
        }
        self.continue_response(&paused.request_id).await;
    }

    /// Reads the main frame's document whose response is paused at
    /// `paused` with `status` through the page's document `reader`, before
    /// anything renders it (ADR 0119): its body is read only when the
    /// reader asks for it, then the navigation gate judges it, and it goes
    /// on as it came, in the charset the reader names, or not at all, the
    /// page then left on a blank document and no download started. The
    /// navigation it answers keeps the site's own headers.
    async fn read_document(
        &self,
        reader: &Reader,
        paused: &fetch::RequestPausedEvent,
        status: u16,
    ) {
        let Some(page_id) = self.sender.page_id() else {
            return;
        };
        let headers = cdp_response::paused_header_list(
            paused.response_headers.as_deref().unwrap_or_default(),
        );
        let request_id = paused.network_id.clone().unwrap_or_default();
        let document = ReadDocument {
            page_id,
            request_id: &request_id,
            own: reader.take_own(),
            url: &paused.request.url,
            status,
            headers: &headers,
        };
        let mut body = None;
        let mut reading = reader.read(&document, None);
        if reading == Reading::Body {
            let read = self.body(&paused.request_id).await;
            reading = reader.read(&document, Some(&read));
            body = Some(read);
        }
        // A response read no further is judged by its headers alone.
        let judged = body.as_deref().unwrap_or_default();
        if self
            .refused_document(paused, status, None, Some(judged))
            .await
            .is_some()
        {
            return;
        }
        let site = |stopped| SiteResponse {
            headers: headers.iter().cloned().collect(),
            stopped,
        };
        match reading {
            Reading::Stop => {
                reader.keep(&request_id, site(true));
                tracing::debug!(
                    url.origin = goodfetch_engine::log::origin(&paused.request.url),
                    "a handler's page's response is no document it reads: its navigation stopped at the interception"
                );
                self.fulfill(
                    &paused.request_id,
                    Fulfillment {
                        status,
                        headers: BTreeMap::from([(
                            "content-type".to_owned(),
                            "text/html".to_owned(),
                        )]),
                        body: String::new(),
                    },
                )
                .await;
            }
            Reading::Charset(charset) => {
                reader.keep(&request_id, site(false));
                let essence = headers
                    .iter()
                    .find(|(name, _)| name == "content-type")
                    .and_then(|(_, value)| value.split(';').next())
                    .map(str::trim)
                    .filter(|essence| !essence.is_empty())
                    .unwrap_or("text/html");
                // The body was read, after its content codings, so it is
                // answered whole: the browser takes a charset only from
                // the response it decodes.
                let mut entries: Vec<fetch::HeaderEntry> = paused
                    .response_headers
                    .iter()
                    .flatten()
                    .filter(|header| {
                        !["content-type", "content-encoding", "content-length"]
                            .iter()
                            .any(|name| header.name.eq_ignore_ascii_case(name))
                    })
                    .cloned()
                    .collect();
                entries.push(fetch::HeaderEntry {
                    name: "Content-Type".to_owned(),
                    value: format!("{essence}; charset={charset}"),
                });
                self.fulfill_whole(&paused.request_id, status, entries, judged)
                    .await;
            }
            Reading::AsItCame | Reading::Body => {
                self.continue_response(&paused.request_id).await;
            }
        }
    }

    /// Judges the main-frame document whose response is paused at `paused`
    /// with `status` by the page's navigation gate, before anything renders
    /// it or a route gets it: `carried` is the page whose navigation a
    /// service worker's request carried, none for this pipeline's own page.
    /// On a refusal the request is answered with a blank document, and the
    /// refusal raises from the page's navigation call, or else from the
    /// waiters armed for it; the refusal is answered. A redirect, and a page
    /// with no gate, are never judged; `body` is the response's when it was
    /// read already, and is read only for a gate that reads it.
    async fn refused_document(
        &self,
        paused: &fetch::RequestPausedEvent,
        status: u16,
        carried: Option<&PageState>,
        body: Option<&[u8]>,
    ) -> Option<CallError> {
        let gate = self
            .driver
            .value()
            .gate()
            .filter(|_| !cdp_response::redirects(status))?;
        let read;
        let body = match body {
            Some(body) => body,
            // A gate that reads no body costs the browser no read of it.
            None if !gate.reads_document_body() => &[],
            None => {
                read = self.body(&paused.request_id).await;
                &read
            }
        };
        let headers = cdp_response::paused_header_list(
            paused.response_headers.as_deref().unwrap_or_default(),
        );
        let response = DocumentResponse {
            url: &paused.request.url,
            status,
            headers: &headers,
            body,
        };
        let refusal = gate.judge_document(&response).await.err()?;
        let error = gate_refused(&paused.request.url, &refusal);
        let page = carried.and_then(|state| state.navigating.as_ref().map(|page| (state, page)));
        let (state, driver, refusals) = match page {
            Some((state, page)) => (state, &page.driver, &page.refusals),
            None => (&*self.state, &self.driver, &self.refusals),
        };
        if driver.navigation_waits() {
            state.leave(error.clone());
        } else {
            tracing::info!(
                code = error.code(),
                url.origin = goodfetch_engine::log::origin(&paused.request.url),
                "the navigation gate refused a document no call waited on: the page is left blank"
            );
            refusals.refuse_navigation(&paused.request.url, error.clone());
            // The Run trace holds the Error code no reply carries (ADR 0072).
            self.run.record_outcome(error.code());
        }
        self.fulfill(
            &paused.request_id,
            Fulfillment {
                status: 200,
                headers: BTreeMap::from([("content-type".to_owned(), "text/html".to_owned())]),
                body: String::new(),
            },
        )
        .await;
        Some(error)
    }

    /// The body of the response paused at `id`, after any content coding;
    /// empty, and logged why, when the browser gives none, one past what a
    /// message carries, or one that does not decode.
    async fn body(&self, id: &fetch::RequestId) -> Vec<u8> {
        cdp_response::whole_body(&self.driver, id)
            .await
            .unwrap_or_default()
    }

    /// Fails `held`, which step 4 blocked, as `BlockedByClient`, as a
    /// blocker does: a navigation's call raises the browser's
    /// `navigation_failed`, and any other request's failure is recorded in
    /// the Run trace. Answers the error a `fetch` of it raises.
    async fn blocked(&self, held: &Held) -> CallError {
        let url = held.url().to_owned();
        let details = NavigationFailedDetails {
            url: Some(url.clone()),
            error: Some(NetError::BlockedByClient.name().to_owned()),
        };
        let error = CallError::new(
            &NAVIGATION_FAILED,
            format!("{url} was blocked by the Session's Filter lists or `browser.block_resources`"),
            &details,
        );
        let details = match serde_json::to_value(&details) {
            Ok(serde_json::Value::Object(details)) => details,
            _ => serde_json::Map::new(),
        };
        self.leave_refusal(
            held,
            ErrorData {
                code: NAVIGATION_FAILED.name.to_owned(),
                retryable: NAVIGATION_FAILED.retryable,
                details,
            },
        );
        self.fail(
            &held.paused.request_id,
            network::ErrorReason::BlockedByClient,
        )
        .await;
        error
    }

    /// Leaves `refusal` for `held`'s failure to carry, before the browser
    /// fails it: the Run trace records it unless the request is the
    /// navigation a call waits on, which raises it. A request no page's
    /// event reports, a frame's or a worker's, is recorded refused at once.
    fn leave_refusal(&self, held: &Held, refusal: ErrorData) {
        if self.sender.records_at_once() {
            self.record_refusal(held.url(), held.method(), refusal);
        } else if matches!(self.sender, Sender::ServiceWorker { .. }) {
            self.trace_service_worker(
                held,
                Some(RequestFailure {
                    error_text: NetError::BlockedByClient.name().to_owned(),
                    error: Some(refusal),
                }),
            );
        } else if let Some(network_id) = &held.paused.network_id {
            // A navigation the page sent itself, which no call waits on, is
            // recorded too.
            let recorded = !held.navigation || !self.driver.navigation_waits();
            self.refusals.leave(network_id, refusal, recorded);
        }
    }

    /// Judges the WebSocket the page opened to `url`, whose handshake no
    /// interception sees: the browser itself holds it to the domain policy
    /// where it connects (`launch`'s proxy script), and the engine records
    /// the refusal in the Run trace, as a request no call waited for that
    /// failed. On a Borrowed browser or behind the Session's own proxy
    /// nothing holds it, which is logged.
    fn web_socket(&self, url: &str) {
        let request = egress::Request {
            url,
            kind: RequestKind::Page,
        };
        let egress::Judgment::Refused(refused) = egress::judge(&request, &self.policy) else {
            return;
        };
        // Behind the egress relay, the relay refuses it as it connects and
        // records the refusal.
        if self.driver.relay().is_some() {
            return;
        }
        let borrowed = self
            .lookups
            .as_ref()
            .is_some_and(|lookups| lookups.borrowed());
        if borrowed || self.driver.proxy().is_some() {
            tracing::warn!(%refused, "a WebSocket the domain policy refuses is not held where it connects: the Session's browser or proxy is not the engine's");
            return;
        }
        self.record_refusal(url, "GET", refused.failure());
    }

    /// Records the request to `url` that `method` sent, refused with
    /// `refusal`, in the sender's request log, as a request that no call
    /// waited for failed, and its Error code in the Run's outcome record.
    fn record_refusal(&self, url: &str, method: &str, refusal: ErrorData) {
        self.run.record_outcome(&refusal.code);
        let (log, failed) = (
            Arc::clone(&self.log),
            RequestFailed {
                url: url.to_owned(),
                method: method.to_owned(),
                failure: refusal,
            },
        );
        self.run
            .metrics()
            .counted_work()
            .count(Row::InterceptedSpawns);
        self.run.spawn(async move {
            if let Err(error) = log.request_failed(&failed).await {
                tracing::warn!(%error, "the Run trace did not take a refused request");
            }
        });
    }

    /// Records `held`, a service worker's own request, in the Run trace,
    /// sent on or refused with `failure` (ADR 0121); a page's request is
    /// its page's events'.
    fn trace_service_worker(&self, held: &Held, failure: Option<RequestFailure>) {
        let Sender::ServiceWorker { session, log } = &self.sender else {
            return;
        };
        if failure.is_some() {
            self.run
                .metrics()
                .counted_work()
                .count(Row::InterceptedSpawns);
        }
        let request = ServiceWorkerRequest {
            session_id: session.clone(),
            url: held.url().to_owned(),
            method: held.method().to_owned(),
            resource_type: resource_type(held.paused.resource_type),
            failure,
        };
        let log = Arc::clone(log);
        self.run.spawn(async move {
            if let Err(error) = log.service_worker_request(&request).await {
                tracing::warn!(%error, "the Run trace did not take a service worker's request");
            }
        });
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

    /// Answers the request `id` with `status`, `headers`, each as given,
    /// and `body`.
    async fn fulfill_whole(
        &self,
        id: &fetch::RequestId,
        status: u16,
        headers: Vec<fetch::HeaderEntry>,
        body: &[u8],
    ) {
        let fulfilled = self
            .driver
            .command(&fetch::FulfillRequestParams {
                request_id: id.clone(),
                response_code: i64::from(status),
                response_headers: Some(headers),
                binary_response_headers: None,
                body: Some(BASE64.encode(body)),
                response_phrase: None,
            })
            .await;
        if let Err(failure) = fulfilled {
            tracing::debug!(?failure, "a paused request went before it was fulfilled");
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

/// The headers of the response paused at `paused`, for its body after any
/// content coding: its `Content-Encoding` and `Content-Length` go, as the
/// body is answered decoded and whole.
fn decoded_headers(paused: &fetch::RequestPausedEvent) -> Vec<fetch::HeaderEntry> {
    paused
        .response_headers
        .iter()
        .flatten()
        .filter(|header| {
            !["content-encoding", "content-length"]
                .iter()
                .any(|name| header.name.eq_ignore_ascii_case(name))
        })
        .cloned()
        .collect()
}

/// The URL in `headers`' `Referer`, which names the document that sent the
/// request.
fn referer(headers: &network::Headers) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("referer"))
        .and_then(|(_, value)| value.as_str())
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

/// `closed` for a fetched request whose response never came: its page,
/// its request or its Run went first.
fn gone(url: &str) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the response to {url} never came: its page, its request or its Run went first"),
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
