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
//! its robots.txt says of each document and of each `page.request`'s
//! response, never gating it (the Sessions' `robots_txt`, ADR 0086).
//!
//! Size: one table of the page API's handlers over the engine's shared
//! pages, each call opening its page through one `open` that raises what
//! the page left for it, a Block cause among them, kept whole so every call
//! meets its page the same way.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::{ParsedHtml, Row};
use goodfetch_engine::browser_cache::PinnedArtifact;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::egress::{self, Navigator};
use goodfetch_engine::hostile_input::check_headers;
use goodfetch_engine::ledger::Refusal as LedgerRefusal;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::run::{limit_not_understood, limit_reached};
use goodfetch_engine::runs::{ChargeError, Run};
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::{CallContext, CallError, Contribution, Enclose, Guard, Handler, named};
use goodfetch_extractor::{BuiltIn, Serialization};
use goodfetch_protocol::connection::{Form, WaitedFor};
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
    AddInitScriptParams, EvaluateParams, Evaluated, InitScriptEnded, LOCATOR_EVALUATE_ALL,
    LocatorEvaluateAllParams, PAGE_ADD_INIT_SCRIPT, PAGE_EVALUATE, World,
};
use goodfetch_protocol::page::read::{PAGE_READ, PageRead, PageReadParams};
use goodfetch_protocol::page::request::{MAX_REDIRECTS, PAGE_REQUEST, PageRequestParams};
use goodfetch_protocol::page::route::UrlMatcher;
use goodfetch_protocol::page::route::{
    PAGE_ROUTE, PageRouteParams, ROUTE_ABORT, ROUTE_CONTINUE, ROUTE_FALLBACK, ROUTE_FETCH,
    ROUTE_FULFILL, RouteAbortParams, RouteAnswered, RouteFetched, RouteFulfillParams,
    RouteOverridesParams, SESSION_ROUTE, SessionRouteParams,
};
use goodfetch_protocol::page::session::{
    NewSessionPageParams, NewSessionParams, RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE,
    SessionClosed, SessionParams,
};
use goodfetch_protocol::page::session_pages::{
    self, PageCloseParams, PageClosed, SESSION_ON, SESSION_PAGES, SessionEventName, SessionOnParams,
};
use goodfetch_protocol::page::{
    Acted, ClickParams, FillParams, FormatsParams, FormatsRequest, GotoParams, LOCATOR_CLICK,
    LOCATOR_FILL, LOCATOR_PRESS, Locator, Navigated, NewPageParams, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageFormats, PageId, PageParams, PageResponse,
    PressParams, RUN_NEW_PAGE, SessionId, WaitUntil,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};
use goodfetch_protocol::usage::REQUESTS;
use serde_json::Value;

use crate::blocks::{Block, BlockClassifier, NoBlocks};
use crate::browser_fetch;
use crate::dialogs::{self, Dialogs};
use crate::document_request::{DocumentRequest, Unread};
use crate::driver::{Destination, SerializedDocument};
use crate::extractor;
use crate::fields;
use crate::guard::{self, Action, Request, SentMark, Window};
use crate::hiding::Hidden;
use crate::interception::{
    Answer, ConsentHandling, Fulfillment, Interception, Overrides, Reply, Scope,
};
use crate::launch::Launcher;
use crate::page_events::{self, Requested};
use crate::page_functions::{self, InitScripts, Injection, Matches, Pending, Registered};
use crate::page_value::{
    BrowserPages, EngineFetch, EngineFetched, FetchedBody, HandlerPages, HeldPlace, Opening,
    OwnFetch, PageValue, PooledSession, ProgramPages, ServedFormatsAhead,
};
use crate::proxy;
use crate::recovery;
use crate::sessions::{self, EngineWork, OpenPage, PageEvent, Sessions};
use crate::trace;
use crate::url_pattern::UrlPattern;

mod arrival;
mod blocked;
mod blocking_off;
mod clean_up;
pub(crate) mod served_formats;
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
    contribution_with(
        (pins, Launcher::Executable),
        ProgramPages,
        Arc::new(NoBlocks),
    )
}

/// What the page crate adds to the engine, as [`contribution`], each page
/// it opens taking its value and its response observer from
/// `handler_pages` (ADR 0087): the seam through which Foxhound sets them
/// on the pages it hands to handlers; `blocks` tells a page's Block cause,
/// from the Product's rules (ADR 0035); `launcher` starts each Run's
/// browsers (ADR 0059), the Browser cache's build but in a benchmark or a
/// test.
pub fn contribution_with(
    (pins, launcher): (Vec<PinnedArtifact>, Launcher),
    handler_pages: impl HandlerPages + 'static,
    blocks: Arc<dyn BlockClassifier>,
) -> Contribution {
    let handler_pages: Arc<dyn HandlerPages> = Arc::new(handler_pages);
    let interception = Interception::default();
    let events = page_events::Table::default();
    let dialogs = Dialogs::default();
    let pages = Pages {
        sessions: Sessions::new(
            (pins, launcher),
            interception.clone(),
            events.clone(),
            dialogs.clone(),
            Arc::clone(&handler_pages),
        ),
        interception,
        events,
        dialogs,
        product: handler_pages,
        blocks,
        init_scripts: InitScripts::default(),
    };
    let contribution = with_page_handlers(
        Contribution::new("page")
            .with_run_of_call(Arc::new(pages.sessions.clone()))
            .with_run_state_tables(sessions::TABLES)
            .with_politeness_tables(proxy::TABLES)
            .with_clean_up_versions(pages.interception.versions()),
        &pages,
    );
    let contribution = with_page_read_handlers(contribution, &pages);
    let contribution = with_session_handlers(contribution, &pages.sessions);
    let contribution = with_locator_handlers(contribution, &pages);
    let contribution = with_event_handlers(contribution, &pages);
    let contribution = with_route_handlers(contribution, &pages.interception);
    let contribution = with_dialog_handlers(contribution, &pages.dialogs);
    let contribution = with_function_handlers(contribution, &pages);
    let contribution = waits::with_wait_handlers(contribution, &pages);
    let contribution = with_request_handler(contribution, &pages);
    pages.product.opened(Arc::new(Opener {
        pages: pages.clone(),
    }));
    // A call on a page a fetch tier serves from a document is the
    // Product's to answer, serve or refuse (ADR 0093).
    let product = Arc::clone(&pages.product);
    let guard: Guard = Arc::new(move |operation, params, read, context| {
        let page_id = PageId(params.page()?.into_owned());
        product.call(&page_id, operation.name, read, context)
    });
    let contribution = contribution.guarded(&guard);
    if pages.product.encloses_calls() {
        contribution.enclosed(&enclosing_handler(&pages))
    } else {
        contribution
    }
}

