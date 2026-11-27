//! The handler of `locator.scroll_collect` (#807): it reads a virtualized
//! or infinite list whole, step by step. Each step reads every match of
//! the locator through the program's page function (`collect.js`), keeps
//! each new key's value in first-seen order, scrolls the last match into
//! view through the guard, a gesture like a click's, and waits up to its
//! `step_wait` for an element with a new key. It ends `end_of_list` after
//! `idle_steps` steps in a row without one, or `max_rows` once it kept
//! that many, all under the call's `timeout`, else the Run's `timeout.wait`,
//! whose `deadline_exceeded` says what it kept. No other read scrolls.
#![expect(
    clippy::disallowed_types,
    reason = "a collection's kept rows' one lock guards one read or write, never held across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::connection::{Form, WaitedFor};
use goodfetch_protocol::page::Locator;
use goodfetch_protocol::page::functions::{
    Declared, LOCATOR_SCROLL_COLLECT, ListEnded, LocatorScrollCollectParams, ScrollCollected, World,
};

use super::{Pages, gesture, in_call};
use crate::cdp::CdpPage;
use crate::guard::Action;
use crate::page_functions::{self, Collected, Injection};
use crate::page_reads;
use crate::sessions::{OpenPage, PageEvent};

mod kept;

use kept::{Bounds, Kept, declared};

/// How long a step leaves between two reads while it waits for a new key.
const POLL: Duration = Duration::from_millis(100);

/// Adds the handler of `locator.scroll_collect`.
pub(super) fn with_scroll_collect_handler(
    contribution: Contribution,
    pages: &Pages,
) -> Contribution {
    let collecting = pages.clone();
    contribution.with_handler(Handler::new(
        &LOCATOR_SCROLL_COLLECT,
        move |params: LocatorScrollCollectParams, context: CallContext| {
            let pages = collecting.clone();
            in_call((LOCATOR_SCROLL_COLLECT.name, context.form()), async move {
                let bounds = Bounds::of(&params)?;
                Box::pin(pages.scroll_collect(params, bounds, context.form())).await
            })
        },
    ))
}

/// One collection's page and how it reads and scrolls it.
struct Collection<'a> {
    page: &'a OpenPage,
    locator: &'a Locator,
    expression: String,
    world: World,
    form: Form,
}

impl Collection<'_> {
    /// Reads every match once through the program's page function.
    async fn read(&self) -> Result<Collected, CallError> {
        let operation = LOCATOR_SCROLL_COLLECT.name;
        let returns = self
            .page
            .life
            .call(
                PageEvent::Read,
                page_reads::evaluate(
                    &CdpPage::of(&self.page.driver),
                    &self.expression,
                    self.world,
                    operation,
                ),
            )
            .await?;
        page_functions::collected(page_functions::answered(returns, (self.form, operation))?)
    }

    /// Scrolls the last match into view through the guard, a gesture under
    /// the Run's `timeout.action`.
    async fn scroll(&self) -> Result<(), CallError> {
        gesture(
            self.page,
            LOCATOR_SCROLL_COLLECT.name,
            self.locator,
            Action::Scroll,
            Duration::ZERO,
            self.form,
        )
        .await
    }

    /// Reads and scrolls until the list ends, keeping its rows in `kept`.
    async fn run(&self, bounds: Bounds, kept: &Mutex<Kept>) -> Result<ScrollCollected, CallError> {
        let keep = |collected: Collected| {
            let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
            let added = kept.keep(collected, bounds.max_rows);
            (added, kept.full(bounds.max_rows))
        };
        let answer = |ended: ListEnded, declared: Option<Declared>| ScrollCollected {
            rows: std::mem::take(&mut kept.lock().unwrap_or_else(PoisonError::into_inner).rows),
            ended,
            declared,
        };
        let clock = &self.page.clock;
        let mut collected = self.read().await?;
        let mut matched = collected.matched;
        let mut list = declared(collected.declared);
        if keep(collected).1 {
            return Ok(answer(ListEnded::MaxRows, list));
        }
        let mut idle = 0;
        loop {
            if matched > 0 {
                self.scroll().await?;
            }
            let until = clock.now() + bounds.step_wait;
            let grew = loop {
                collected = self.read().await?;
                matched = collected.matched;
                if matched > 0 {
                    list = declared(collected.declared);
                }
                let (added, full) = keep(collected);
                if full {
                    return Ok(answer(ListEnded::MaxRows, list));
                }
                if added > 0 {
                    break true;
                }
                let now = clock.now();
                if now >= until {
                    break false;
                }
                clock.sleep(POLL.min(until - now)).await;
            };
            idle = if grew { 0 } else { idle + 1 };
            if idle >= bounds.idle_steps {
                return Ok(answer(ListEnded::EndOfList, list));
            }
        }
    }
}

impl Pages {
    /// Collects the list `params` names, as the module says.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.locator.page_id.0, operation = LOCATOR_SCROLL_COLLECT.name, run_id = tracing::field::Empty))]
    async fn scroll_collect(
        &self,
        params: LocatorScrollCollectParams,
        bounds: Bounds,
        form: Form,
    ) -> Result<ScrollCollected, CallError> {
        let page_id = &params.locator.page_id;
        let (page, spent) = self
            .open(page_id, |_| {
                let deadline = self.events(page_id)?.wait_deadline(params.timeout)?;
                Ok(deadline.deadline().cloned())
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.identity.id().as_str());
        let deadline = self
            .events(page_id)?
            .wait_deadline(params.timeout)?
            .having_spent(spent);
        let world = params.world.unwrap_or(World::Isolated);
        if world == World::Main {
            page.injections
                .mark(
                    &page.run.trace,
                    &page.session,
                    page_id,
                    Injection::ScrollCollect,
                )
                .await;
        }
        page.sending(LOCATOR_SCROLL_COLLECT.name).await?;
        let collection = Collection {
            page: &page,
            locator: &params.locator,
            expression: page_functions::scroll_collect(
                &params.function,
                params.arg.as_ref(),
                &params.locator.selector,
            ),
            world,
            form,
        };
        let kept = Mutex::new(Kept::default());
        deadline
            .within_seeing(
                WaitedFor::LIST_END,
                Box::pin(collection.run(bounds, &kept)),
                || {
                    Some(
                        kept.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .observed(),
                    )
                },
            )
            .await?
    }
}
