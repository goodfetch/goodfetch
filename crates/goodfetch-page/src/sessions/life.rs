//! Where each Session and page is, kept through its table (`lifecycle`,
//! ADR 0128), and what that does to the calls on it. A call asks its
//! object's table first: a refused call raises the table's code, a call
//! that must wait waits for the state to change, and a served one runs
//! until it answers or its object ends under it. An object that ends
//! (closed, closed in the browser, crashed, its browser quit or crashed)
//! ends its calls in flight with the code its effect names, and a crash
//! leaves that code for every later call (spec #594's decision 3).
//!
//! A navigation is admitted through its own event (`Goto`), but the page
//! stays `Open` while it runs: the engine lets a later navigation supersede
//! one not yet committed (#698, ADR 0085), where the table's `Navigating`
//! row would have it wait, so `Navigating` is never entered, as ADR 0085's
//! note of 2026-10-24 records for the owner to decide.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::lease::LeaseCheck;
use goodfetch_engine::transition::{InvalidTransition, Refusal};
use goodfetch_protocol::connection::{CLOSED, ClosedValue, StoppedBy};
use goodfetch_protocol::governor::PAGE_MEMORY_EXCEEDED;
use goodfetch_protocol::page::{BrowserExit, PAGE_CRASHED, PageId, SessionId};
use goodfetch_protocol::settings::SettingLimit;
use serde_json::json;
use tokio::sync::watch;

use super::lifecycle::{
    self, Attributes, PageEffect, PageEvent, PageState, SessionEffect, SessionEvent, SessionState,
};
use crate::errors::{
    browser_crashed, cancelled, invalid_transition, page_memory_exceeded, renderer_crashed,
};

/// How long a call that failed as its page went waits for the page's end
/// to be known, so it raises what ended the page rather than its own
/// command's failure: a destroyed target waits up to 2 s for a quit, and a
/// lost browser up to 2 s more for its exit status.
const ENDING_WAIT: Duration = Duration::from_secs(5);

/// How a page ended, as its table's event names it.
#[derive(Clone, Debug)]
pub(crate) enum Ending {
    /// The program closed it or its Session (`Close`): what was closed.
    Closed(ClosedValue),
    /// Its target was destroyed (`TargetDestroyed`).
    Destroyed,
    /// Its renderer crashed (`RendererCrash`).
    RendererCrash,
    /// Its browser quit (`BrowserQuit`).
    BrowserQuit,
    /// Its browser crashed (`BrowserCrash`), with how it exited.
    BrowserCrash(Option<BrowserExit>),
    /// The Resource governor ended it (`MemoryExceeded`): it held `memory`
    /// past `limit`, `governor.browser.page_memory_cap`.
    MemoryExceeded {
        /// What it held, in bytes.
        memory: u64,
        /// The cap, with its value and layer.
        limit: SettingLimit,
    },
    /// Its crawl handler's lease ended (`LeaseEnded`): what its lease check
    /// raised for the command it withheld, which its calls in flight raise
    /// too.
    LeaseEnded(CallError),
}

impl Ending {
    fn event(&self) -> PageEvent {
        match self {
            Self::Closed(_) => PageEvent::Close,
            Self::Destroyed => PageEvent::TargetDestroyed,
            Self::RendererCrash => PageEvent::RendererCrash,
            Self::BrowserQuit => PageEvent::BrowserQuit,
            Self::BrowserCrash(_) => PageEvent::BrowserCrash,
            Self::MemoryExceeded { .. } => PageEvent::MemoryExceeded,
            Self::LeaseEnded(_) => PageEvent::LeaseEnded,
        }
    }
}

/// One page's state. Clones share it.
#[derive(Clone)]
pub(crate) struct PageLife {
    shared: Arc<PageShared>,
}

