//! The page API's operations, the formats of a page's document among them
//! (ADR 0007), and the engine's pages (ADRs 0085 and 0092): each live Run's
//! default Session, opened by its first `run.new_page` and shared by every
//! later one, and each open page by its ID. A Run's pages and Session leave
//! with it: when it stops, its browser closes and its entries go. A locator
//! action reaches its page's driver through the guard, under the Run's
//! `timeout.action`. A navigation the browser turns into a download answers
//! no response and publishes `page.download` on the engine's event stream.
//! Every page is under the interception from its opening: `page.route`
//! and `session.route` arm their routes there, `route.*` answer the
//! requests those routes hold, and a failure a route's handler left on a
//! page is raised by the navigation waiting on its request, or else by the
//! page's next call. A page's events are published while `page.on` listens to
//! them, and `response.body` and `page.wait_for_request_finished` read its
//! responses' bodies (`page_events`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::Events;
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_extractor::{Context, Document};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::events::{
    PAGE_ON, PAGE_WAIT_FOR_REQUEST_FINISHED, PageOnEnded, PageOnParams, RESPONSE_BODY,
    ResponseBodyParams, WaitForRequestFinishedParams,
};
use goodfetch_protocol::page::route::UrlMatcher;
use goodfetch_protocol::page::route::{
    PAGE_ROUTE, PageRouteParams, ROUTE_ABORT, ROUTE_CONTINUE, ROUTE_FALLBACK, ROUTE_FETCH,
    ROUTE_FULFILL, RouteAbortParams, RouteAnswered, RouteFetched, RouteFulfillParams,
    RouteOverridesParams, SESSION_ROUTE, SessionRouteParams,
};
use goodfetch_protocol::page::{
    Acted, ClickParams, FillParams, FormatsParams, GotoParams, LOCATOR_CLICK, LOCATOR_FILL,
    LOCATOR_PRESS, Locator, Navigated, NewPageParams, PAGE_DOWNLOAD, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageDownload, PageFormats, PageId, PageOpened,
    PageParams, PressParams, RUN_NEW_PAGE, SessionId, WaitUntil,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::{
    SettingLimit, SettingsLayer, TIMEOUT_ACTION, TIMEOUT_NAVIGATION, TIMEOUT_WAIT,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::driver::{Arrived, Destination, LocalBrowser, PageDriver, SessionDriver};
use crate::guard::{self, Action, Request, Window};
use crate::interception::{Answer, Fulfillment, Interception, Overrides, Reply, Scope};
use crate::page_events::{self, PageEvents, Refusals};
use crate::url_pattern::UrlPattern;

/// A navigation's deadline when the Run's settings name none: 30 s.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The guard's window when the Run's settings name none: 30 s, the
/// setting's default.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// An app-state wait's deadline when the Run's settings name none: 30 s,
/// the setting's default.
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// What the page crate adds to the engine (ADR 0087): the handlers of
/// `run.new_page`, of the navigations `page.goto`, `page.reload`,
/// `page.go_back` and `page.go_forward`, of `page.formats` and of the
/// locator actions, and of the routes and their answers, whose browsers
/// come from `pins`, the builds the Product version pins.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    let pages = Pages {
        state: Arc::new(Mutex::new(State::default())),
        pins,
        interception: Interception::default(),
        events: page_events::Table::default(),
    };
    let contribution = with_page_handlers(Contribution::new("page"), &pages);
    let contribution = with_locator_handlers(contribution, &pages);
    let contribution = with_event_handlers(contribution, &pages.events);
    with_route_handlers(contribution, &pages.interception)
}

