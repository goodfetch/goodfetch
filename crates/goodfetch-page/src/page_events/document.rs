//! The page's document and the waits on it (spec #577's `waits`; ADRs 0080
//! and 0121): the main frame's URL, which a committed navigation and a
//! change within the document, a `pushState`, a `replaceState` or a hash
//! change, each set, and the load states its document reached. `networkidle`
//! comes once the document fired `load` and no request of the page was in
//! flight for [`NETWORK_IDLE`] since: a WebSocket's and a server-sent-event
//! stream's connections are no requests in flight, nor is a `data:` or
//! `blob:` URL, and a service worker's own requests reach its own target,
//! never the page's. A URL waiter is armed when its call arrives, and
//! answered by the change that makes the URL match.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::network::{self, LoaderId, ResourceType};
use goodfetch_cdp::generated::page;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::waits::{LoadState, Reached};
use goodfetch_protocol::settings::TIMEOUT_WAIT;
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use super::{PageEvents, Shared, State, gone, lock};
use crate::url_pattern::UrlPattern;

/// How long no request of the page may be in flight, once its document
/// fired `load`, before `networkidle` comes.
pub(super) const NETWORK_IDLE: Duration = Duration::from_millis(500);

/// The URL of a page the browser just opened.
const BLANK: &str = "about:blank";

/// The main frame's document: the load states it reached.
#[derive(Clone, Debug)]
pub(super) struct Document {
    /// How many documents the main frame committed since the page opened.
    generation: u64,
    /// The browser's ID of its load; none for the page's first, blank one.
    loader: Option<LoaderId>,
    dom_content_loaded: bool,
    load: bool,
    network_idle: bool,
    /// Since when no request `networkidle` counts was in flight, once the
    /// document fired `load`.
    quiet_since: Option<Instant>,
}

impl Document {
    /// The page's first document, `about:blank`, loaded and idle.
    pub(super) fn blank() -> Self {
        Self {
            generation: 0,
            loader: None,
            dom_content_loaded: true,
            load: true,
            network_idle: true,
            quiet_since: None,
        }
    }

    /// Whether it reached `state`.
    pub(super) fn reached(&self, state: LoadState) -> bool {
        match state {
            LoadState::DomContentLoaded => self.dom_content_loaded,
            LoadState::Load => self.load,
            LoadState::NetworkIdle => self.network_idle,
        }
    }

    /// Its count among the main frame's documents.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// The browser's ID of its load, which is its document request's; none
    /// for the page's first, blank one.
    pub(super) fn loader(&self) -> Option<&LoaderId> {
        self.loader.as_ref()
    }
}

/// Where a URL waiter's URL came.
#[derive(Clone, Copy, Debug)]
pub(super) enum UrlReached {
    /// In the document the page shows, which a `pushState`, a
    /// `replaceState` or a hash change moved there.
    Here,
    /// In the main frame's document of this generation: just committed,
    /// or the page's own when the waiter was armed.
    Committed(u64),
}

/// A `page.wait_for_url` call's waiter.
pub(super) struct UrlWaiter {
    pub(super) pattern: UrlPattern,
    pub(super) answer: oneshot::Sender<UrlReached>,
}

/// What the page's documents keep of its requests: those `networkidle`
/// counts in flight, by the browser's ID, with the load they belong to.
#[derive(Default)]
pub(super) struct InFlight(BTreeMap<String, LoaderId>);

/// What a wait for `state` waits for, as `deadline_exceeded` names it.
fn waited_for(state: LoadState) -> WaitedFor {
    match state {
        LoadState::DomContentLoaded => WaitedFor::DOM_CONTENT_LOADED,
        LoadState::Load => WaitedFor::LOAD,
        LoadState::NetworkIdle => WaitedFor::NETWORK_IDLE,
    }
}

/// Whether `networkidle` counts the request `sent` while it is in flight.
fn counted(sent: &network::RequestWillBeSentEvent) -> bool {
    !matches!(
        sent.r#type,
        Some(ResourceType::EventSource | ResourceType::WebSocket)
    ) && !sent.request.url.starts_with("data:")
        && !sent.request.url.starts_with("blob:")
}

/// A request sent: in flight until it ends, when `networkidle` counts it.
pub(super) fn sent(shared: &Shared, state: &mut State, sent: &network::RequestWillBeSentEvent) {
    if counted(sent) {
        state
            .in_flight
            .0
            .insert(sent.request_id.clone(), sent.loader_id.clone());
        shared.document.send_if_modified(|document| {
            let was = document.quiet_since.take();
            was.is_some()
        });
    }
}

/// A request ended, finished or failed: once none is left in flight after
/// `load`, the page is quiet from now.
pub(super) fn ended(shared: &Shared, state: &mut State, request_id: &str) {
    if state.in_flight.0.remove(request_id).is_some() && state.in_flight.0.is_empty() {
        let now = shared.clock.now();
        shared.document.send_if_modified(|document| {
            if document.load && document.quiet_since.is_none() {
                document.quiet_since = Some(now);
                return true;
            }
            false
        });
    }
}