struct PageShared {
    page_id: PageId,
    state: watch::Sender<PageState>,
    /// What its calls in flight raise once it ended under them, and its
    /// later calls after a crash or an end past its memory cap.
    ended: watch::Sender<Option<CallError>>,
    /// How many calls on it are in flight.
    calls: watch::Sender<usize>,
    /// Set once it was found past its memory cap.
    past_cap: OnceLock<()>,
    /// Its lease check, a crawl handler's page's (ADR 0087); none on a page
    /// a program opens.
    lease: Option<Arc<dyn LeaseCheck>>,
    /// Set once its lease ended it, so its target closes.
    ended_by_lease: watch::Sender<bool>,
}

impl PageLife {
    /// The page `page_id`, `Open`, under the lease check `lease` when it is
    /// a crawl handler's.
    pub(crate) fn new(page_id: PageId, lease: Option<Arc<dyn LeaseCheck>>) -> Self {
        Self {
            shared: Arc::new(PageShared {
                page_id,
                state: watch::Sender::new(PageState::Open),
                ended: watch::Sender::new(None),
                calls: watch::Sender::new(0),
                past_cap: OnceLock::new(),
                lease,
                ended_by_lease: watch::Sender::new(false),
            }),
        }
    }

    /// Waits until the page's lease ended it (`LeaseEnded`, P31), after
    /// which the engine closes its target.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn ended_by_lease(&self) {
        let mut ended = self.shared.ended_by_lease.subscribe();
        let _ = ended.wait_for(|ended| *ended).await;
    }

    /// `error`, which a call on the page failed with: once its lease check
    /// refuses, that is the lease's end, which moves the page to `Closing`
    /// (spec #594's P31, decision 13), its calls in flight raising `error`
    /// and its later ones `closed`.
    pub(crate) fn withheld(&self, error: CallError) -> CallError {
        let Some(lease) = &self.shared.lease else {
            return error;
        };
        if lease.check().is_ok() {
            return error;
        }
        match self.end(&Ending::LeaseEnded(error.clone())) {
            Ok(PageEffect::Close) => {
                self.shared.ended_by_lease.send_replace(true);
            }
            Ok(_) => {}
            Err(refused) => tracing::error!(%refused, "a page's lease end was refused"),
        }
        error
    }

    /// Whether the page is found past its memory cap for the first time:
    /// only the first finding ends it.
    pub(crate) fn past_cap(&self) -> bool {
        self.shared.past_cap.set(()).is_ok()
    }

    /// Waits until no call on the page is in flight: its safe point, where
    /// the Resource governor ends it (ADR 0040).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn idle(&self) {
        let mut calls = self.shared.calls.subscribe();
        let _ = calls.wait_for(|calls| *calls == 0).await;
    }

    /// What a later call on the crashed page raises: `page_crashed` again,
    /// as its crash did.
    pub(crate) fn crashed(&self) -> CallError {
        self.shared
            .ended
            .borrow()
            .clone()
            .unwrap_or_else(renderer_crashed)
    }

    /// Waits until the page is `Closed` or `Crashed`, after which every
    /// event of its target is late.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn terminal(&self) {
        let mut state = self.shared.state.subscribe();
        let _ = state
            .wait_for(|state| matches!(state, PageState::Closed | PageState::Crashed))
            .await;
    }

    /// Runs `work`, a call on the page that its table names `event`, once
    /// the table serves it: it answers what `work` answers, or the code
    /// of what ended the page under it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops `work`.
    pub(crate) async fn call<T>(
        &self,
        event: PageEvent,
        work: impl Future<Output = Result<T, CallError>>,
    ) -> Result<T, CallError> {
        let _in_flight = InFlight::new(&self.shared.calls);
        self.admit(event).await?;
        let mut ended = self.shared.ended.subscribe();
        let mut work = std::pin::pin!(work);
        tokio::select! {
            biased;
            answered = &mut work => match answered {
                Ok(answer) => Ok(answer),
                Err(error) => Err(self.withheld(self.settled(error).await)),
            },
            Some(error) = ending(&mut ended) => Err(error),
        }
    }

    /// Waits until the page's table serves `event`.
    async fn admit(&self, event: PageEvent) -> Result<(), CallError> {
        let mut state = self.shared.state.subscribe();
        loop {
            let now = *state.borrow_and_update();
            match lifecycle::page(now, event) {
                Ok((_, PageEffect::Serve)) => return Ok(()),
                Ok((_, PageEffect::Wait)) => {
                    let _ = state.changed().await;
                }
                Ok((
                    _,
                    effect @ (PageEffect::Nothing
                    | PageEffect::Commit
                    | PageEffect::FailNavigation
                    | PageEffect::Interrupt
                    | PageEffect::Judge
                    | PageEffect::Defer
                    | PageEffect::Pause
                    | PageEffect::EndPause
                    | PageEffect::Close
                    | PageEffect::ClosedInBrowser
                    | PageEffect::Quit
                    | PageEffect::RendererCrash
                    | PageEffect::BrowserCrash
                    | PageEffect::Confirm
                    | PageEffect::Release),
                )) => {
                    return Err(invalid_transition(
                        "page",
                        &format!("{now:?}"),
                        &format!("{event:?} (effect {effect:?})"),
                    ));
                }
                Err(invalid) => return Err(self.refused(invalid)),
            }
        }
    }

    /// What a call the table refused raises.
    fn refused(&self, invalid: InvalidTransition<PageState, PageEvent>) -> CallError {
        match invalid.refusal {
            Refusal::Program(code) if code == CLOSED.name => self
                .shared
                .ended
                .borrow()
                .clone()
                .filter(|ended| ended.code() == PAGE_MEMORY_EXCEEDED.name)
                .unwrap_or_else(|| page_closed(&self.shared.page_id)),
            Refusal::Program(code) if code == PAGE_CRASHED.name => self
                .shared
                .ended
                .borrow()
                .clone()
                .unwrap_or_else(renderer_crashed),
            Refusal::Program(_) | Refusal::Internal => invalid_transition(
                "page",
                &format!("{:?}", invalid.state),
                &format!("{:?}", invalid.event),
            ),
        }
    }

    /// The code a call that failed with `error` raises: what ended its page,
    /// when the failure was the page going and its end is known in time.
    async fn settled(&self, error: CallError) -> CallError {
        if error.code() != PAGE_CRASHED.name {
            return error;
        }
        self.ending_or(error).await
    }

    /// What ended the page, once it is known in time, else `error`: the
    /// code a call raises that learnt otherwise that its page went, such as
    /// a wait whose page's events ended.
    pub(crate) async fn ending_or(&self, error: CallError) -> CallError {
        let mut ended = self.shared.ended.subscribe();
        match tokio::time::timeout(ENDING_WAIT, ending(&mut ended)).await {
            Ok(Some(ending)) => ending,
            Ok(None) | Err(_) => error,
        }
    }

    /// Applies `ending` to the page: its calls in flight raise what its
    /// effect names. Answers the effect, or `internal` for a pair only a bug
    /// sends.
    pub(crate) fn end(&self, ending: &Ending) -> Result<PageEffect, CallError> {
        let event = ending.event();
        let mut applied = None;
        self.shared.state.send_if_modified(|state| {
            let outcome = lifecycle::page(*state, event);
            let changed = match outcome {
                Ok((next, _)) if next != *state => {
                    *state = next;
                    true
                }
                Ok(_) | Err(_) => false,
            };
            applied = Some(outcome);
            changed
        });
        let effect = match applied.expect("invariant: the table was asked") {
            Ok((_, effect)) => effect,
            Err(invalid) => return Err(self.refused(invalid)),
        };
        let raised = match (effect, ending) {
            (PageEffect::Close, Ending::Closed(closed)) => {
                Some(cancelled(StoppedBy::Program, *closed))
            }
            (PageEffect::Close, Ending::LeaseEnded(withheld)) => Some(withheld.clone()),
            (PageEffect::Close, Ending::MemoryExceeded { memory, limit }) => Some(
                page_memory_exceeded(&self.shared.page_id, *memory, limit.clone()),
            ),
            (PageEffect::Close, _) => Some(cancelled(StoppedBy::Program, ClosedValue::Page)),
            (PageEffect::ClosedInBrowser, _) => {
                Some(cancelled(StoppedBy::Browser, ClosedValue::Page))
            }
            (PageEffect::Quit, _) => Some(cancelled(StoppedBy::Browser, ClosedValue::Browser)),
            (PageEffect::RendererCrash, _) => Some(renderer_crashed()),
            (PageEffect::BrowserCrash, Ending::BrowserCrash(exit)) => {
                Some(browser_crashed(exit.clone()))
            }
            (PageEffect::BrowserCrash, _) => Some(browser_crashed(None)),
            (
                PageEffect::Nothing
                | PageEffect::Serve
                | PageEffect::Wait
                | PageEffect::Commit
                | PageEffect::FailNavigation
                | PageEffect::Interrupt
                | PageEffect::Judge
                | PageEffect::Defer
                | PageEffect::Pause
                | PageEffect::EndPause
                | PageEffect::Confirm
                | PageEffect::Release,
                _,
            ) => None,
        };
        if let Some(raised) = raised {
            self.shared.ended.send_replace(Some(raised));
        }
        Ok(effect)
    }
}

