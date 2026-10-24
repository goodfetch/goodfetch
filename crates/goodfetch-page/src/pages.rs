//! The page API's operations, the formats of a page's document among them
//! (ADR 0007), over the engine's Sessions and their pages (`sessions`, ADRs
//! 0085 and 0092): `run.new_page` opens a page in the Run's default
//! Session, `run.new_session` a Session of the Run's own, `session.new_page`
//! a page in it and `session.close` closes it; `session.pages` lists a
//! Session's pages, its popups among them, `session.on` announces each one
//! as it opens, and `page.close` closes one. A locator action reaches its
//! page's driver through the guard, under the Run's `timeout.action`. A
//! navigation the browser turns into a download answers no response and
//! publishes `page.download` on the engine's event stream. `page.goto`
//! judges its URL's scheme first: a `file:` URL, or any other scheme but
//! HTTP's and `about:blank`, only for the program's own code, and a
//! `javascript:` URL only in a frame the domain policy allows
//! (`egress::navigation`). Every page is
//! under the interception from its opening: `page.route` and
//! `session.route` arm their routes there, `route.*` answer the requests
//! those routes hold, and a failure a route's handler left on a page is
//! raised by the navigation waiting on its request, or else by the page's
//! next call. A page's events are published while `page.on` listens to
//! them, and `response.body` and `page.wait_for_request_finished` read its
//! responses' bodies (`page_events`). `page.request` sends a request from
//! the page's Session through the browser-fetch tier (`browser_fetch`) and
//! keeps its response's body for `response.body`. `page.on_dialog` arms a
//! dialog handler and `dialog.*` answer the dialogs it is asked about
//! (`dialogs`); a failure a dialog handler left is raised by the action the
//! dialog blocked, or else by the page's next call. `page.evaluate` runs a
//! page function and `page.add_init_script` registers an init script
//! (`page_functions`), which the page's navigations wait for. The
//! app-state waits are armed when their call arrives (ADR 0080): those on
//! the page's requests, URL and load states in its events
//! (`page_events`), `page.wait_for_function` by running its function until
//! it is truthy (`page_functions`), and the locator waits through the
//! guard's check, each under the Run's `timeout.wait`. Every document a
//! navigation reached is judged by its Page check (`recovery`), and a page
//! that blocking may have broken is loaded once more with blocking off
//! (`blocking_off`, ADR 0047), the Product's classifier telling its Block
//! cause (`blocks`); on a page a program opened, Provenance records what
//! its robots.txt says of each document, never gating it (`robots_txt`,
//! ADR 0086).
//!
//! Size: one table of the page API's handlers over the engine's shared
//! pages, each call opening its page through one `open` that raises what
//! the page left for it, a Block cause among them, kept whole so every call
//! meets its page the same way.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::browser_cache::PinnedArtifact;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::{self, Navigator};
use goodfetch_engine::hostile_input::check_headers;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::dialog::{
    DIALOG_ACCEPT, DIALOG_DISMISS, DialogAcceptParams, DialogAnswered, DialogDismissParams,
    PAGE_ON_DIALOG, PageOnDialogParams,
};
use goodfetch_protocol::page::events::{
    PAGE_ON, PAGE_WAIT_FOR_REQUEST_FINISHED, PageOnEnded, PageOnParams, RESPONSE_BODY,
    ResponseBodyParams, WaitForRequestFinishedParams,
};
use goodfetch_protocol::page::functions::{
    AddInitScriptParams, EvaluateParams, Evaluated, InitScriptEnded, PAGE_ADD_INIT_SCRIPT,
    PAGE_EVALUATE, World,
};
use goodfetch_protocol::page::request::{MAX_REDIRECTS, PAGE_REQUEST, PageRequestParams};
use goodfetch_protocol::page::route::UrlMatcher;
use goodfetch_protocol::page::route::{
    PAGE_ROUTE, PageRouteParams, ROUTE_ABORT, ROUTE_CONTINUE, ROUTE_FALLBACK, ROUTE_FETCH,
    ROUTE_FULFILL, RouteAbortParams, RouteAnswered, RouteFetched, RouteFulfillParams,
    RouteOverridesParams, SESSION_ROUTE, SessionRouteParams,
};
use goodfetch_protocol::page::session::{
    NewSessionParams, RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE, SessionClosed,
    SessionParams,
};
use goodfetch_protocol::page::tabs::{
    self, PageCloseParams, PageClosed, SESSION_ON, SESSION_PAGES, SessionEventName, SessionOnParams,
};
use goodfetch_protocol::page::{
    Acted, ClickParams, FillParams, FormatsParams, GotoParams, LOCATOR_CLICK, LOCATOR_FILL,
    LOCATOR_PRESS, Locator, Navigated, NewPageParams, PAGE_FORMATS, PAGE_GO_BACK, PAGE_GO_FORWARD,
    PAGE_GOTO, PAGE_RELOAD, PageFormats, PageId, PageParams, PageResponse, PressParams,
    RUN_NEW_PAGE, WaitUntil,
};
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};

