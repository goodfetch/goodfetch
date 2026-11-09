//! The waits on a page's requests and responses (ADR 0080): each armed
//! when its call arrives and answered by the next request sent, or the
//! next response received, a redirect's included, whose URL its pattern
//! matches, any when none, unless its deadline passes first.

use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::events::{PageExchange, PageRequest};
use tokio::sync::oneshot;

use super::{PageEvents, State, WaitDeadline, gone, lock};
use crate::url_pattern::UrlPattern;

/// A request's or a response's waiter, which the next one whose URL its
/// pattern matches, any when none, answers.
pub(super) struct Armed<T> {
    pattern: Option<UrlPattern>,
    answer: oneshot::Sender<T>,
}

/// Answers each waiter of `waiters` whose pattern matches `url` with
/// `value`, and lets go of those whose call went.
pub(super) fn answer_armed<T: Clone>(waiters: &mut Vec<Armed<T>>, url: &str, value: &T) {
    *waiters = std::mem::take(waiters)
        .into_iter()
        .filter_map(|waiter| {
            if waiter.answer.is_closed() {
                None
            } else if waiter
                .pattern
                .as_ref()
                .is_none_or(|pattern| pattern.matches(url))
            {
                let _ = waiter.answer.send(value.clone());
                None
            } else {
                Some(waiter)
            }
        })
        .collect();
}

impl PageEvents {
    /// Arms a waiter for the next request the page sends whose URL
    /// `pattern` matches, any when none, and answers it, unless `deadline`
    /// passes first.
    pub(crate) fn wait_for_request(
        &self,
        pattern: Option<UrlPattern>,
        deadline: WaitDeadline,
    ) -> impl Future<Output = Result<PageRequest, CallError>> + use<> {
        self.arm(
            |state| &mut state.request_waiters,
            pattern,
            (deadline, WaitedFor::REQUEST),
        )
    }

    /// Arms a waiter for the next response, a redirect's included, to a
    /// request of the page whose URL `pattern` matches, any when none, and
    /// answers it with its request, unless `deadline` passes first. A
    /// main-frame navigation to such a URL that the navigation gate
    /// refuses, with no navigation call waiting on it, raises its refusal
    /// from the waiter instead.
    pub(crate) fn wait_for_response(
        &self,
        pattern: Option<UrlPattern>,
        deadline: WaitDeadline,
    ) -> impl Future<Output = Result<PageExchange, CallError>> + use<> {
        let matching = pattern.clone();
        let refused = self
            .shared
            .refusals
            .refused(move |url| matching.as_ref().is_none_or(|pattern| pattern.matches(url)));
        let responded = self.arm(
            |state| &mut state.response_waiters,
            pattern,
            (deadline, WaitedFor::RESPONSE),
        );
        async move {
            tokio::select! {
                responded = responded => responded,
                refused = refused => Err(refused),
            }
        }
    }

    /// Arms a waiter in `waiters` under its deadline, which names what it
    /// waited for when it passes.
    fn arm<T: Send + 'static>(
        &self,
        waiters: fn(&mut State) -> &mut Vec<Armed<T>>,
        pattern: Option<UrlPattern>,
        (deadline, waited_for): (WaitDeadline, WaitedFor),
    ) -> impl Future<Output = Result<T, CallError>> + use<T> {
        let (answer, answered) = oneshot::channel();
        {
            let mut state = lock(&self.shared.state);
            let waiters = waiters(&mut state);
            // Those whose call went, at its deadline or cancelled, go now,
            // not at the next request or response.
            waiters.retain(|waiter| !waiter.answer.is_closed());
            waiters.push(Armed { pattern, answer });
        }
        let shared = Arc::clone(&self.shared);
        async move {
            deadline
                .within(waited_for, answered)
                .await?
                .map_err(|_| gone(&shared))
        }
    }
}