/// A call on a page in flight, counted until the value drops.
struct InFlight<'a>(&'a watch::Sender<usize>);

impl<'a> InFlight<'a> {
    fn new(calls: &'a watch::Sender<usize>) -> Self {
        calls.send_modify(|calls| *calls += 1);
        Self(calls)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|calls| *calls -= 1);
    }
}

/// What its page's end raises, once `ended` holds it; none when the page's
/// state went first.
async fn ending(ended: &mut watch::Receiver<Option<CallError>>) -> Option<CallError> {
    ended
        .wait_for(Option::is_some)
        .await
        .ok()
        .and_then(|raised| raised.clone())
}

/// `closed` for the page `page`.
pub(crate) fn page_closed(page: &PageId) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the page `{}` is closed, or its Run ended", page.0),
        &json!({ "page_id": page }),
    )
}

/// `closed` for the Session `session`.
pub(crate) fn session_closed(session: &SessionId) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the Session `{}` is closed, or its Run ended", session.0),
        &json!({ "session_id": session }),
    )
}

/// One Session's state. Clones share it.
#[derive(Clone)]
pub(crate) struct SessionLife {
    shared: Arc<SessionShared>,
}

struct SessionShared {
    session: SessionId,
    attributes: Attributes,
    state: watch::Sender<SessionState>,
    /// What its later calls raise once its browser crashed.
    crashed: OnceLock<CallError>,
    /// What the calls that waited on its open raise once the open failed.
    failed: OnceLock<CallError>,
}

