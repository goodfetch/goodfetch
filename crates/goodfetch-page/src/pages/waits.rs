//! The handlers of the app-state waits (ADR 0080): those on the page's
//! requests, URL and load states, armed in its events when their call
//! arrives, `page.wait_for_function`, which runs its function until it is
//! truthy, and the locator waits, through the guard's check, each under the
//! Run's `timeout.wait`.

use std::time::Duration;

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::Locator;
use goodfetch_protocol::page::functions::{Evaluated, World};
use goodfetch_protocol::page::waits::{
    Count, Counted, LOCATOR_WAIT_FOR, LOCATOR_WAIT_FOR_COUNT, LocatorWaitForParams,
    PAGE_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_REQUEST,
    PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL, Reached, WaitForCountParams, WaitForExchangeParams,
    WaitForFunctionParams, WaitForLoadStateParams, WaitForUrlParams,
};

use super::{Pages, pattern_of};
use crate::guard::{self, Awaited, Wait, Window};
use crate::page_functions::{self, Injection};
use crate::sessions::PageEvent;

/// How long `page.wait_for_function` leaves between two checks when the
/// call names no interval: about one frame's.
const FRAME: Duration = Duration::from_millis(16);

/// Adds the handlers of the waits on the page's requests and responses,
/// each armed when its call arrives.
fn with_exchange_waits(contribution: Contribution, pages: &Pages) -> Contribution {
    let (requesting, responding) = (pages.events.clone(), pages.events.clone());
    let (judging_requests, judging_responses) = (pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_REQUEST,
            move |params: WaitForExchangeParams, _context: CallContext| {
                let armed = requesting.get(&params.page_id).and_then(|events| {
                    let pattern = params.url.as_ref().map(pattern_of).transpose()?;
                    Ok(events.wait_for_request(pattern))
                });
                let pages = judging_requests.clone();
                async move {
                    let request = armed?.await?;
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
                let armed = responding.get(&params.page_id).and_then(|events| {
                    let pattern = params.url.as_ref().map(pattern_of).transpose()?;
                    Ok(events.wait_for_response(pattern))
                });
                let pages = judging_responses.clone();
                async move {
                    let response = armed?.await?;
                    pages
                        .judge_waited(&params.page_id, PAGE_WAIT_FOR_RESPONSE.name)
                        .await?;
                    Ok::<_, CallError>(response)
                }
            },
        ))
}

/// Adds the handlers of the app-state waits: those on the page's requests,
/// URL and load states each armed when its call arrives, the others
/// checking from then on.
pub(super) fn with_wait_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (locating, loading) = (pages.events.clone(), pages.events.clone());
    let (functioning, waiting, counting) = (pages.clone(), pages.clone(), pages.clone());
    let (judging, judging_loads) = (pages.clone(), pages.clone());
    // Each wait on the page's requests, URL or load states judges the
    // document a navigation it waited for reached, its Block cause and its
    // Page check, raised by that wait (ADRs 0006 and 0092).
    with_exchange_waits(contribution, pages)
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_URL,
            move |params: WaitForUrlParams, _context: CallContext| {
                let armed = locating.get(&params.page_id).and_then(|events| {
                    let pattern = pattern_of(&params.url)?;
                    Ok(events.wait_for_url(pattern, params.wait_until.unwrap_or_default()))
                });
                let pages = judging.clone();
                async move {
                    let reached = armed?.await?;
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
                let armed = loading
                    .get(&params.page_id)
                    .map(|events| events.wait_for_load_state(params.state.unwrap_or_default()));
                let pages = judging_loads.clone();
                async move {
                    let reached = armed?.await?;
                    pages
                        .judge_waited(&params.page_id, PAGE_WAIT_FOR_LOAD_STATE.name)
                        .await?;
                    Ok::<_, CallError>(reached)
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_WAIT_FOR_FUNCTION,
            move |params: WaitForFunctionParams, context: CallContext| {
                let pages = functioning.clone();
                async move { pages.wait_for_function(params, context.surface()).await }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_WAIT_FOR,
            move |params: LocatorWaitForParams, context: CallContext| {
                let pages = waiting.clone();
                let awaited = Awaited::State(params.state.unwrap_or_default());
                async move {
                    pages
                        .wait_on_locator(LOCATOR_WAIT_FOR.name, params.locator, awaited, &context)
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
                    let counted = pages
                        .wait_on_locator(
                            LOCATOR_WAIT_FOR_COUNT.name,
                            params.locator,
                            Awaited::Count(count),
                            &context,
                        )
                        .await?;
                    Ok::<Counted, CallError>(Counted {
                        count: counted.unwrap_or_default(),
                    })
                }
            },
        ))
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
    /// Runs the page function `params` names on its page, again at its
    /// interval and in each new document, until its result is truthy, and
    /// answers that result, unless the Run's `timeout.wait` passes first; a
    /// main-world call marks the Session in Provenance. Its errors are
    /// spelled for `surface`.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_WAIT_FOR_FUNCTION.name, run_id = tracing::field::Empty))]
    async fn wait_for_function(
        &self,
        params: WaitForFunctionParams,
        surface: Surface,
    ) -> Result<Evaluated, CallError> {
        let interval = match params.polling_interval {
            Some(0) => {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "the polling interval is 0 ms: it is at least 1",
                ));
            }
            Some(millis) => Duration::from_millis(millis),
            None => FRAME,
        };
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let (deadline, clock) = self.events.get(&params.page_id)?.wait_deadline();
        let world = params.world.unwrap_or_default();
        if world == World::Main {
            self.injections
                .mark(
                    &page.run,
                    &page.session,
                    &params.page_id,
                    Injection::WaitForFunction,
                )
                .await;
        }
        let expression = page_functions::expression(&params.function, params.arg.as_ref());
        let truthy = page_functions::until_truthy(
            &page.driver,
            &expression,
            world,
            (interval, &clock),
            surface,
        );
        page.life
            .call(PageEvent::Read, async {
                deadline.within(&clock, "function", truthy).await?
            })
            .await
    }

    /// Waits until the matches of `locator` are as `awaited` says, through
    /// the guard's check, under the Run's `timeout.wait`; answers how many
    /// matched for a count.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %locator.page_id.0, operation = operation, run_id = tracing::field::Empty))]
    async fn wait_on_locator(
        &self,
        operation: &'static str,
        locator: Locator,
        awaited: Awaited,
        context: &CallContext,
    ) -> Result<Option<u64>, CallError> {
        let page = self.open(&locator.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
        let (deadline, _) = self.events.get(&locator.page_id)?.wait_deadline();
        let wait = Wait {
            operation,
            selector: &locator.selector,
            awaited,
            window: Window {
                limit: deadline.limit().clone(),
                duration: deadline.duration(),
            },
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