/// What ends a call on a handler's page at its handler's end (ADR 0080):
/// the enclosing call a served page's Product names, else the one a browser
/// page's lease names; a call on a page no call encloses, as on every page
/// a program opens, keeps its answer. `observed` says whether a navigation
/// was under way on the page, or what document a served page held. Only a
/// Product whose pages a call may enclose is given it.
fn enclosing_handler(pages: &Pages) -> Enclose {
    let found_in = Arc::new((pages.sessions.clone(), Arc::clone(&pages.product)));
    Enclose::new(move |_, params, context, answer| {
        let Some(page_id) = params.page() else {
            return answer;
        };
        let page_id = PageId(page_id.into_owned());
        let (found_in, clock) = (Arc::clone(&found_in), context.engine().clock());
        Box::pin(async move {
            let (sessions, product) = &*found_in;
            if let Some(enclosing) = product.enclosing(&page_id) {
                let observe = || {
                    product.document(&page_id).map(|document| {
                        format!("page `{}` served from `{}`", page_id.0, document.url)
                    })
                };
                return enclosing.bound(&clock, answer, observe).await;
            }
            let Ok(page) = sessions.page(&page_id).await else {
                return answer.await;
            };
            // Past its lease, the page refuses the call itself with
            // `attempt_ended` (ADRs 0003 and 0087), which the handler's
            // end, passed before the lease's, would answer first with
            // `deadline_exceeded`.
            let Some(enclosing) = page
                .driver
                .value()
                .lease()
                .filter(|lease| lease.check().is_ok())
                .and_then(|lease| lease.enclosing())
            else {
                return answer.await;
            };
            let observe = || {
                Some(if page.driver.navigation_waits() {
                    format!("a navigation under way on page `{}`", page_id.0)
                } else {
                    format!("no navigation under way on page `{}`", page_id.0)
                })
            };
            enclosing.bound(&clock, answer, observe).await
        })
    })
}