impl SessionLife {
    /// The Session `session` of `attributes`, in `state`.
    pub(crate) fn new(session: SessionId, attributes: Attributes, state: SessionState) -> Self {
        Self {
            shared: Arc::new(SessionShared {
                session,
                attributes,
                state: watch::Sender::new(state),
                crashed: OnceLock::new(),
                failed: OnceLock::new(),
            }),
        }
    }

    /// Where the Session is.
    pub(crate) fn state(&self) -> SessionState {
        *self.shared.state.borrow()
    }

    /// Waits until the Session closes or crashes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn ended(&self) {
        let mut state = self.shared.state.subscribe();
        let _ = state
            .wait_for(|state| {
                matches!(
                    state,
                    SessionState::Closing | SessionState::Closed | SessionState::Crashed
                )
            })
            .await;
    }

    /// Waits until the Session's table serves a program call on it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) async fn call(&self) -> Result<(), CallError> {
        let mut state = self.shared.state.subscribe();
        loop {
            let now = *state.borrow_and_update();
            match lifecycle::session(now, SessionEvent::Call, self.shared.attributes) {
                Ok((_, SessionEffect::Serve)) => return Ok(()),
                Ok((_, SessionEffect::Wait)) => {
                    let _ = state.changed().await;
                }
                Ok((
                    _,
                    effect @ (SessionEffect::Nothing
                    | SessionEffect::Opened
                    | SessionEffect::FailOpen
                    | SessionEffect::AbandonOpen
                    | SessionEffect::CloseChildren
                    | SessionEffect::Lend
                    | SessionEffect::EndAttach
                    | SessionEffect::EndAttachAndClose
                    | SessionEffect::Quit
                    | SessionEffect::Crash
                    | SessionEffect::Suspend
                    | SessionEffect::EndAttachAndSuspend
                    | SessionEffect::Unreachable
                    | SessionEffect::Resume
                    | SessionEffect::Reopen
                    | SessionEffect::Release),
                )) => {
                    return Err(invalid_transition(
                        "session",
                        &format!("{now:?}"),
                        &format!("Call (effect {effect:?})"),
                    ));
                }
                Err(invalid) => return Err(self.refused(&invalid)),
            }
        }
    }

    /// What a call the table refused raises.
    fn refused(&self, invalid: &InvalidTransition<SessionState, SessionEvent>) -> CallError {
        match invalid.refusal {
            Refusal::Program(code) if code == CLOSED.name => self
                .shared
                .failed
                .get()
                .cloned()
                .unwrap_or_else(|| session_closed(&self.shared.session)),
            Refusal::Program(code) if code == PAGE_CRASHED.name => self
                .shared
                .crashed
                .get()
                .cloned()
                .unwrap_or_else(|| browser_crashed(None)),
            Refusal::Program(_) | Refusal::Internal => invalid_transition(
                "session",
                &format!("{:?}", invalid.state),
                &format!("{:?}", invalid.event),
            ),
        }
    }

    /// Applies `OpenFailed`: the calls that waited on the open raise
    /// `error`, the open's own code (spec #594's S2 and S20).
    pub(crate) fn open_failed(&self, error: CallError) -> Result<SessionEffect, CallError> {
        if self.state() == SessionState::Opening {
            let _ = self.shared.failed.set(error);
        }
        self.apply(SessionEvent::OpenFailed, None)
    }

    /// Applies `event` to the Session, `exit` saying how its browser exited
    /// when it crashed, and emits its new state as a `tracing` event (ADR
    /// 0060). Answers the effect, or the code of a refused event.
    pub(crate) fn apply(
        &self,
        event: SessionEvent,
        exit: Option<BrowserExit>,
    ) -> Result<SessionEffect, CallError> {
        let mut applied = None;
        self.shared.state.send_if_modified(|state| {
            let outcome = lifecycle::session(*state, event, self.shared.attributes);
            let changed = match outcome {
                Ok((next, _)) if next != *state => {
                    tracing::info!(
                        target: "goodfetch::session",
                        session_id = %self.shared.session.0,
                        from = ?*state,
                        to = ?next,
                        ?event,
                        "a Session changed state"
                    );
                    *state = next;
                    true
                }
                Ok(_) | Err(_) => false,
            };
            applied = Some(outcome);
            changed
        });
        match applied.expect("invariant: the table was asked") {
            Ok((_, SessionEffect::Crash)) => {
                let _ = self.shared.crashed.set(browser_crashed(exit));
                Ok(SessionEffect::Crash)
            }
            Ok((_, effect)) => Ok(effect),
            Err(invalid) => Err(self.refused(&invalid)),
        }
    }
}

#[cfg(test)]
mod tests;
