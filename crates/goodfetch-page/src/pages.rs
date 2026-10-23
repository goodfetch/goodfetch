//! The page API's operations, the formats of a page's document among them
//! (ADR 0007), over the engine's Sessions and their pages (`sessions`, ADRs
//! 0085 and 0092): `run.new_page` opens a page in the Run's default
//! Session, `run.new_session` a Session of the Run's own, `session.new_page`
//! a page in it and `session.close` closes it. A locator action reaches its
//! page's driver through the guard, under the Run's `timeout.action`. A
//! navigation the browser turns into a download answers no response and
//! publishes `page.download` on the engine's event stream. Every page is
//! under the interception from its opening: `page.route` and
//! `session.route` arm their routes there, `route.*` answer the requests
//! those routes hold, and a failure a route's handler left on a page is
//! raised by the navigation waiting on its request, or else by the page's
//! next call.

use std::collections::BTreeMap;

use goodfetch_engine::browser_cache::PinnedArtifact;
use goodfetch_engine::events::Events;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::route::{
    PAGE_ROUTE, PageRouteParams, ROUTE_ABORT, ROUTE_CONTINUE, ROUTE_FALLBACK, ROUTE_FETCH,
    ROUTE_FULFILL, RouteAbortParams, RouteAnswered, RouteFetched, RouteFulfillParams,
    RouteOverridesParams, SESSION_ROUTE, SessionRouteParams,
};
use goodfetch_protocol::page::session::{
    NewSessionParams, RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE, SessionClosed,
    SessionParams,
};
use goodfetch_protocol::page::{
    Acted, ClickParams, FillParams, FormatsParams, GotoParams, LOCATOR_CLICK, LOCATOR_FILL,
    LOCATOR_PRESS, Locator, Navigated, NewPageParams, PAGE_DOWNLOAD, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageDownload, PageFormats, PageId, PageParams,
    PressParams, RUN_NEW_PAGE, WaitUntil,
};

use crate::driver::{Arrived, Destination};
use crate::guard::{self, Action, Request, Window};
use crate::interception::{Answer, Fulfillment, Interception, Overrides, Reply, Scope};
use crate::sessions::{self, OpenPage, Sessions};
use crate::trace;

mod clean_up;

/// What the page crate adds to the engine (ADR 0087): its run state's
/// Sessions table and the handlers of `run.new_page`, of the Sessions'
/// operations, of the navigations `page.goto`, `page.reload`,
/// `page.go_back` and `page.go_forward`, of `page.formats` and of the
/// locator actions, and of the routes and their answers, whose browsers
/// come from `pins`, the builds the Product version pins.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    let interception = Interception::default();
    let pages = Pages {
        sessions: Sessions::new(pins, interception.clone()),
        interception,
    };
    let contribution = with_page_handlers(
        Contribution::new("page").with_run_state_tables(sessions::TABLES),
        &pages,
    );
    let contribution = with_session_handlers(contribution, &pages.sessions);
    let contribution = with_locator_handlers(contribution, &pages);
    with_route_handlers(contribution, &pages.interception)
}

/// Adds the handlers of `run.new_session`, `session.new_page` and
/// `session.close`.
fn with_session_handlers(contribution: Contribution, sessions: &Sessions) -> Contribution {
    let (opening, paging, closing) = (sessions.clone(), sessions.clone(), sessions.clone());
    contribution
        .with_handler(Handler::new(
            &RUN_NEW_SESSION,
            move |params: NewSessionParams, context: CallContext| {
                let sessions = opening.clone();
                async move { sessions.new_session(params, &context).await }
            },
        ))
        .with_handler(Handler::new(
            &SESSION_NEW_PAGE,
            move |params: SessionParams, context: CallContext| {
                let sessions = paging.clone();
                async move { sessions.new_page(&params.session_id, &context).await }
            },
        ))
        .with_handler(Handler::new(
            &SESSION_CLOSE,
            move |params: SessionParams, context: CallContext| {
                let sessions = closing.clone();
                async move {
                    sessions.close(&params.session_id, &context).await?;
                    Ok::<SessionClosed, CallError>(SessionClosed {})
                }
            },
        ))
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
                async move {
                    pages
                        .sessions
                        .new_default_page(&params.run_id, &context)
                        .await
                }
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
    sessions: Sessions,
    interception: Interception,
}

impl Pages {
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
            hiding: page.hiding.as_deref(),
        };
        guard::act(&page.driver, request, &page.clock).await?;
        Ok(Acted {})
    }

    /// The open page `page_id`, or `closed`; a failure a route's handler
    /// left on it is raised here, by the page's next call.
    async fn open(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        let page = self.sessions.page(page_id).await?;
        match self.interception.take_failure(page_id) {
            Some(failure) => Err(failure),
            None => Ok(page),
        }
    }

    /// The formats the request names, read from one serialization of the
    /// page's document, its shadow roots included, on a blocking thread of
    /// its Run's, once extraction-side removal took what its Session's
    /// cosmetic rules match (ADR 0047), which Provenance records.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_FORMATS.name, run_id = tracing::field::Empty))]
    async fn formats(&self, params: FormatsParams) -> Result<PageFormats, CallError> {
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let document = page.driver.document().await?;
        let request = params.formats;
        let cosmetic = self.interception.cosmetic(&params.page_id);
        let (formats, removed) = page
            .run
            .owner()
            .blocking(move || clean_up::read(&document, request, cosmetic.as_ref()))
            .await;
        if let Some(removed) = removed {
            trace::provenance(&page.run, &removed.entry(&params.page_id, &page.session)).await;
        }
        Ok(formats)
    }
}