use crate::blocks::{BlockClassifier, NoBlocks};
use crate::browser_fetch;
use crate::dialogs::{self, Dialogs};
use crate::driver::{Destination, SerializedDocument};
use crate::guard::{self, Action, Request, Window};
use crate::interception::{Answer, Fulfillment, Interception, Overrides, Reply, Scope};
use crate::page_events::{self, Requested};
use crate::page_functions::{self, InitScripts, Injection, Injections, Pending, Registered};
use crate::page_value::{HandlerPages, ProgramPages};
use crate::proxy;
use crate::recovery;
use crate::sessions::{self, OpenPage, PageEvent, Sessions};
use crate::trace;
use crate::url_pattern::UrlPattern;

mod arrival;
mod blocked;
mod blocking_off;
mod clean_up;
mod robots_txt;
mod waits;

/// What the page crate adds to the engine (ADR 0087): its run state's
/// Sessions table and the handlers of `run.new_page`, of the Sessions'
/// operations, of the navigations `page.goto`, `page.reload`,
/// `page.go_back` and `page.go_forward`, of `page.formats` and of the
/// locator actions, and of the routes and their answers, whose browsers
/// come from `pins`, the builds the Product version pins.
/// Every page it opens carries an empty value and registers no response
/// observer, as a Collie program's pages do, and no document has a Block
/// cause.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    contribution_with(pins, Arc::new(ProgramPages), Arc::new(NoBlocks))
}

/// What the page crate adds to the engine, as [`contribution`], each page
/// it opens taking its value and its response observer from `hooks` (ADR
/// 0087): the hook through which Foxhound sets them on the pages it hands
/// to handlers; `blocks` tells a page's Block cause, from the Product's
/// rules (ADR 0035).
pub fn contribution_with(
    pins: Vec<PinnedArtifact>,
    hooks: Arc<dyn HandlerPages>,
    blocks: Arc<dyn BlockClassifier>,
) -> Contribution {
    let interception = Interception::default();
    let events = page_events::Table::default();
    let dialogs = Dialogs::default();
    let pages = Pages {
        sessions: Sessions::new(
            pins,
            interception.clone(),
            events.clone(),
            dialogs.clone(),
            Arc::clone(&hooks),
        ),
        interception,
        events,
        dialogs,
        hooks,
        blocks,
        init_scripts: InitScripts::default(),
        injections: Injections::default(),
    };
    let contribution = with_page_handlers(
        Contribution::new("page")
            .with_run_state_tables(sessions::TABLES)
            .with_politeness_tables(proxy::TABLES)
            .with_clean_up_versions(pages.interception.versions()),
        &pages,
    );
    let contribution = with_session_handlers(contribution, &pages.sessions);
    let contribution = with_locator_handlers(contribution, &pages);
    let contribution = with_event_handlers(contribution, &pages.events);
    let contribution = with_route_handlers(contribution, &pages.interception);
    let contribution = with_dialog_handlers(contribution, &pages.dialogs);
    let contribution = with_function_handlers(contribution, &pages);
    let contribution = waits::with_wait_handlers(contribution, &pages);
    with_request_handler(contribution, &pages)
}