/// An event of the main frame's document: a navigation committed, one
/// within the document, or a lifecycle event of its document. Answers the
/// URL waiters whose URL came.
pub(super) fn take_in(shared: &Shared, state: &mut State, event: &goodfetch_cdp::RawEvent) {
    let main = shared.driver.main_frame();
    if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
        if &navigated.frame.id != main || navigated.frame.parent_id.is_some() {
            return;
        }
        let frame = navigated.frame;
        state.url = format!(
            "{}{}",
            frame.url,
            frame.url_fragment.as_deref().unwrap_or_default()
        );
        // The requests of the document it replaced no longer count.
        state
            .in_flight
            .0
            .retain(|_, loader| *loader == frame.loader_id);
        let blank = state.url == BLANK;
        // A blank document may report no lifecycle: it has loaded, and is
        // quiet from now when nothing is in flight.
        let quiet_since = (blank && state.in_flight.0.is_empty()).then(|| shared.clock.now());
        let mut generation = 0;
        shared.document.send_modify(|document| {
            document.generation += 1;
            generation = document.generation;
            document.loader = Some(frame.loader_id.clone());
            document.dom_content_loaded = blank;
            document.load = blank;
            document.network_idle = false;
            document.quiet_since = quiet_since;
        });
        answer_url_waiters(state, UrlReached::Committed(generation));
    } else if let Some(within) = event.read::<page::NavigatedWithinDocumentEvent>() {
        if &within.frame_id == main {
            state.url = within.url;
            answer_url_waiters(state, UrlReached::Here);
        }
    } else if let Some(lifecycle) = event.read::<page::LifecycleEventEvent>()
        && &lifecycle.frame_id == main
    {
        let quiet = state.in_flight.0.is_empty();
        let now = shared.clock.now();
        shared.document.send_if_modified(|document| {
            if document.loader.as_ref() != Some(&lifecycle.loader_id) {
                return false;
            }
            match lifecycle.name.as_str() {
                "DOMContentLoaded" if !document.dom_content_loaded => {
                    document.dom_content_loaded = true;
                    true
                }
                "load" if !document.load => {
                    document.load = true;
                    if quiet {
                        document.quiet_since = Some(now);
                    }
                    true
                }
                _ => false,
            }
        });
    }
}

/// Answers each URL waiter whose pattern matches the page's URL now.
fn answer_url_waiters(state: &mut State, reached: UrlReached) {
    let url = state.url.clone();
    let waiters = std::mem::take(&mut state.url_waiters);
    state.url_waiters = waiters
        .into_iter()
        .filter_map(|waiter| {
            if waiter.answer.is_closed() {
                None
            } else if waiter.pattern.matches(&url) {
                let _ = waiter.answer.send(reached);
                None
            } else {
                Some(waiter)
            }
        })
        .collect();
}

/// Marks the page's document `networkidle` once it stayed quiet for
/// [`NETWORK_IDLE`] after `load`, until the page goes.
pub(super) async fn watch_idle(shared: Arc<Shared>) {
    let mut document = shared.document.subscribe();
    let mut gone = shared.gone.subscribe();
    loop {
        let quiet = {
            let now = document.borrow_and_update();
            now.quiet_since
                .filter(|_| now.load && !now.network_idle)
                .map(|since| (since, now.generation))
        };
        let went = async {
            let _ = gone.wait_for(|gone| *gone).await;
        };
        match quiet {
            Some((since, generation)) => {
                tokio::select! {
                    () = shared.clock.sleep_until(since + NETWORK_IDLE) => {
                        shared.document.send_if_modified(|document| {
                            let idle = document.generation == generation
                                && document.quiet_since == Some(since)
                                && !document.network_idle;
                            document.network_idle |= idle;
                            idle
                        });
                    }
                    changed = document.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    () = went => return,
                }
            }
            None => {
                tokio::select! {
                    changed = document.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    () = went => return,
                }
            }
        }
    }
}

