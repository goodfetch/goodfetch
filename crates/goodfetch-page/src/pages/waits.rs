//! The handlers of the app-state waits (ADR 0080): those on the page's
//! requests, URL and load states, armed in its events when their call
//! arrives, `page.wait_for_function` and `locator.wait_for_function`, which
//! run their function until it is truthy, and the locator waits, through the guard's check, each under its
//! call's own `timeout`, else the Run's `timeout.wait`, and each a call on
//! its page, which raises what ended the page under it.

use std::time::Duration;

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::functions::{Evaluated, World};
use goodfetch_protocol::page::waits::{
    Count, Counted, LOCATOR_WAIT_FOR, LOCATOR_WAIT_FOR_COUNT, LOCATOR_WAIT_FOR_FUNCTION,
    LocatorWaitForFunctionParams, LocatorWaitForParams, PAGE_WAIT_FOR_FUNCTION,
    PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_REQUEST, PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL,
    Reached, WaitForCountParams, WaitForExchangeParams, WaitForFunctionParams,
    WaitForLoadStateParams, WaitForUrlParams,
};
use goodfetch_protocol::page::{Locator, PageId};

use super::{Pages, pattern_of};
use crate::guard::{self, Awaited, Wait, Waited, Window};
use crate::page_events::WaitDeadline;
use crate::page_functions::{self, Injection, Matches};
use crate::sessions::PageEvent;

/// How long `page.wait_for_function` leaves between two checks when the
/// call names no interval: about one frame's.
const FRAME: Duration = Duration::from_millis(16);

/// Adds the handlers of the app-state waits: those on the page's requests,
/// URL and load states each armed when its call arrives, the others
/// checking from then on.
pub(super) fn with_wait_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    with_checking_handlers(with_armed_handlers(contribution, pages), pages)
}