/// Adds the handlers of `page.on`, each listening from when its call
/// arrives until the call goes, of `page.wait_for_request_finished`, armed
/// when its call arrives, and of `response.body`.
fn with_event_handlers(contribution: Contribution, table: &page_events::Table) -> Contribution {
    let (listening, waiting, reading) = (table.clone(), table.clone(), table.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_ON,
            move |params: PageOnParams, _context: CallContext| {
                let listened = listening
                    .get(&params.page_id)
                    .map(|events| events.listen(params.event, Clock::system().wall()));
                async move {
                    let (listener, runtime) = listened?;
                    listener.until_gone(runtime).await?;
                    Ok::<PageOnEnded, CallError>(PageOnEnded {})
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_REQUEST_FINISHED,
            move |params: WaitForRequestFinishedParams, context: CallContext| {
                let armed = waiting.get(&params.page_id).and_then(|events| {
                    let pattern = params.url.as_ref().map(pattern_of).transpose()?;
                    Ok(events.wait_for_request_finished(pattern, context.surface()))
                });
                async move { armed?.await }
            },
        ))
        .with_handler(Handler::new(
            &RESPONSE_BODY,
            move |params: ResponseBodyParams, context: CallContext| {
                let events = reading.get(&params.page_id);
                let surface = context.surface();
                async move { events?.body(&params.request_id, surface).await }
            },
        ))
}

/// The engine's matcher for `matcher`.
fn pattern_of(matcher: &UrlMatcher) -> Result<UrlPattern, CallError> {
    Ok(match matcher {
        UrlMatcher::Glob { pattern } => UrlPattern::glob(pattern)?,
        UrlMatcher::Regex { pattern } => UrlPattern::regex(pattern)?,
    })
}

/// Adds the handlers of `page.route` and `session.route`, each arming its
/// route when its call arrives, and of the answers to their requests, each
/// delivered when its call arrives.
fn with_route_handlers(contribution: Contribution, interception: &Interception) -> Contribution {
    let (paging, sessioning) = (interception.clone(), interception.clone());
    let contribution = contribution
        .with_handler(Handler::new(
            &PAGE_ROUTE,
            move |params: PageRouteParams, context: CallContext| {
                let armed = paging.arm(
                    Scope::Page(&params.page_id),
                    &params.url,
                    context.client(),
                    context.id(),
                );
                async move { armed?.ended().await }
            },
        ))
        .with_handler(Handler::new(
            &SESSION_ROUTE,
            move |params: SessionRouteParams, context: CallContext| {
                let armed = sessioning.arm(
                    Scope::Session(&params.session_id),
                    &params.url,
                    context.client(),
                    context.id(),
                );
                async move { armed?.ended().await }
            },
        ));
    let (continuing, falling_back, fulfilling, aborting, fetching) = (
        interception.clone(),
        interception.clone(),
        interception.clone(),
        interception.clone(),
        interception.clone(),
    );
    contribution
        .with_handler(Handler::new(
            &ROUTE_CONTINUE,
            move |params: RouteOverridesParams, _context: CallContext| {
                let route_id = params.route_id.clone();
                let replied = continuing.answer(&route_id, Answer::Continue(overrides(params)));
                async move { answered(replied?.await) }
            },
        ))
        .with_handler(Handler::new(
            &ROUTE_FALLBACK,
            move |params: RouteOverridesParams, _context: CallContext| {
                let route_id = params.route_id.clone();
                let replied = falling_back.answer(&route_id, Answer::Fallback(overrides(params)));
                async move { answered(replied?.await) }
            },
        ))
        .with_handler(Handler::new(
            &ROUTE_FULFILL,
            move |params: RouteFulfillParams, _context: CallContext| {
                let route_id = params.route_id.clone();
                let replied = fulfilling.answer(&route_id, Answer::Fulfill(fulfillment(params)));
                async move { answered(replied?.await) }
            },
        ))
        .with_handler(Handler::new(
            &ROUTE_ABORT,
            move |params: RouteAbortParams, _context: CallContext| {
                let replied = aborting.answer(
                    &params.route_id,
                    Answer::Abort(params.error_code.unwrap_or_default()),
                );
                async move { answered(replied?.await) }
            },
        ))
        .with_handler(Handler::new(
            &ROUTE_FETCH,
            move |params: RouteOverridesParams, _context: CallContext| {
                let route_id = params.route_id.clone();
                let replied = fetching.answer(&route_id, Answer::Fetch(overrides(params)));
                async move {
                    match replied?.await? {
                        Reply::Fetched(fetched) => Ok::<RouteFetched, CallError>(fetched),
                        Reply::Answered => Err(CallError::protocol(
                            ProtocolFailure::InternalError,
                            "the pipeline answered a fetch with no response",
                        )),
                    }
                }
            },
        ))
}

/// The overrides `params` names.
fn overrides(params: RouteOverridesParams) -> Overrides {
    Overrides {
        url: params.url,
        method: params.method,
        headers: params.headers,
        post_data: params.post_data,
    }
}

/// The response `params` names: 200 and an empty body unless it says
/// otherwise, its `content_type` over its headers' own.
fn fulfillment(params: RouteFulfillParams) -> Fulfillment {
    let mut headers: BTreeMap<String, String> = params
        .headers
        .unwrap_or_default()
        .into_iter()
        .filter(|(name, _)| {
            params.content_type.is_none() || !name.eq_ignore_ascii_case("content-type")
        })
        .collect();
    if let Some(content_type) = params.content_type {
        headers.insert("content-type".to_owned(), content_type);
    }
    Fulfillment {
        status: params.status.unwrap_or(200),
        headers,
        body: params.body.unwrap_or_default(),
    }
}

/// What an answer to a route answers once the pipeline acted on it.
fn answered(reply: Result<Reply, CallError>) -> Result<RouteAnswered, CallError> {
    reply.map(|_| RouteAnswered {})
}

/// Adds the handlers of `run.new_page`, the navigations and `page.formats`.
fn with_page_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (opening, going, reloading, going_back, going_forward, formatting) = (
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
    );
    contribution
        .with_handler(Handler::new(
            &RUN_NEW_PAGE,
            move |params: NewPageParams, context: CallContext| {
                let pages = opening.clone();
                async move { pages.new_page(params, context).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GOTO,
            move |params: GotoParams, context: CallContext| {
                let pages = going.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Url(&params.url),
                            params.wait_until.unwrap_or_default(),
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_RELOAD,
            move |params: PageParams, context: CallContext| {
                let pages = reloading.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Reload,
                            WaitUntil::Load,
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GO_BACK,
            move |params: PageParams, context: CallContext| {
                let pages = going_back.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Back,
                            WaitUntil::Load,
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GO_FORWARD,
            move |params: PageParams, context: CallContext| {
                let pages = going_forward.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Forward,
                            WaitUntil::Load,
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_FORMATS,
            move |params: FormatsParams, _context: CallContext| {
                let pages = formatting.clone();
                async move { pages.formats(params).await }
            },
        ))
}

/// Adds the handlers of the locator actions.
fn with_locator_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (clicking, filling, pressing) = (pages.clone(), pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &LOCATOR_CLICK,
            move |params: ClickParams, context: CallContext| {
                let pages = clicking.clone();
                async move {
                    pages
                        .act(
                            LOCATOR_CLICK.name,
                            params.locator,
                            Ok(Action::Click),
                            &context,
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_FILL,
            move |params: FillParams, context: CallContext| {
                let pages = filling.clone();
                async move {
                    pages
                        .act(
                            LOCATOR_FILL.name,
                            params.locator,
                            Ok(Action::Fill(params.value)),
                            &context,
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_PRESS,
            move |params: PressParams, context: CallContext| {
                let pages = pressing.clone();
                async move {
                    let action = Action::press(&params.key);
                    pages
                        .act(LOCATOR_PRESS.name, params.locator, action, &context)
                        .await
                }
            },
        ))
}

/// The engine's pages, which every call of every Connection shares.
#[derive(Clone)]
struct Pages {
    state: Arc<Mutex<State>>,
    pins: Vec<PinnedArtifact>,
    interception: Interception,
    /// Each open page's events, which calls reach as they arrive.
    events: page_events::Table,
}

#[derive(Default)]
struct State {
    runs: BTreeMap<RunId, Arc<RunPages>>,
    pages: BTreeMap<PageId, OpenPage>,
    opened_sessions: u64,
    opened_pages: u64,
}

/// A live Run's default Session.
struct RunPages {
    session: SessionId,
    driver: SessionDriver<LocalBrowser>,
}

#[derive(Clone)]
struct OpenPage {
    run: Run,
    driver: PageDriver,
    /// Its Run's `timeout.action` and where it was set.
    action_timeout: (SettingLimit, Duration),
    /// Its Run's clock.
    clock: Clock,
}

impl Pages {
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(run_id = %params.run_id, page_id = tracing::field::Empty, session_id = tracing::field::Empty))]
    async fn new_page(
        &self,
        params: NewPageParams,
        context: CallContext,
    ) -> Result<PageOpened, CallError> {
        let run = context
            .runs()
            .live(params.run_id.clone())
            .await
            .map_err(|error| run_error(&error, &params.run_id))?;
        let navigation_timeout = run
            .settings()
            .and_then(|settings| settings.resolved().duration(TIMEOUT_NAVIGATION.name))
            .unwrap_or(NAVIGATION_TIMEOUT);
        let action_timeout = run
            .settings()
            .and_then(|settings| Deadline::of(settings.resolved(), &TIMEOUT_ACTION))
            .map_or_else(
                || {
                    let limit = SettingLimit {
                        key: TIMEOUT_ACTION.name.to_owned(),
                        value: json!(ACTION_TIMEOUT.as_millis()),
                        layer: SettingsLayer::Default,
                    };
                    (limit, ACTION_TIMEOUT)
                },
                |deadline| (deadline.limit().clone(), deadline.duration()),
            );
        let run_pages = self.default_session(&run, &context).await;
        let driver = run_pages.driver.new_page(navigation_timeout).await?;
        let page = {
            let mut state = self.state.lock().await;
            state.opened_pages += 1;
            PageId(format!("page-{}", state.opened_pages))
        };
        // The page's events are read from before its requests pause, so
        // every request of the page is followed.
        let wait = run
            .settings()
            .and_then(|settings| Deadline::of(settings.resolved(), &TIMEOUT_WAIT))
            .unwrap_or_else(|| {
                Deadline::given(
                    &TIMEOUT_WAIT,
                    u64::try_from(WAIT_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                    SettingsLayer::Default,
                )
            });
        let refusals = Refusals::default();
        let events = PageEvents::start(
            page.clone(),
            driver.clone(),
            &run,
            context.events().clone(),
            refusals.clone(),
            driver.events().await,
            wait,
        );
        self.events.insert(page.clone(), events);
        self.interception
            .start(&page, &run_pages.session, &driver, &run, refusals)
            .await?;
        let mut state = self.state.lock().await;
        let span = tracing::Span::current();
        span.record("page_id", page.0.as_str());
        span.record("session_id", run_pages.session.0.as_str());
        state.pages.insert(
            page.clone(),
            OpenPage {
                run: run.clone(),
                driver,
                action_timeout,
                clock: run.clock(),
            },
        );
        Ok(PageOpened {
            page_id: page,
            session_id: run_pages.session.clone(),
        })
    }

    /// The Run's default Session, opened now when it has none; its entries
    /// leave when the Run stops.
    async fn default_session(&self, run: &Run, context: &CallContext) -> Arc<RunPages> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.runs.get(run.id()) {
            return Arc::clone(existing);
        }
        state.opened_sessions += 1;
        let opened = Arc::new(RunPages {
            session: SessionId(format!("session-{}", state.opened_sessions)),
            driver: SessionDriver::new(LocalBrowser::new(
                run.clone(),
                context.runs().home().clone(),
                Pins {
                    pinned_by: PinnedBy {
                        product: context.identity().product,
                        product_version: context.identity().product_version.clone(),
                    },
                    artifacts: self.pins.clone(),
                },
            )),
        });
        state.runs.insert(run.id().clone(), Arc::clone(&opened));
        drop(state);
        self.interception.open_session(&opened.session);
        let pages = self.clone();
        let stopping = run.cancellation().clone();
        let run_id = run.id().clone();
        run.spawn(async move {
            stopping.cancelled().await;
            let mut state = pages.state.lock().await;
            let session = state.runs.remove(&run_id).map(|run| run.session.clone());
            let gone: Vec<PageId> = state
                .pages
                .iter()
                .filter(|(_, page)| page.run.id() == &run_id)
                .map(|(page_id, _)| page_id.clone())
                .collect();
            state.pages.retain(|_, page| page.run.id() != &run_id);
            drop(state);
            pages.events.close(&gone);
            if let Some(session) = session {
                pages.interception.close(&gone, &session);
            }
        });
        opened
    }

    /// Navigates the page `page_id` to `destination`, answering once its
    /// document reached the state `wait_until` names; a download it turns
    /// into is published on `events`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %page_id.0, operation = destination.operation(), run_id = tracing::field::Empty, url.origin = tracing::field::Empty))]
    async fn navigate(
        &self,
        page_id: &PageId,
        destination: Destination<'_>,
        wait_until: WaitUntil,
        events: &Events,
    ) -> Result<Navigated, CallError> {
        let span = tracing::Span::current();
        if let Destination::Url(url) = destination {
            span.record("url.origin", goodfetch_engine::log::origin(url));
        }
        let page = self.open(page_id).await?;
        span.record("run_id", page.run.id().as_str());
        let navigated = page.driver.navigate(destination, wait_until).await;
        // A failure the interception left while the navigation ran, such
        // as a route's handler failing on its document, is the
        // navigation's.
        if let Some(failure) = self.interception.take_failure(page_id) {
            return Err(failure);
        }
        match navigated? {
            Arrived::Document(response) => Ok(Navigated { response }),
            Arrived::Download {
                url,
                suggested_filename,
            } => {
                events
                    .publish(
                        &PAGE_DOWNLOAD,
                        Some(&page_id.0),
                        &PageDownload {
                            page_id: page_id.clone(),
                            url,
                            suggested_filename,
                        },
                    )
                    .await;
                Ok(Navigated { response: None })
            }
        }
    }

    /// Performs `action` on the element `locator` matches, through the
    /// guard; `action` is the params' own failure when they name none.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %locator.page_id.0, operation = operation, run_id = tracing::field::Empty))]
    async fn act(
        &self,
        operation: &'static str,
        locator: Locator,
        action: Result<Action, CallError>,
        context: &CallContext,
    ) -> Result<Acted, CallError> {
        let action = action?;
        let page = self.open(&locator.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let (limit, duration) = page.action_timeout.clone();
        let request = Request {
            operation,
            selector: &locator.selector,
            action,
            window: Window { limit, duration },
            surface: context.surface(),
        };
        guard::act(&page.driver, request, &page.clock).await?;
        Ok(Acted {})
    }

    /// The open page `page_id`, or `closed`; a failure a route's handler
    /// left on it is raised here, by the page's next call.
    async fn open(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        let page = self
            .state
            .lock()
            .await
            .pages
            .get(page_id)
            .cloned()
            .ok_or_else(|| {
                CallError::new(
                    &CLOSED,
                    format!("the page `{}` is closed, or its Run ended", page_id.0),
                    &json!({ "page_id": page_id }),
                )
            })?;
        match self.interception.take_failure(page_id) {
            Some(failure) => Err(failure),
            None => Ok(page),
        }
    }

    /// The formats the request names, read from one serialization of the
    /// page's document on a blocking thread of its Run's.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_FORMATS.name, run_id = tracing::field::Empty))]
    async fn formats(&self, params: FormatsParams) -> Result<PageFormats, CallError> {
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let document = page.driver.document().await?;
        let request = params.formats;
        Ok(page
            .run
            .owner()
            .blocking(move || {
                goodfetch_extractor::formats(
                    &Document {
                        html: &document.html,
                        url: &document.url,
                        base_url: &document.base_url,
                    },
                    &request,
                    &Context::default(),
                )
            })
            .await)
    }
}

fn run_error(error: &RunError, run_id: &RunId) -> CallError {
    CallError::new(
        &RUN_NOT_FOUND,
        format!("No live Run has the Run ID `{run_id}`: {error}"),
        &RunDetails {
            run_id: run_id.clone(),
            path: None,
        },
    )
}