/// Adds the handlers of `page.evaluate`, of `locator.evaluate_all` and of
/// `page.add_init_script`, whose script is counted on its way in when its
/// call arrives, so that a navigation sent after it waits for it.
fn with_function_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (evaluating, evaluating_all, registering) = (pages.clone(), pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_EVALUATE,
            move |params: EvaluateParams, context: CallContext| {
                let pages = evaluating.clone();
                async move {
                    let expression =
                        page_functions::expression(&params.function, params.arg.as_ref());
                    pages
                        .evaluate(
                            &params.page_id,
                            (&expression, params.world),
                            (Injection::Evaluate, PAGE_EVALUATE.name),
                            context.form(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_EVALUATE_ALL,
            move |params: LocatorEvaluateAllParams, context: CallContext| {
                let pages = evaluating_all.clone();
                async move {
                    let expression = page_functions::on_locator(
                        &params.function,
                        params.arg.as_ref(),
                        (&params.locator.selector, Matches::All),
                    );
                    pages
                        .evaluate(
                            &params.locator.page_id,
                            (&expression, params.world),
                            (Injection::EvaluateAll, LOCATOR_EVALUATE_ALL.name),
                            context.form(),
                        )
                        .await
                }
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
        move |params: PageRequestParams, _: CallContext| {
            let pages = requesting.clone();
            async move { Box::pin(pages.request(params)).await }
        },
    ))
}

/// The count of redirects `page.request` follows, `max_redirects` or its
/// default; `setting_invalid` above the browser-fetch tier's count, its
/// parameter spelled in its Client's form (ADRs 0099 and 0118).
fn redirect_count(max_redirects: Option<u32>) -> Result<u32, CallError> {
    let count = max_redirects.unwrap_or(MAX_REDIRECTS);
    if count <= MAX_REDIRECTS {
        return Ok(count);
    }
    Err(CallError::new(
        &SETTING_INVALID,
        format!(
            "`{}` is {count}, above {MAX_REDIRECTS}: the browser-fetch tier that sends `page.request` follows at most {MAX_REDIRECTS} redirects",
            named::parameter(MAX_REDIRECTS_PARAMETER)
        ),
        &SettingInvalidDetails {
            key: MAX_REDIRECTS_PARAMETER.to_owned(),
            layer: Some(SettingsLayer::Run),
            reason: format!(
                "{count} is above {MAX_REDIRECTS}, the most redirects a request follows"
            ),
            limit: None,
            fix: None,
        },
    ))
}

/// `page.request`'s parameter that counts its redirects, by its canonical
/// name.
const MAX_REDIRECTS_PARAMETER: &str = "max_redirects";

/// Adds the handlers of `page.on`, each listening from when its call
/// arrives until the call goes and sending its events on the call itself,
/// of `page.wait_for_request_finished`, armed when its call arrives, and
/// of `response.body`.
fn with_event_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let table = &pages.events;
    let (listening, waiting, reading) = (table.clone(), table.clone(), table.clone());
    let (finishing, reading_pages) = (pages.clone(), pages.clone());
    contribution
        .with_handler(
            Handler::new(
                &PAGE_ON,
                move |params: PageOnParams, context: CallContext| {
                    let listened = listening
                        .get(&params.page_id)
                        .map(|events| events.listen(params.event));
                    async move {
                        listened?.serve(context.stream()).await?;
                        Ok::<PageOnEnded, CallError>(PageOnEnded {})
                    }
                },
            )
            .streaming(),
        )
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_REQUEST_FINISHED,
            move |params: WaitForRequestFinishedParams, context: CallContext| {
                let armed = waiting.get(&params.page_id).and_then(|events| {
                    let pattern = params.url.as_ref().map(pattern_of).transpose()?;
                    let deadline = events.wait_deadline(params.timeout)?;
                    Ok(events.wait_for_request_finished(pattern, context.form(), deadline))
                });
                let pages = finishing.clone();
                in_call(
                    (PAGE_WAIT_FOR_REQUEST_FINISHED.name, context.form()),
                    async move { pages.waited(&params.page_id, armed?).await },
                )
            },
        ))
        .with_handler(Handler::new(
            &RESPONSE_BODY,
            move |params: ResponseBodyParams, context: CallContext| {
                let read = reading.get(&params.page_id).and_then(|events| {
                    let deadline = events.wait_deadline(params.timeout)?;
                    Ok((events, deadline))
                });
                let (form, pages) = (context.form(), reading_pages.clone());
                // A body the engine kept reads from the Run's folder, even on
                // a crashed page (ADR 0085); any other read is a call on the
                // page, as its waits are: a page that ends under it raises
                // what ended it.
                async move {
                    let (events, deadline) = read?;
                    if let Some(kept) = events.kept(&params.request_id) {
                        return kept;
                    }
                    let body = Box::pin(events.body(&params.request_id, form, deadline));
                    pages.waited(&params.page_id, body).await
                }
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
                // Counted when the call arrives, so no page opened meanwhile
                // is missed.
                let listener = match params.event {
                    SessionEventName::Page => sessions.listen_for_pages(&params.session_id),
                };
                async move {
                    sessions
                        .listen_to_pages(&params.session_id, listener, &context)
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &session_pages::PAGE_CLOSE,
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
            move |params: NewSessionPageParams, context: CallContext| {
                let sessions = paging.clone();
                async move {
                    Box::pin(sessions.new_page(
                        &params.session_id,
                        &params.settings,
                        &HeldPlace::default(),
                        &context,
                    ))
                    .await
                }
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

/// Adds the handlers of `run.new_page` and the navigations.
fn with_page_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (opening, going, reloading, going_back, going_forward) = (
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
                // Boxed: a page's opening, its browser's launch included, is
                // past clippy's large-future size on Windows.
                async move {
                    Box::pin(pages.sessions.new_default_page(
                        &params.run_id,
                        &params.settings,
                        &context,
                    ))
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
                            (Destination::Url(&params.url), Navigating::Program),
                            params.wait_until.unwrap_or_default(),
                            params.timeout,
                            Charge::OneRequest,
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
                            (Destination::Reload, Navigating::Program),
                            WaitUntil::Load,
                            None,
                            Charge::Nothing,
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
                            (Destination::Back, Navigating::Program),
                            WaitUntil::Load,
                            None,
                            Charge::Nothing,
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
                            (Destination::Forward, Navigating::Program),
                            WaitUntil::Load,
                            None,
                            Charge::Nothing,
                            &context,
                        )
                        .await
                }
            },
        ))
}

/// Adds the handlers of the page's reads, `page.formats` and `page.read`.
fn with_page_read_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (formatting, reading) = (pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_FORMATS,
            move |params: FormatsParams, _context: CallContext| {
                let pages = formatting.clone();
                async move { pages.formats(params).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_READ,
            move |params: PageReadParams, _context: CallContext| {
                let pages = reading.clone();
                async move { pages.read_fields(params).await }
            },
        ))
}

/// Answers `call`, a call of the operation named first, from a Client
/// writing names in the form named second: its failure headed by the
/// operation in that form, with `operation` its canonical name (ADRs 0079
/// and 0099).
async fn in_call<T>(
    (operation, form): (&'static str, Form),
    call: impl Future<Output = Result<T, CallError>>,
) -> Result<T, CallError> {
    call.await.map_err(|error| error.in_call(operation, form))
}

/// Adds the handlers of the locator actions.
fn with_locator_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (clicking, filling, pressing) = (pages.clone(), pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &LOCATOR_CLICK,
            move |params: ClickParams, context: CallContext| {
                let pages = clicking.clone();
                in_call((LOCATOR_CLICK.name, context.form()), async move {
                    pages
                        .act(
                            LOCATOR_CLICK.name,
                            params.locator,
                            Ok(Action::Click),
                            &context,
                        )
                        .await
                })
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_FILL,
            move |params: FillParams, context: CallContext| {
                let pages = filling.clone();
                in_call((LOCATOR_FILL.name, context.form()), async move {
                    pages
                        .act(
                            LOCATOR_FILL.name,
                            params.locator,
                            Ok(Action::Fill(params.value)),
                            &context,
                        )
                        .await
                })
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_PRESS,
            move |params: PressParams, context: CallContext| {
                let pages = pressing.clone();
                in_call((LOCATOR_PRESS.name, context.form()), async move {
                    let action = Action::press(&params.key);
                    pages
                        .act(LOCATOR_PRESS.name, params.locator, action, &context)
                        .await
                })
            },
        ))
}

/// The browser pages the Product opens for its handlers itself: each in the
/// Session of its Session pool that serves the handler, navigated as
/// `page.goto` navigates.
struct Opener {
    pages: Pages,
}

/// Whose navigation [`Pages::navigate`] sends, which says whether it opens
/// ADR 0106's send window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Navigating {
    /// A program's command, which may change a site: it marks the window's
    /// bodies sent before it goes out.
    Program,
    /// A crawl request's own document, a read whatever its method (ADR
    /// 0104's amendment): it marks nothing.
    OwnDocument,
}

/// A page [`Opener`] opened whose navigation has not answered yet: dropped
/// so, as when its opening is cancelled, it closes on a task of its Run,
/// its navigation with it.
struct ClosingOnDrop {
    sessions: Sessions,
    run: Run,
    page_id: Option<PageId>,
}

impl ClosingOnDrop {
    /// Keeps the page open: its navigation answered.
    fn disarm(mut self) {
        self.page_id = None;
    }
}

impl Drop for ClosingOnDrop {
    fn drop(&mut self) {
        let Some(page_id) = self.page_id.take() else {
            return;
        };
        let sessions = self.sessions.clone();
        self.run.spawn(async move {
            if let Err(error) = sessions.close_page(&page_id, false).await {
                tracing::debug!(%error, page_id = %page_id.0, "a cancelled handler's browser page was closed already");
            }
        });
    }
}

impl BrowserPages for Opener {
    fn open<'a>(
        &'a self,
        run_id: &'a RunId,
        pooled: &'a PooledSession,
        own: OwnFetch<'a>,
        context: &'a CallContext,
    ) -> Opening<'a, Result<(PageId, Navigated), CallError>> {
        Box::pin(async move {
            // Boxed: opening holds a live Run's lookup and a browser
            // context's opening (clippy's `large_futures`, reached on
            // Windows too).
            let opened =
                Box::pin(self.pages.sessions.new_pool_page(run_id, pooled, context)).await?;
            let page_id = opened.page_id;
            let page = self.pages.sessions.page(&page_id).await?;
            // The document its navigation reaches is its crawl request's
            // own, which its document reader reads as such (ADR 0119).
            if let Some(reader) = page.driver.reader() {
                reader.expect_own();
            }
            // Dropped before its navigation answered, the page closes.
            let closing = ClosingOnDrop {
                sessions: self.pages.sessions.clone(),
                run: page.run,
                page_id: Some(page_id.clone()),
            };
            // Its document is fetched with the crawl request's method and
            // body for as long as the navigation goes on, its request charged
            // at its admission already.
            let sent = self.pages.interception.send_own(&page_id, own);
            let navigated = self
                .pages
                .navigate(
                    &page_id,
                    (Destination::Url(own.url), Navigating::OwnDocument),
                    WaitUntil::Load,
                    None,
                    Charge::Nothing,
                    context,
                )
                .await;
            drop(sent);
            closing.disarm();
            match navigated {
                Ok(navigated) => Ok((page_id, navigated)),
                Err(error) => {
                    self.close(&page_id).await;
                    Err(error)
                }
            }
        })
    }

    fn close<'a>(&'a self, page_id: &'a PageId) -> Opening<'a, ()> {
        Box::pin(async move {
            if let Err(error) = self.pages.sessions.close_page(page_id, false).await {
                tracing::debug!(%error, page_id = %page_id.0, "a handler's browser page was closed already");
            }
        })
    }

    fn document_request<'a>(
        &'a self,
        run: &'a Run,
    ) -> Opening<'a, Result<Option<DocumentRequest>, Unread>> {
        Box::pin(self.pages.sessions.document_request(run))
    }

    fn retire<'a>(
        &'a self,
        run_id: &'a RunId,
        pooled: &'a SessionId,
        context: &'a CallContext,
    ) -> Opening<'a, Result<(), CallError>> {
        Box::pin(async move { self.pages.sessions.retire(run_id, pooled, context).await })
    }

    fn certificate_exception<'a>(
        &'a self,
        page_id: &'a PageId,
        host: &'a str,
    ) -> Opening<'a, Option<goodfetch_engine::tls::ExceptionUsed>> {
        Box::pin(async move {
            let page = self.pages.sessions.page(page_id).await.ok()?;
            page.driver.relay()?.exception(host)
        })
    }

    fn fetch<'a>(
        &'a self,
        run_id: &'a RunId,
        pooled: &'a PooledSession,
        value: PageValue,
        request: &'a EngineFetch,
        context: &'a CallContext,
    ) -> Opening<'a, Result<EngineFetched, CallError>> {
        Box::pin(async move {
            let session = self
                .pages
                .sessions
                .pool_session(run_id, pooled, context)
                .await?;
            let (run, own) = self.pages.sessions.own_settings(&session).await?;
            let steps = self
                .pages
                .interception
                .engine_requests_of(&session, &run, &own)
                .await?;
            let limits = Limits::of_run(run.settings());
            let headers: BTreeMap<String, String> = request.headers.iter().cloned().collect();
            let post_data = request
                .body
                .as_deref()
                .map(|body| data_encoding::BASE64.encode(body));
            let sent = browser_fetch::Request {
                url: &request.url,
                method: (request.method != "GET").then_some(request.method.as_str()),
                headers: (!headers.is_empty()).then_some(&headers),
                post_data: post_data.as_deref(),
                max_redirects: 0,
                option: "fetch.redirect.max",
                limits,
                read: browser_fetch::BodyRead::Whole,
            };
            // A cancelled fetch closes the engine's page as it drops.
            let engine_page = self
                .pages
                .sessions
                .engine_page(&session, &value, EngineWork::Request)
                .await?;
            let fetched = browser_fetch::send(engine_page.driver(), &steps, &sent).await;
            engine_page.close().await;
            let fetched = fetched?;
            let headers = fetched
                .response
                .headers
                .iter()
                .flat_map(|(name, values)| {
                    values
                        .split('\n')
                        .map(move |value| (name.clone(), value.to_owned()))
                })
                .collect();
            Ok(EngineFetched {
                url: fetched.response.url,
                status: fetched.response.status,
                headers,
                body: match fetched.body {
                    browser_fetch::Body::Whole(bytes) => FetchedBody::Whole(bytes),
                    browser_fetch::Body::Cut { received, expected } => {
                        FetchedBody::Cut { received, expected }
                    }
                },
            })
        })
    }

    fn read<'a>(
        &'a self,
        page_id: &'a PageId,
        function: &'static str,
        mut argument: Value,
        operation: &'static str,
    ) -> Opening<'a, Result<Option<Value>, CallError>> {
        Box::pin(async move {
            let page = self.pages.sessions.page(page_id).await?;
            let cosmetic = match &page.hiding {
                Some(hiding) => hiding.applied().await,
                None => Vec::new(),
            };
            if let Some(fields) = argument.as_object_mut() {
                fields.insert("cosmetic".to_owned(), cosmetic.into());
            }
            page.life
                .call(
                    PageEvent::Read,
                    page.driver.read_in_page(function, argument, operation),
                )
                .await
        })
    }

    fn formats<'a>(
        &'a self,
        page_id: &'a PageId,
        request: FormatsRequest,
    ) -> Opening<'a, Result<PageFormats, CallError>> {
        Box::pin(self.pages.formats(FormatsParams {
            page_id: page_id.clone(),
            formats: request,
        }))
    }

    fn served_formats<'a>(
        &'a self,
        run: &'a Run,
        request: FormatsRequest,
        label: Option<BuiltIn>,
    ) -> Opening<'a, Option<Arc<dyn ServedFormatsAhead>>> {
        Box::pin(async move {
            let cosmetic = self.pages.interception.served_cosmetic(run).await;
            let reader = served_formats::ServedFormatsReader::of(run, request, label, cosmetic);
            Some(Arc::new(reader) as Arc<dyn ServedFormatsAhead>)
        })
    }

    fn prepare_served_formats(&self, run: &Run) {
        let (pages, preparing) = (self.pages.clone(), run.clone());
        run.spawn(async move {
            drop(pages.interception.served_cosmetic(&preparing).await);
        });
    }

    fn classify(&self, response: &PageResponse, markup: &str) -> Option<Block> {
        self.pages.blocks.classify(response, markup)
    }
}