/// Adds the handlers of the waits on the page's requests, URL and load
/// states, each armed when its call arrives. Each judges the document a
/// navigation it waited for reached, its Block cause and its Page check,
/// raised by that wait (ADRs 0006 and 0092).
fn with_armed_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (requesting, responding, locating, loading) =
        (pages.clone(), pages.clone(), pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_REQUEST,
            move |params: WaitForExchangeParams, _context: CallContext| {
                let pages = requesting.clone();
                let armed = pages.events.get(&params.page_id).and_then(|events| {
                    let pattern = params.url.as_ref().map(pattern_of).transpose()?;
                    let deadline = events.wait_deadline(params.timeout)?;
                    Ok(events.wait_for_request(pattern, deadline))
                });
                async move {
                    let request = Box::pin(pages.waited(&params.page_id, armed?)).await?;
                    pages
                        .judge_waited(&params.page_id, PAGE_WAIT_FOR_REQUEST.name)
                        .await?;
                    Ok::<_, CallError>(request)
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_RESPONSE,
            move |params: WaitForExchangeParams, _context: CallContext| {
                let pages = responding.clone();
                let armed = pages.events.get(&params.page_id).and_then(|events| {
                    let pattern = params.url.as_ref().map(pattern_of).transpose()?;
                    let deadline = events.wait_deadline(params.timeout)?;
                    Ok(events.wait_for_response(pattern, deadline))
                });
                async move {
                    let response = Box::pin(pages.waited(&params.page_id, armed?)).await?;
                    pages
                        .judge_waited(&params.page_id, PAGE_WAIT_FOR_RESPONSE.name)
                        .await?;
                    Ok::<_, CallError>(response)
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_URL,
            move |params: WaitForUrlParams, _context: CallContext| {
                let pages = locating.clone();
                let armed = pages.events.get(&params.page_id).and_then(|events| {
                    let pattern = pattern_of(&params.url)?;
                    let deadline = events.wait_deadline(params.timeout)?;
                    Ok(events.wait_for_url(
                        pattern,
                        params.wait_until.unwrap_or_default(),
                        deadline,
                    ))
                });
                // The document the navigation it waited for reached is
                // judged for its Block cause, raised by this call (ADR
                // 0092).
                async move {
                    let reached = Box::pin(pages.waited(&params.page_id, armed?)).await?;
                    pages
                        .judge_waited(&params.page_id, PAGE_WAIT_FOR_URL.name)
                        .await?;
                    Ok::<_, CallError>(reached)
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_LOAD_STATE,
            move |params: WaitForLoadStateParams, _context: CallContext| {
                let pages = loading.clone();
                let armed = pages.events.get(&params.page_id).and_then(|events| {
                    let deadline = events.wait_deadline(params.timeout)?;
                    Ok(events.wait_for_load_state(params.state.unwrap_or_default(), deadline))
                });
                async move {
                    let reached = Box::pin(pages.waited(&params.page_id, armed?)).await?;
                    pages
                        .judge_waited(&params.page_id, PAGE_WAIT_FOR_LOAD_STATE.name)
                        .await?;
                    Ok::<_, CallError>(reached)
                }
            },
        ))
}

/// Adds the handlers of the waits that check the page from when their call
/// arrives: the function waits and the locator waits.
fn with_checking_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (functioning, locating, waiting, counting) =
        (pages.clone(), pages.clone(), pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_FUNCTION,
            move |params: WaitForFunctionParams, context: CallContext| {
                let pages = functioning.clone();
                async move {
                    let expression =
                        page_functions::expression(&params.function, params.arg.as_ref());
                    let wait = FunctionWait {
                        page_id: &params.page_id,
                        expression: &expression,
                        world: params.world,
                        polling_interval: params.polling_interval,
                        timeout: params.timeout,
                        by: (Injection::WaitForFunction, PAGE_WAIT_FOR_FUNCTION.name),
                    };
                    Box::pin(pages.wait_for_function(wait, context.surface())).await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_WAIT_FOR_FUNCTION,
            move |params: LocatorWaitForFunctionParams, context: CallContext| {
                let pages = locating.clone();
                async move {
                    let expression = page_functions::on_locator(
                        &params.function,
                        params.arg.as_ref(),
                        (&params.locator.selector, Matches::One),
                    );
                    let wait = FunctionWait {
                        page_id: &params.locator.page_id,
                        expression: &expression,
                        world: params.world,
                        polling_interval: params.polling_interval,
                        timeout: params.timeout,
                        by: (
                            Injection::LocatorWaitForFunction,
                            LOCATOR_WAIT_FOR_FUNCTION.name,
                        ),
                    };
                    Box::pin(pages.wait_for_function(wait, context.surface())).await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_WAIT_FOR,
            move |params: LocatorWaitForParams, context: CallContext| {
                let pages = waiting.clone();
                let awaited = Awaited::State(params.state.unwrap_or_default());
                async move {
                    let deadline = pages
                        .events
                        .get(&params.locator.page_id)?
                        .wait_deadline(params.timeout)?;
                    pages
                        .wait_on_locator(
                            LOCATOR_WAIT_FOR.name,
                            params.locator,
                            (awaited, deadline),
                            &context,
                        )
                        .await?;
                    Ok::<Reached, CallError>(Reached {})
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_WAIT_FOR_COUNT,
            move |params: WaitForCountParams, context: CallContext| {
                let pages = counting.clone();
                async move {
                    let count = count_of(params.count)?;
                    let deadline = pages
                        .events
                        .get(&params.locator.page_id)?
                        .wait_deadline(params.timeout)?;
                    let waited = pages
                        .wait_on_locator(
                            LOCATOR_WAIT_FOR_COUNT.name,
                            params.locator,
                            (Awaited::Count(count), deadline),
                            &context,
                        )
                        .await?;
                    match waited {
                        Waited::Count(count) => Ok::<Counted, CallError>(Counted { count }),
                        Waited::State => Err(CallError::protocol(
                            ProtocolFailure::InternalError,
                            "a count wait reached a state, not a count",
                        )),
                    }
                }
            },
        ))
}

/// A function wait: its page, its page function's call as
/// `page_functions` built it, its world, its interval and its deadline, as
/// its call gave them, and the operation it is with its Provenance mark.
struct FunctionWait<'a> {
    page_id: &'a PageId,
    expression: &'a str,
    world: Option<World>,
    polling_interval: Option<u64>,
    #[expect(
        clippy::option_option,
        reason = "three states (ADR 0080): absent for `timeout.wait`, `null` for none, a value"
    )]
    timeout: Option<Option<u64>>,
    by: (Injection, &'static str),
}

/// `count`, or `invalid_params` when its least is above its most.
fn count_of(count: Count) -> Result<Count, CallError> {
    match (count.min, count.max) {
        (Some(min), Some(max)) if min > max => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("the count's `min`, {min}, is above its `max`, {max}"),
        )),
        _ => Ok(count),
    }
}

impl Pages {
    /// Answers `armed`, a wait on the page `page_id` armed when its call
    /// arrived, as a call on the page: a failure a handler left on the page
    /// is raised first, and a page that ends under the wait raises what
    /// ended it, `call_cancelled` or `page_crashed`, rather than `closed`.
    pub(super) async fn waited<T>(
        &self,
        page_id: &PageId,
        armed: impl Future<Output = Result<T, CallError>>,
    ) -> Result<T, CallError> {
        let page = self.open(page_id).await?;
        page.life
            .call(PageEvent::Read, async {
                match armed.await {
                    Err(error) if error.code() == CLOSED.name => {
                        Err(page.life.ending_or(error).await)
                    }
                    answered => answered,
                }
            })
            .await
    }

    /// Runs the page function of `wait` on its page, again at its interval
    /// and in each new document, until its result is truthy, and answers
    /// that result, unless its deadline passes first; a main-world call
    /// marks the Session in Provenance. Its errors are spelled for
    /// `surface`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %wait.page_id.0, operation = wait.by.1, run_id = tracing::field::Empty))]
    async fn wait_for_function(
        &self,
        wait: FunctionWait<'_>,
        surface: Surface,
    ) -> Result<Evaluated, CallError> {
        let (injection, operation) = wait.by;
        let interval = match wait.polling_interval {
            Some(0) => {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "the polling interval is 0 ms: it is at least 1",
                ));
            }
            Some(millis) => Duration::from_millis(millis),
            None => FRAME,
        };
        let page = self.open(wait.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let deadline = self.events.get(wait.page_id)?.wait_deadline(wait.timeout)?;
        let world = wait.world.unwrap_or_default();
        if world == World::Main {
            page.injections
                .mark(&page.run, &page.session, wait.page_id, injection)
                .await;
        }
        let truthy = Box::pin(page_functions::until_truthy(
            &page.driver,
            (wait.expression, operation),
            world,
            (interval, deadline.clock()),
            surface,
        ));
        page.life
            .call(PageEvent::Read, async {
                deadline.within("function", truthy).await?
            })
            .await
    }

    /// Waits until the matches of `locator` are as `awaited` says, through
    /// the guard's check, until `deadline` passes, if the wait has one;
    /// answers what it reached.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %locator.page_id.0, operation = operation, run_id = tracing::field::Empty))]
    async fn wait_on_locator(
        &self,
        operation: &'static str,
        locator: Locator,
        (awaited, deadline): (Awaited, WaitDeadline),
        context: &CallContext,
    ) -> Result<Waited, CallError> {
        let page = self.open(&locator.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let wait = Wait {
            operation,
            selector: &locator.selector,
            awaited,
            window: deadline.deadline().map(|deadline| Window {
                limit: deadline.limit().clone(),
                duration: deadline.duration(),
            }),
            surface: context.surface(),
        };
        page.life
            .call(
                PageEvent::Read,
                guard::wait(&page.driver, wait, &page.clock),
            )
            .await
    }
}