/// Adds the handlers of `page.evaluate` and of `page.add_init_script`,
/// whose script is counted on its way in when its call arrives, so that a
/// navigation sent after it waits for it.
fn with_function_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (evaluating, registering) = (pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_EVALUATE,
            move |params: EvaluateParams, context: CallContext| {
                let pages = evaluating.clone();
                async move { pages.evaluate(params, context.surface()).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_ADD_INIT_SCRIPT,
            move |params: AddInitScriptParams, _context: CallContext| {
                let pending = registering.init_scripts.begin(&params.page_id);
                let pages = registering.clone();
                async move { pages.add_init_script(params, pending).await }
            },
        ))
}

/// Adds the handler of `page.request`.
fn with_request_handler(contribution: Contribution, pages: &Pages) -> Contribution {
    let requesting = pages.clone();
    contribution.with_handler(Handler::new(
        &PAGE_REQUEST,
        move |params: PageRequestParams, context: CallContext| {
            let pages = requesting.clone();
            async move { pages.request(params, context.surface()).await }
        },
    ))
}

/// The count of redirects `page.request` follows, `max_redirects` or its
/// default; `setting_invalid` above the browser-fetch tier's count, its
/// option spelled for `surface` (ADR 0118).
fn redirect_count(max_redirects: Option<u32>, surface: Surface) -> Result<u32, CallError> {
    let count = max_redirects.unwrap_or(MAX_REDIRECTS);
    if count <= MAX_REDIRECTS {
        return Ok(count);
    }
    let option = max_redirects_option(surface);
    Err(CallError::new(
        &SETTING_INVALID,
        format!(
            "`{option}` is {count}, above {MAX_REDIRECTS}: the browser-fetch tier that sends `page.request` follows at most {MAX_REDIRECTS} redirects"
        ),
        &SettingInvalidDetails {
            key: option.to_owned(),
            layer: Some(SettingsLayer::Run),
            reason: format!(
                "{count} is above {MAX_REDIRECTS}, the most redirects a request follows"
            ),
        },
    ))
}

/// `page.request`'s count of redirects, in `surface`'s spelling.
fn max_redirects_option(surface: Surface) -> &'static str {
    match surface {
        Surface::SdkTypescript => "maxRedirects",
        Surface::SdkPython
        | Surface::Rust
        | Surface::Cli
        | Surface::Mcp
        | Surface::Console
        | Surface::Cloud
        | Surface::BrowserHost
        | Surface::Worker => "max_redirects",
    }
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
                    let (armed, runtime) = listened?;
                    armed.until_gone(runtime).await?;
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