/// What a navigation charges its page's Run before it is sent (issue 3416).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Charge {
    /// One `requests` Usage for a URL the network serves: a program's
    /// `page.goto`, charged as a crawl's request is.
    OneRequest,
    /// Nothing: a crawl's navigation, whose request its admission charged,
    /// or a reload or a move in the page's history.
    Nothing,
}

/// Charges `run` the one `requests` Usage of a navigation: a Limit with
/// too little left refuses it `limit_reached` and stops the Run at its next
/// safe point, as a crawl's request is refused.
async fn charge_request(run: &Run, context: &CallContext) -> Result<(), CallError> {
    run.charge(REQUESTS, 1).await.map_err(|error| match error {
        ChargeError::Refused(LedgerRefusal::Reached(reached)) => limit_reached(&reached),
        ChargeError::Refused(LedgerRefusal::NotUnderstood(limit)) => limit_not_understood(*limit),
        error => sessions::unavailable(run, &error, context),
    })
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
    /// The Product's seam that sets what its handlers' pages carry, which
    /// also serves the pages its fetch tiers opened from a document.
    product: Arc<dyn HandlerPages>,
    /// The Product's Block classification.
    blocks: Arc<dyn BlockClassifier>,
    /// Each page's init scripts on their way in or out, which its
    /// navigations wait for.
    init_scripts: InitScripts,
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
    /// document's origin the domain policy allows. The navigation waits
    /// under the call's own `timeout` when it gives one, else under the
    /// Run's `timeout.navigation` (ADR 0080). Under `charge`, a URL the
    /// network serves is charged as one `requests` Usage to the page's Run
    /// before the navigation is sent (issue 3416). A program's navigation
    /// marks the page's send window before it goes out (ADR 0106).
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %page_id.0, operation = destination.operation(), run_id = tracing::field::Empty, url.origin = tracing::field::Empty))]
    #[expect(
        clippy::option_option,
        reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
    )]
    async fn navigate(
        &self,
        page_id: &PageId,
        (destination, navigating): (Destination<'_>, Navigating),
        wait_until: WaitUntil,
        timeout: Option<Option<u64>>,
        charge: Charge,
        context: &CallContext,
    ) -> Result<Navigated, CallError> {
        let span = tracing::Span::current();
        if let Destination::Url(url) = destination {
            span.record("url.origin", goodfetch_engine::log::origin(url));
        }
        let (page, spent) = self
            .open(page_id, |page| page.driver.navigation_deadline(timeout))
            .await?;
        span.record("run_id", page.run.id().as_str());
        let navigator = Navigator {
            acting_for_model: context.surface().acts_for_model(),
            recipe_run: page
                .run
                .settings()
                .is_some_and(|settings| settings.resolved().recipe_run()),
            // No engine serves accounts before area 12's: a person's own
            // engine serves the program's own code alone (`engine_context::CALLER`).
            accounts: false,
        };
        if let Destination::Url(url) = destination {
            match egress::navigation(url, navigator).map_err(|refused| refused.error())? {
                egress::Destination::JavaScript => {
                    let document = page.driver.document_url().await?;
                    self.interception
                        .judge_javascript(page_id, url, &document)?;
                }
                egress::Destination::Network if charge == Charge::OneRequest => {
                    charge_request(&page.run, context).await?;
                }
                egress::Destination::Network
                | egress::Destination::Blank
                | egress::Destination::Local => {}
            }
        }
        let _files = navigator
            .reads_files()
            .then(|| self.interception.allow_files(page_id, navigator));
        let events = context.events();
        let deadline = page.driver.navigation_deadline(timeout)?;
        let blocked_before = self.interception.lists_blocked(page_id);
        if navigating == Navigating::Program {
            page.sending(destination.operation()).await?;
        }
        let navigated = page
            .life
            .call(
                PageEvent::Goto,
                page.driver.having_spent(spent).navigate(
                    destination,
                    wait_until,
                    deadline.as_ref(),
                ),
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
        let work = page.run.metrics().counted_work();
        let block = blocked::block_of(
            &page.driver,
            work,
            self.blocks.as_ref(),
            &response,
            operation,
        )
        .await?;
        let touched = self.interception.lists_blocked(page_id) > blocked_before;
        if let Some(block) = &block
            && !(touched && block.checks_the_browser())
        {
            return Err(blocked::blocked(block.clone(), &response, received));
        }
        let judged =
            recovery::judge(&arrival::judging(&page, page_id, destination), &response).await;
        let first = blocking_off::FirstLoad {
            destination,
            wait_until,
            timeout,
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
        let (page, spent) = self
            .open(&locator.page_id, |page| {
                Ok(Some(page.driver.action_deadline().clone()))
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let (limit, duration) = page.action_timeout.clone();
        let request = Request {
            operation,
            selector: &locator.selector,
            action,
            window: Window {
                limit,
                duration,
                spent,
            },
            form: context.form(),
            hiding: Hidden {
                live: page.hiding.as_deref(),
                sheets: &page.sheets,
                run: &page.run,
                page_id: &locator.page_id,
                session_id: &page.session,
            },
            sent: SentMark {
                run: &page.run,
                lease: page.lease(),
            },
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
    /// page's events keep, whole or as cut, for `response.body`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_REQUEST.name, run_id = tracing::field::Empty, url.origin = goodfetch_engine::log::origin(&params.url)))]
    async fn request(&self, params: PageRequestParams) -> Result<PageResponse, CallError> {
        let max_redirects = redirect_count(params.max_redirects)?;
        headers_sent(params.headers.as_ref())?;
        let (page, spent) = self
            .open(&params.page_id, |page| {
                page.driver.navigation_deadline(None)
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let steps = self.interception.engine_requests(&params.page_id)?;
        let events = self.events.get(&params.page_id)?;
        let limits = Limits::of_run(page.run.settings());
        let request = browser_fetch::Request {
            url: &params.url,
            method: params.method.as_deref(),
            headers: params.headers.as_ref(),
            post_data: params.post_data.as_deref(),
            max_redirects,
            option: MAX_REDIRECTS_PARAMETER,
            limits,
            read: browser_fetch::BodyRead::Whole,
        };
        page.sending(PAGE_REQUEST.name).await?;
        // A cancelled call closes the engine's page as it drops.
        // A command its lease check withheld ends the page (P31).
        let engine_page = self
            .sessions
            .engine_page(&page.session, page.driver.value(), EngineWork::Request)
            .await
            .map_err(|error| page.life.withheld(error))?;
        let fetched =
            browser_fetch::send(&engine_page.driver().having_spent(spent), &steps, &request).await;
        engine_page.close().await;
        let fetched = fetched.map_err(|error| page.life.withheld(error))?;
        // A page a program opened records what the response's robots.txt
        // says, beside the call, never before it (ADR 0086).
        if let Some(reader) = sessions::robots_txt::Reader::of(
            &self.sessions,
            &params.page_id,
            &page.session,
            page.driver.value(),
            &page.run,
        ) {
            reader.read(steps, &fetched.response.url);
        }
        let kept = match fetched.body {
            browser_fetch::Body::Whole(bytes) => {
                let handle = page.run.store_body(bytes.into()).await.map_err(|error| {
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

    /// Runs `expression`, a page function's call as `page_functions` built
    /// it, on the page `page_id`, in `world`, the main world when absent,
    /// for the operation `operation`, and answers its JSON; a main-world
    /// call marks the Session in Provenance as `injection`. Its errors are
    /// spelled in `form`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %page_id.0, operation = operation, run_id = tracing::field::Empty))]
    async fn evaluate(
        &self,
        page_id: &PageId,
        (expression, world): (&str, Option<World>),
        (injection, operation): (Injection, &'static str),
        form: Form,
    ) -> Result<Evaluated, CallError> {
        let (page, _) = self
            .open(page_id, |page| {
                Ok(Some(page.driver.action_deadline().clone()))
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let world = world.unwrap_or_default();
        if world == World::Main {
            page.injections
                .mark(&page.run, &page.session, page_id, injection)
                .await;
        }
        page.sending(operation).await?;
        let returns = page
            .life
            .call(
                PageEvent::Read,
                page.driver.evaluate(expression, world, operation),
            )
            .await?;
        page_functions::answered(returns, (form, operation))
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
        let page = self.opened(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        page.sending(PAGE_ADD_INIT_SCRIPT.name).await?;
        // Registered on a task of the Run's, so a call dropped meanwhile
        // never leaves the script registered: its registration, answered to
        // no one, is removed there, counted before `pending` goes.
        let (answer, answered) = tokio::sync::oneshot::channel::<Result<Registered, CallError>>();
        let (driver, run, frames) = (
            page.driver.clone(),
            page.run.clone(),
            page.frame_scripts.clone(),
        );
        let counted = (self.init_scripts.clone(), params.page_id.clone());
        page.run.spawn(async move {
            let registered = async {
                let identifier = driver
                    .add_init_script(source.clone(), PAGE_ADD_INIT_SCRIPT.name)
                    .await?;
                // The page's out-of-process iframes take it too.
                let key = frames.add(source).await;
                Ok::<_, CallError>(Registered::new(
                    driver,
                    identifier,
                    run,
                    counted,
                    (frames, key),
                ))
            }
            .await;
            drop(answer.send(registered));
            drop(pending);
        });
        let registered = page
            .life
            .call(PageEvent::Read, async {
                match answered.await {
                    Ok(registered) => registered,
                    // The Run ended under it: its ending answers.
                    Err(_) => std::future::pending().await,
                }
            })
            .await?;
        page.injections
            .mark(
                &page.run,
                &page.session,
                &params.page_id,
                Injection::InitScript,
            )
            .await;
        let life = page.life.clone();
        drop(page);
        life.terminal().await;
        registered.ended();
        Ok(InitScriptEnded {})
    }

    /// The open page `page_id` once no init script of its is on its way in
    /// or out, so that a call sent after `page.add_init_script`, a gesture
    /// or a navigation, meets the script registered, and one sent after its
    /// call was cancelled meets it removed, and once `Runtime` a `page.on` sent
    /// before the call asked for is on, so that what the call makes the page
    /// log is read live (ADR 0019); as [`Self::opened`] says. That wait counts
    /// inside the call's own limit, the deadline `limit` reads off the page,
    /// if it has one: past it the call raises `deadline_exceeded`, and what it
    /// spent is answered beside the page, for the call's work to take off
    /// what it allows.
    async fn open(
        &self,
        page_id: &PageId,
        limit: impl FnOnce(&OpenPage) -> Result<Option<Deadline>, CallError>,
    ) -> Result<(OpenPage, Duration), CallError> {
        self.init_scripts.settled(page_id).await;
        let mut spent = Duration::ZERO;
        if let Some(arming) = self.events.arming(page_id) {
            let page = self.sessions.page(page_id).await?;
            let started = page.clock.now();
            match limit(&page)? {
                Some(limit) => {
                    limit
                        .within(&page.clock, WaitedFor::BROWSER_ANSWER, arming)
                        .await?;
                }
                None => arming.await,
            }
            spent = page.clock.now() - started;
        }
        Ok((self.opened(page_id).await?, spent))
    }

    /// The open page `page_id`, or `closed`; a failure a route's or a
    /// dialog handler left on it is raised here, by the page's next call,
    /// and so is the Block cause of a document a navigation no call waited
    /// for reached (ADR 0092).
    async fn opened(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
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
        // With `consent.handling: off` no runner looks, so there is nothing
        // to ask (#2434).
        if navigated
            && ConsentHandling::of(&page.run) != ConsentHandling::Off
            && crate::consent::first_pass_found(&page.driver, page.run.metrics().counted_work())
                .await
        {
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

    /// Judges the document the main frame of `page`, the page `page_id`,
    /// committed last, unless a call judged it already, as every page of a
    /// Session is judged (ADRs 0006 and 0092): its Block cause first, then
    /// its Page check. For the call that waited for its navigation, the
    /// operation `waiting`, whose code a failure to read the document
    /// raises; for the page's next call, `None`, when such a failure reads
    /// as a document showing nothing.
    async fn judge_unjudged(
        &self,
        page: &OpenPage,
        page_id: &PageId,
        waiting: Option<&'static str>,
    ) -> Result<(), CallError> {
        let Some((response, received, requested)) = self
            .events
            .get(page_id)
            .ok()
            .and_then(|events| events.take_unjudged())
        else {
            return Ok(());
        };
        if let Some(block) = blocked::block_of(
            &page.driver,
            page.run.metrics().counted_work(),
            self.blocks.as_ref(),
            &response,
            waiting,
        )
        .await?
        {
            return Err(blocked::blocked(block, &response, received));
        }
        recovery::judge(
            &arrival::judging_unsent(page, page_id, &requested, waiting),
            &response,
        )
        .await
    }

    /// Judges, for a wait such as `page.wait_for_url` that waited for the
    /// navigation of the page `page_id`, the document it reached: its
    /// Block cause and its Page check (ADRs 0006 and 0092).
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
    /// its Run's, the Clean-up rules of its Session applied before the main
    /// content (ADR 0047), on a document a fetch tier served those of its
    /// Run, as in every tier; Provenance records each call with the rules'
    /// version and what they removed (G6).
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_FORMATS.name, run_id = tracing::field::Empty))]
    async fn formats(&self, params: FormatsParams) -> Result<PageFormats, CallError> {
        if let Some(served) = self.product.document(&params.page_id) {
            tracing::Span::current().record("run_id", served.run.id().as_str());
            let request = params.formats;
            // Formats read ahead in the document's hand-off answer a call
            // asking for them (issue 2189).
            if let Some(ahead) = &served.formats
                && let Some(formats) = ahead.answers(request)
            {
                if let Some(entry) = ahead.entry(&params.page_id) {
                    trace::provenance(&served.run, entry).await;
                }
                return Ok(formats.clone());
            }
            let document = SerializedDocument {
                html: served.html.clone(),
                url: served.url.clone(),
                base_url: served.base_url.clone(),
            };
            let extractor = extractor::served(
                served.run.settings().map(RunSettings::resolved),
                served.extractor,
            );
            let measure =
                clean_up::ShareMeasure::of(served.run.settings().map(RunSettings::resolved));
            // A request that reads nothing takes no cosmetic rule, so it
            // loads no Filter list (#2573).
            let cosmetic = if clean_up::reads_nothing(&request) {
                None
            } else {
                self.interception.served_cosmetic(&served.run).await
            };
            let work = served.run.metrics().counted_work().clone();
            let read = move |kept: Option<&ParsedHtml>| {
                clean_up::read(
                    (&document, kept),
                    Serialization::Served,
                    request,
                    (extractor.extractor(), measure),
                    (cosmetic.as_ref(), &[]),
                    &work,
                )
            };
            // A page whose handler's reads keep its untouched tree reads a
            // copy of it, on its owner (ADR 0138); any other parses it.
            let (formats, read) = match &served.tree {
                // A request that reads nothing of the page needs no tree
                // and no blocking thread (issue 2189).
                _ if clean_up::reads_nothing(&request) => read(None),
                // Its tree is gone only as its Run stops, or after a read
                // of it panicked.
                Some(tree) => tree
                    .read(move |kept| read(Some(kept)))
                    .await
                    .ok_or_else(|| sessions::page_closed(&params.page_id))?,
                None => {
                    served
                        .run
                        .owner()
                        .analysis(served.run.metrics().counted_work(), move || read(None))
                        .await
                }
            };
            if let Some(read) = read {
                trace::provenance(&served.run, read.entry(&params.page_id, None)).await;
            }
            return Ok(formats);
        }
        let (page, _) = self
            .open(&params.page_id, |page| {
                page.driver.navigation_deadline(None)
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let document = page
            .life
            .call(PageEvent::Read, page.driver.document())
            .await?;
        let request = params.formats;
        let (_, session) = self.sessions.own_settings(&page.session).await?;
        let extractor = extractor::chosen(
            page.run.settings().map(RunSettings::resolved),
            (&session, page.driver.value().extractor()),
            &page.settings,
        );
        let measure = clean_up::ShareMeasure::of(page.run.settings().map(RunSettings::resolved));
        let cosmetic = self.interception.cosmetic(&params.page_id);
        let hidden = page.sheets.runner_hidden();
        let work = page.run.metrics().counted_work().clone();
        let (formats, read) = page
            .run
            .owner()
            .analysis(page.run.metrics().counted_work(), move || {
                clean_up::read(
                    (&document, None),
                    Serialization::Dom,
                    request,
                    (extractor.extractor(), measure),
                    (cosmetic.as_ref(), &hidden),
                    &work,
                )
            })
            .await;
        if let Some(read) = read {
            trace::provenance(&page.run, read.entry(&params.page_id, Some(&page.session))).await;
        }
        Ok(formats)
    }

    /// The fields `params` declares, read from the browser page's document
    /// in one engine read (ADR 0147): each locator found by the page itself,
    /// as a served document's tree finds it, and each value read by the
    /// extractor's `fields` module. A field the page's Run refuses, or a
    /// locator the page does not read, is `invalid_params`. A page a fetch
    /// tier served answers `page.read` itself, never reaching here.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_READ.name, run_id = tracing::field::Empty))]
    async fn read_fields(&self, params: PageReadParams) -> Result<PageRead, CallError> {
        let (page, _) = self
            .open(&params.page_id, |page| {
                page.driver.navigation_deadline(None)
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let work = page.run.metrics().counted_work();
        for (name, field) in &params.fields {
            goodfetch_extractor::fields::check(name, field, work).map_err(|sentence| {
                CallError::protocol(ProtocolFailure::InvalidParams, sentence)
            })?;
        }
        let declared = goodfetch_extractor::fields::Fields::of(&params.fields, work);
        let reading = page.driver.counting(Row::PageFieldReadCommands);
        // A document replaced under the read is read once more in the next
        // one, as `page.evaluate`'s is.
        let mut answer = None;
        for _ in 0..2 {
            answer = page
                .life
                .call(
                    PageEvent::Read,
                    reading.read_in_page(
                        fields::READ,
                        fields::argument(&params.fields),
                        PAGE_READ.name,
                    ),
                )
                .await?;
            if answer.is_some() {
                break;
            }
        }
        let answer = answer.ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!(
                    "The page's document went, or its fields' read threw, twice in a row as `{}` read it.",
                    PAGE_READ.name
                ),
            )
        })?;
        let answered = fields::answered(&answer, &params.fields, PAGE_READ.name)?;
        let read = declared.read(answered.found, &answered.base_url);
        Ok(PageRead {
            values: read.values,
            unread: read.unread,
        })
    }
}