impl PageEvents {
    /// Arms a waiter until the page's URL matches `pattern`: at once when it
    /// already does, else once a navigation commits a document at a URL it
    /// matches, or one within the document moves there. A document the
    /// navigation committed, or the page's own when it already matched, is
    /// then waited for until it reaches `wait_until`. It ends at
    /// `deadline`. A main-frame navigation to a URL it matches that the
    /// navigation gate refuses, with no navigation call waiting on it,
    /// raises its refusal from the waiter instead.
    pub(crate) fn wait_for_url(
        &self,
        pattern: UrlPattern,
        wait_until: LoadState,
        deadline: WaitDeadline,
    ) -> impl Future<Output = Result<Reached, CallError>> + use<> {
        let (answer, answered) = oneshot::channel();
        let mut document = self.shared.document.subscribe();
        let matching = pattern.clone();
        let refused = self
            .shared
            .refusals
            .refused(move |url| matching.matches(url));
        {
            let mut state = lock(&self.shared.state);
            if pattern.matches(&state.url) {
                let _ = answer.send(UrlReached::Committed(document.borrow().generation()));
            } else {
                // Those whose call went go now, not at the next URL change.
                state
                    .url_waiters
                    .retain(|waiter| !waiter.answer.is_closed());
                state.url_waiters.push(UrlWaiter { pattern, answer });
            }
        }
        let mut gone_now = self.shared.gone.subscribe();
        let shared = Arc::clone(&self.shared);
        async move {
            let reached = async move {
                match answered.await {
                    Err(_) => false,
                    Ok(UrlReached::Here) => true,
                    Ok(UrlReached::Committed(generation)) => {
                        until_gone(&mut gone_now, async {
                            document
                                .wait_for(|now| {
                                    now.generation() != generation || now.reached(wait_until)
                                })
                                .await
                                .is_ok()
                        })
                        .await
                    }
                }
            };
            let outcome = async move {
                tokio::select! {
                    reached = reached => Ok(reached),
                    refused = refused => Err(refused),
                }
            };
            match deadline.within(WaitedFor::URL, outcome).await? {
                Ok(true) => Ok(Reached {}),
                Ok(false) => Err(gone(&shared)),
                Err(refused) => Err(refused),
            }
        }
    }

    /// Waits until the page's document reached `state`: at once when it
    /// already did when the call arrived, else until its document, or the
    /// one a navigation commits meanwhile, reaches it, unless `deadline`
    /// passes first.
    pub(crate) fn wait_for_load_state(
        &self,
        state: LoadState,
        deadline: WaitDeadline,
    ) -> impl Future<Output = Result<Reached, CallError>> + use<> {
        let mut document = self.shared.document.subscribe();
        let already = document.borrow().reached(state);
        let mut gone_now = self.shared.gone.subscribe();
        let shared = Arc::clone(&self.shared);
        async move {
            if already {
                return Ok(Reached {});
            }
            let reached = until_gone(&mut gone_now, async {
                document.wait_for(|now| now.reached(state)).await.is_ok()
            });
            if deadline.within(waited_for(state), reached).await? {
                Ok(Reached {})
            } else {
                Err(gone(&shared))
            }
        }
    }

    /// The deadline of an app-state wait whose call's own `timeout` is
    /// `timeout` (ADR 0080): the Run's `timeout.wait` when absent, none
    /// when `null`, else the call's, which is that setting at the run layer.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a `timeout` of 0, as `Deadline::argument`
    /// says.
    #[expect(
        clippy::option_option,
        reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
    )]
    pub(crate) fn wait_deadline(
        &self,
        timeout: Option<Option<u64>>,
    ) -> Result<WaitDeadline, CallError> {
        let deadline = match timeout {
            None => Some(self.shared.wait.clone()),
            Some(None) => None,
            Some(Some(millis)) => Some(Deadline::argument(&TIMEOUT_WAIT, millis)?),
        };
        Ok(WaitDeadline {
            deadline,
            spent: Duration::ZERO,
            clock: self.shared.clock,
        })
    }
}

/// An app-state wait's deadline, if it has one, on the Run's clock.
#[derive(Clone)]
pub(crate) struct WaitDeadline {
    deadline: Option<Deadline>,
    /// What the wait's call spent of it before the wait began.
    spent: Duration,
    clock: Clock,
}

impl WaitDeadline {
    /// The deadline, if the wait has one.
    pub(crate) fn deadline(&self) -> Option<&Deadline> {
        self.deadline.as_ref()
    }

    /// This deadline for a wait whose call spent `spent` of it before the
    /// wait began, waiting for its page's `Runtime` (ADR 0019).
    pub(crate) fn having_spent(self, spent: Duration) -> Self {
        Self { spent, ..self }
    }

    /// The Run's clock it is counted on.
    pub(crate) fn clock(&self) -> &Clock {
        &self.clock
    }

    /// Waits for `work` until the deadline, if any, passes, then ends it
    /// with `deadline_exceeded`, which says it waited for `waited_for`.
    ///
    /// # Errors
    ///
    /// `deadline_exceeded` once the deadline passed first.
    ///
    /// # Cancel safety
    ///
    /// As cancel-safe as `work`.
    pub(crate) async fn within<T>(
        &self,
        waited_for: WaitedFor,
        work: impl Future<Output = T>,
    ) -> Result<T, CallError> {
        let Some(deadline) = &self.deadline else {
            return Ok(work.await);
        };
        let started = self.clock.now();
        let end = started + deadline.duration().saturating_sub(self.spent);
        tokio::select! {
            biased;
            answer = work => Ok(answer),
            () = self.clock.sleep_until(end) => {
                Err(deadline.exceeded(waited_for, self.clock.now() - started + self.spent))
            }
        }
    }
}

/// What `work` answers, or `false` once the page went first.
async fn until_gone(gone: &mut watch::Receiver<bool>, work: impl Future<Output = bool>) -> bool {
    tokio::select! {
        done = work => done,
        _ = gone.wait_for(|gone| *gone) => false,
    }
}