/// Adds the handlers of `run.new_session`, `session.new_page`,
/// `session.close`, `session.pages`, `session.on` and `page.close`.
fn with_session_handlers(contribution: Contribution, sessions: &Sessions) -> Contribution {
    let (opening, paging, closing) = (sessions.clone(), sessions.clone(), sessions.clone());
    let (listing, listening, closing_page) = (sessions.clone(), sessions.clone(), sessions.clone());
    contribution
        .with_handler(Handler::new(
            &SESSION_PAGES,
            move |params: SessionParams, context: CallContext| {
                let sessions = listing.clone();
                async move { sessions.pages(&params.session_id, &context).await }
            },
        ))
        .with_handler(Handler::new(
            &SESSION_ON,
            move |params: SessionOnParams, context: CallContext| {
                let sessions = listening.clone();
                async move {
                    match params.event {
                        SessionEventName::Page => {
                            sessions.listen_to_pages(&params.session_id, &context).await
                        }
                    }
                }
            },
        ))
        .with_handler(Handler::new(
            &tabs::PAGE_CLOSE,
            move |params: PageCloseParams, _context: CallContext| {
                let sessions = closing_page.clone();
                async move {
                    sessions
                        .close_page(&params.page_id, params.run_before_unload.unwrap_or(false))
                        .await?;
                    Ok::<PageClosed, CallError>(PageClosed {})
                }
            },
        ))
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
                let replied = headers_sent(params.headers.as_ref()).and_then(|()| {
                    continuing.answer(&route_id, Answer::Continue(overrides(params)))
                });
                async move { answered(replied?.await) }
            },
        ))
        .with_handler(Handler::new(
            &ROUTE_FALLBACK,
            move |params: RouteOverridesParams, _context: CallContext| {
                let route_id = params.route_id.clone();
                let replied = headers_sent(params.headers.as_ref()).and_then(|()| {
                    falling_back.answer(&route_id, Answer::Fallback(overrides(params)))
                });
                async move { answered(replied?.await) }
            },
        ))
        .with_handler(Handler::new(
            &ROUTE_FULFILL,
            move |params: RouteFulfillParams, _context: CallContext| {
                let route_id = params.route_id.clone();
                let replied = fulfillment(params).and_then(|fulfillment| {
                    fulfilling.answer(&route_id, Answer::Fulfill(fulfillment))
                });
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
                let replied = headers_sent(params.headers.as_ref())
                    .and_then(|()| fetching.answer(&route_id, Answer::Fetch(overrides(params))));
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

/// Adds the handlers of `page.on_dialog`, arming its handler when its call
/// arrives, and of the answers to its dialogs, each delivered when its call
/// arrives.
fn with_dialog_handlers(contribution: Contribution, dialogs: &Dialogs) -> Contribution {
    let (arming, accepting, dismissing) = (dialogs.clone(), dialogs.clone(), dialogs.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_ON_DIALOG,
            move |params: PageOnDialogParams, context: CallContext| {
                let armed = arming.arm(&params.page_id, context.client(), context.id());
                async move { armed?.ended().await }
            },
        ))
        .with_handler(Handler::new(
            &DIALOG_ACCEPT,
            move |params: DialogAcceptParams, _context: CallContext| {
                let replied = accepting.answer(
                    &params.dialog_id,
                    dialogs::Answer::Accept(params.prompt_text),
                );
                async move {
                    replied?.await?;
                    Ok::<DialogAnswered, CallError>(DialogAnswered {})
                }
            },
        ))
        .with_handler(Handler::new(
            &DIALOG_DISMISS,
            move |params: DialogDismissParams, _context: CallContext| {
                let replied = dismissing.answer(&params.dialog_id, dialogs::Answer::Dismiss);
                async move {
                    replied?.await?;
                    Ok::<DialogAnswered, CallError>(DialogAnswered {})
                }
            },
        ))
}

/// Refuses `headers`, which a call would have the browser send, when a
/// value holds a CR, an LF or a NUL: `header_value_invalid`, before
/// anything reaches CDP (ADR 0067).
fn headers_sent(headers: Option<&BTreeMap<String, String>>) -> Result<(), CallError> {
    check_headers(
        headers
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )
    .map_err(|refused| refused.error())
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
///
/// # Errors
///
/// `header_value_invalid` for a header value, its `content_type`'s
/// included, holding a CR, an LF or a NUL.
fn fulfillment(params: RouteFulfillParams) -> Result<Fulfillment, CallError> {
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
    headers_sent(Some(&headers))?;
    Ok(Fulfillment {
        status: params.status.unwrap_or(200),
        headers,
        body: params.body.unwrap_or_default(),
    })
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
                            &context,
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
                            &context,
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
                            &context,
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
                            &context,
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
    /// Each open page's events, which calls reach as they arrive.
    events: page_events::Table,
    /// Each open page's dialogs and their handlers.
    dialogs: Dialogs,
    /// The Product's hook, which also serves the pages its fetch tiers
    /// opened from a document.
    hooks: Arc<dyn HandlerPages>,
    /// The Product's Block classification.
    blocks: Arc<dyn BlockClassifier>,
    /// Each page's init scripts on their way in or out, which its
    /// navigations wait for.
    init_scripts: InitScripts,
    /// The Sessions Provenance marks as injecting in the main world.
    injections: Injections,
}

impl Pages {
    /// Navigates the page `page_id` to `destination` for the call
    /// `context`, answering once its document reached the state
    /// `wait_until` names; a download it turns into is published on the
    /// call's events. A URL is judged by its scheme first (ADRs 0017, 0056
    /// and 0089): a `file:` URL, or any other scheme but HTTP's and
    /// `about:blank`, is loaded only for the program's own code or the CLI
    /// on an engine without accounts, refused with `egress_refused`
    /// otherwise, and a `javascript:` URL runs only in a main frame whose
    /// document's origin the domain policy allows.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %page_id.0, operation = destination.operation(), run_id = tracing::field::Empty, url.origin = tracing::field::Empty))]
    async fn navigate(
        &self,
        page_id: &PageId,
        destination: Destination<'_>,
        wait_until: WaitUntil,
        context: &CallContext,
    ) -> Result<Navigated, CallError> {
        let span = tracing::Span::current();
        if let Destination::Url(url) = destination {
            span.record("url.origin", goodfetch_engine::log::origin(url));
        }
        self.init_scripts.settled(page_id).await;
        let page = self.open(page_id).await?;
        span.record("run_id", page.run.id().as_str());
        let navigator = Navigator {
            acting_for_model: context.surface().acts_for_model(),
            recipe_run: page
                .run
                .settings()
                .is_some_and(|settings| settings.resolved().recipe_run()),
            // No engine serves accounts before area 12's: a person's own
            // engine serves the program's own code alone (`runs::CALLER`).
            accounts: false,
        };
        if let Destination::Url(url) = destination
            && egress::navigation(url, navigator).map_err(|refused| refused.error())?
                == egress::Destination::JavaScript
        {
            let document = page.driver.document_url().await?;
            self.interception
                .judge_javascript(page_id, url, &document)?;
        }
        let _files = navigator
            .reads_files()
            .then(|| self.interception.allow_files(page_id, navigator));
        let events = context.events();
        let blocked_before = self.interception.lists_blocked(page_id);
        let navigated = page
            .life
            .call(
                PageEvent::Goto,
                page.driver.navigate(destination, wait_until),
            )
            .await;
        // A failure the interception left while the navigation ran, such
        // as a route's handler failing on its document, is the
        // navigation's.
        if let Some(failure) = self.left_failure(page_id) {
            return Err(failure);
        }
        let Some(response) = arrival::arrived(page_id, navigated?, events).await else {
            return Ok(Navigated { response: None });
        };
        // A page a program opened records what its document's robots.txt
        // says, beside the navigation, never before it (ADR 0086).
        if page.driver.value().gate().is_none() {
            robots_txt::read(
                &self.sessions,
                self.interception.engine_requests(page_id)?,
                page_id,
                &page,
                &response.url,
            );
        }
        // Every document a Session's navigation reached is judged: its Block
        // cause first, which raises `blocked` (ADRs 0088 and 0092), unless
        // it checks the browser and the Filter lists touched its
        // navigation, which loads it once more with blocking off (ADR 0047);
        // then its Page check (ADRs 0006 and 0092).
        let page_events = self.events.get(page_id)?;
        page_events.judged(&response.request_id);
        let received = page_events
            .answered_at(&response.request_id)
            .unwrap_or_else(|| page.clock.wall());
        let operation = Some(destination.operation());
        let block =
            blocked::block_of(&page.driver, self.blocks.as_ref(), &response, operation).await?;
        let touched = self.interception.lists_blocked(page_id) > blocked_before;
        if let Some(block) = block
            && !(touched && block.checks_the_browser())
        {
            return Err(blocked::blocked(block, &response, received));
        }
        let judged =
            recovery::judge(&arrival::judging(&page, page_id, destination), &response).await;
        let first = blocking_off::FirstLoad {
            destination,
            wait_until,
            response,
            judged,
            block,
            touched,
        };
        let navigated = self
            .unless_blocking_broke_it(page_id, &page, first, events)
            .await?;
        // A consent wall the runner found on the navigation's document is
        // the navigation's, once the runner judged it (ADR 0048).
        self.consent_verdict(page_id, &page, true).await?;
        Ok(navigated)
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
        let acted = page
            .life
            .call(
                PageEvent::Gesture,
                guard::act(&page.driver, request, &page.clock),
            )
            .await;
        // A dialog handler that failed on a dialog the action opened fails
        // the action, which the dialog blocked.
        if let Some(failure) = self.dialogs.take_failure(&locator.page_id) {
            return Err(failure);
        }
        acted?;
        Ok(Acted {})
    }

    /// Sends the request `params` names from its page's Session, through
    /// the browser-fetch tier, from a page of the Session the engine opens
    /// for it and closes after it, and answers its response, whose body the
    /// page's events keep, whole or as cut, for `response.body`; its errors
    /// are spelled for `surface`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_REQUEST.name, run_id = tracing::field::Empty, url.origin = goodfetch_engine::log::origin(&params.url)))]
    async fn request(
        &self,
        params: PageRequestParams,
        surface: Surface,
    ) -> Result<PageResponse, CallError> {
        let max_redirects = redirect_count(params.max_redirects, surface)?;
        headers_sent(params.headers.as_ref())?;
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let steps = self.interception.engine_requests(&params.page_id)?;
        let events = self.events.get(&params.page_id)?;
        let limits = page
            .run
            .settings()
            .map_or_else(Limits::default, |settings| Limits::of(settings.resolved()));
        let request = browser_fetch::Request {
            url: &params.url,
            method: params.method.as_deref(),
            headers: params.headers.as_ref(),
            post_data: params.post_data.as_deref(),
            max_redirects,
            option: max_redirects_option(surface),
            limits,
        };
        let (driver, in_browser) = self
            .sessions
            .engine_page(&page.session, page.driver.value())
            .await?;
        let fetched = browser_fetch::send(&driver, &steps, &request).await;
        driver.close().await;
        drop(in_browser);
        let fetched = fetched?;
        let kept = match fetched.body {
            browser_fetch::Body::Whole(bytes) => {
                let handle = page.run.store_body(bytes).await.map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("the response's body was not kept in the Run's folder: {error}"),
                    )
                })?;
                Requested::Whole(handle)
            }
            browser_fetch::Body::Cut { received, expected } => Requested::Cut {
                url: fetched.response.url.clone(),
                received,
                expected,
            },
        };
        events.keep_requested(fetched.response.request_id.clone(), kept);
        Ok(fetched.response)
    }

    /// Runs the page function `params` names on its page, in the world it
    /// names, and answers its JSON; a main-world call marks the Session in
    /// Provenance. Its errors are spelled for `surface`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_EVALUATE.name, run_id = tracing::field::Empty))]
    async fn evaluate(
        &self,
        params: EvaluateParams,
        surface: Surface,
    ) -> Result<Evaluated, CallError> {
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let world = params.world.unwrap_or_default();
        if world == World::Main {
            self.injections
                .mark(
                    &page.run,
                    &page.session,
                    &params.page_id,
                    Injection::Evaluate,
                )
                .await;
        }
        let expression = page_functions::expression(&params.function, params.arg.as_ref());
        let returns = page
            .life
            .call(
                PageEvent::Read,
                page.driver.evaluate(&expression, world, PAGE_EVALUATE.name),
            )
            .await?;
        page_functions::answered(returns, surface)
    }

    /// Registers the init script `params` names on its page, `pending`
    /// until it is, and marks the Session in Provenance; answers once the
    /// page ended, and removes the script when the call is dropped before.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_ADD_INIT_SCRIPT.name, run_id = tracing::field::Empty))]
    async fn add_init_script(
        &self,
        params: AddInitScriptParams,
        pending: Pending,
    ) -> Result<InitScriptEnded, CallError> {
        let source = page_functions::init_script(&params.function, params.arg.as_ref())?;
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let identifier = page
            .life
            .call(
                PageEvent::Read,
                page.driver
                    .add_init_script(source, PAGE_ADD_INIT_SCRIPT.name),
            )
            .await?;
        let registered = Registered::new(
            page.driver.clone(),
            identifier,
            page.run.clone(),
            self.init_scripts.clone(),
            params.page_id.clone(),
        );
        self.injections
            .mark(
                &page.run,
                &page.session,
                &params.page_id,
                Injection::InitScript,
            )
            .await;
        drop(pending);
        let life = page.life.clone();
        drop(page);
        life.terminal().await;
        registered.ended();
        Ok(InitScriptEnded {})
    }

    /// The open page `page_id`, or `closed`; a failure a route's or a
    /// dialog handler left on it is raised here, by the page's next call,
    /// and so is the Block cause of a document a navigation no call waited
    /// for reached (ADR 0092).
    async fn open(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        let page = self.sessions.page(page_id).await?;
        if let Some(failure) = self.left_failure(page_id) {
            return Err(failure);
        }
        self.consent_verdict(page_id, &page, false).await?;
        self.judge_unjudged(&page, page_id, None).await?;
        Ok(page)
    }

    /// Raises the consent wall the Consent prompt runner left in the main
    /// document of `page`, the page `page_id`, once, after waiting, within
    /// the page's action deadline, for the verdict on a prompt the runner's
    /// first pass there found (ADR 0048): a call that `navigated` asks the
    /// runner first whether it found one, as its report may still be on its
    /// way.
    async fn consent_verdict(
        &self,
        page_id: &PageId,
        page: &OpenPage,
        navigated: bool,
    ) -> Result<(), CallError> {
        let walls = self.sessions.consent_walls();
        if navigated && crate::consent::first_pass_found(&page.driver).await {
            walls.detected(page_id);
        }
        let (_, limit) = &page.action_timeout;
        if !walls.verdict(page_id, *limit, page.clock).await {
            tracing::debug!(page_id = %page_id.0, "the Consent prompt runner gave no verdict on a prompt it found within the action deadline");
        }
        self.consent_wall(page_id)
    }

    /// Raises the consent wall the Consent prompt runner left in the
    /// document the page `page_id` holds, once (ADR 0048).
    fn consent_wall(&self, page_id: &PageId) -> Result<(), CallError> {
        let walled = self.events.get(page_id).ok().and_then(|events| {
            blocked::consent_wall(self.sessions.consent_walls(), page_id, &events)
        });
        walled.map_or(Ok(()), Err)
    }

    /// Judges the Block cause of the document the main frame of `page`,
    /// the page `page_id`, committed last, unless a call judged it already:
    /// for the call that waited for its navigation, the operation
    /// `waiting`, whose code a failure to read the document raises; for the
    /// page's next call, `None`, when such a failure reads as no block.
    async fn judge_unjudged(
        &self,
        page: &OpenPage,
        page_id: &PageId,
        waiting: Option<&'static str>,
    ) -> Result<(), CallError> {
        let Some((response, received)) = self
            .events
            .get(page_id)
            .ok()
            .and_then(|events| events.take_unjudged())
        else {
            return Ok(());
        };
        match blocked::block_of(&page.driver, self.blocks.as_ref(), &response, waiting).await? {
            Some(block) => Err(blocked::blocked(block, &response, received)),
            None => Ok(()),
        }
    }

    /// Judges, for `page.wait_for_url`, which waited for the navigation of
    /// the page `page_id` that reached its URL, the Block cause of the
    /// document it reached (ADR 0092).
    async fn judge_waited(
        &self,
        page_id: &PageId,
        operation: &'static str,
    ) -> Result<(), CallError> {
        let page = self.sessions.page(page_id).await?;
        self.judge_unjudged(&page, page_id, Some(operation)).await
    }

    /// The failure a route's handler, or else a dialog handler, left on the
    /// page `page_id` for the call it blocked or its next call.
    fn left_failure(&self, page_id: &PageId) -> Option<CallError> {
        self.interception
            .take_failure(page_id)
            .or_else(|| self.dialogs.take_failure(page_id))
    }

    /// The formats the request names, read from one serialization of the
    /// page's document, its shadow roots included, on a blocking thread of
    /// its Run's, once extraction-side removal took what its Session's
    /// cosmetic rules match (ADR 0047), which Provenance records.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_FORMATS.name, run_id = tracing::field::Empty))]
    async fn formats(&self, params: FormatsParams) -> Result<PageFormats, CallError> {
        if let Some(served) = self.hooks.document(&params.page_id) {
            tracing::Span::current().record("run_id", served.run.id().as_str());
            let document = SerializedDocument {
                html: served.html,
                url: served.url,
                base_url: served.base_url,
            };
            let request = params.formats;
            let (formats, _) = served
                .run
                .owner()
                .blocking(move || clean_up::read(&document, request, None))
                .await;
            return Ok(formats);
        }
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let document = page
            .life
            .call(PageEvent::Read, page.driver.document())
            .await?;
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
