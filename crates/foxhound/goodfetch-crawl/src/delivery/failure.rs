//! Why an attempt failed (ADR 0127): the crawl's own typed failure, made
//! where a failure enters the crawl and nowhere else. One comes from the
//! fetch tier's [`FetchFailure`], whose transport failure says what the
//! hop met ([`AttemptFailure::of_fetch`]); the other from an error object
//! as it enters, a handler's answer or an engine call's error, read once
//! ([`AttemptFailure::entered`]). The retry rule, Politeness and the
//! Session's score match on [`Why`]; none reads the error's details back.
//! The fields of the protocol's error object ride along unread, for the
//! outcome that writes them.

use std::time::SystemTime;

use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::{CallError, retry_after};
use goodfetch_fetch_tier::{FetchFailure, TransportFailure};
use goodfetch_frontier::backoff_classes::{Class, Met, class};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::connection::DEADLINE_EXCEEDED;
use goodfetch_protocol::foxhound::crawl::{HANDLER_FAILED, RESPONSE_UNAVAILABLE};
use goodfetch_protocol::hook::HOOK_FAILED;
use goodfetch_protocol::page::proxy::{PROXY_AUTH_FAILED, PROXY_UNAVAILABLE};
use goodfetch_protocol::page::{BLOCKED, NAVIGATION_FAILED, PAGE_CHECK_FAILED};
use goodfetch_protocol::run::RUN_STATE_UNAVAILABLE;
use goodfetch_protocol::settings::{TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use serde_json::{Map, Value};

use crate::policy::CrawlPolicy;
use crate::work_items::{Failure, Unsuccessful};

/// The detail naming the time a site's `Retry-After` asked for, as the
/// engine's `blocked` spells it on the wire (ADRs 0038 and 0118).
pub(super) const RETRY_AFTER: &str = "retry_after";

/// Why an attempt failed, as far as anything acts on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    /// The site refused or timed out the connection, or reset it with
    /// HTTP/2's `ENHANCE_YOUR_CALM`: its domain backs off (ADRs 0086 and
    /// 0118).
    Unreachable,
    /// The site did not answer in time: the response's first byte or the
    /// navigation passed its deadline.
    TimedOut,
    /// The site blocked the request, for this cause.
    Blocked(BlockCause),
    /// The site was failing for now, with this status when it answered
    /// one (`response_unavailable`).
    Unavailable(Option<u16>),
    /// The page failed its Page check; `noindex` holds the page's URL
    /// and status when it was marked `noindex`, a decoy (ADR 0035).
    PageCheckFailed { noindex: Option<(String, u16)> },
    /// The proxy's own failure: it says nothing of the site's domain.
    Proxy,
    /// The handler's own failure (`handler_failed`), retried only as
    /// `crawl.retry.handler_failed` says.
    Handler,
    /// A Hook in code that raised the program's own exception
    /// (`hook_failed` wrapping no code), retried only as
    /// `crawl.retry.handler_failed` says (ADR 0104).
    Hook,
    /// The run file refused the write of the attempt's outcome: the
    /// attempt is retried as one that failed (issue 2320).
    OutcomeNotWritten,
    /// The engine panicked in the attempt (ADR 0148): retried only as
    /// `crawl.retry.engine_panicked` says; its domain does not back off.
    Panicked,
    /// Any other failure: nothing but its `retryable` acts on it.
    Other,
}

/// An attempt's failure: why, and the error its outcome is written with.
#[derive(Clone, Debug)]
pub(crate) struct AttemptFailure {
    why: Why,
    retry_after: Option<SystemTime>,
    pub(crate) code: String,
    pub(crate) retryable: bool,
    pub(crate) message: String,
    pub(crate) details: Map<String, Value>,
}

impl AttemptFailure {
    /// The failure of an error object entering the crawl, a handler's
    /// answer or an engine call's error: its details are read here, once.
    pub(crate) fn entered(
        code: String,
        retryable: bool,
        message: String,
        details: Map<String, Value>,
    ) -> Self {
        let why = read(&code, &details);
        let retry_after = details.get(RETRY_AFTER).and_then(retry_after::of_detail);
        Self {
            why,
            retry_after,
            code,
            retryable,
            message,
            details,
        }
    }

    /// A failure the crawl itself names, with no details.
    pub(crate) fn named(code: &str, retryable: bool, message: String) -> Self {
        Self::entered(code.to_owned(), retryable, message, Map::new())
    }

    /// The failure of an attempt whose outcome the run file did not
    /// write, for `cause`: retryable, since the attempt's work is lost.
    pub(crate) fn outcome_not_written(cause: &str) -> Self {
        Self {
            why: Why::OutcomeNotWritten,
            retry_after: None,
            code: RUN_STATE_UNAVAILABLE.name.to_owned(),
            retryable: true,
            message: format!("The attempt's outcome was not written: {cause}."),
            details: Map::new(),
        }
    }

    /// The failure of an attempt the engine's panic ended, as `error`,
    /// its `engine_panicked`, tells it (ADR 0148).
    pub(crate) fn panicked(error: &CallError) -> Self {
        Self {
            why: Why::Panicked,
            retry_after: None,
            code: error.code().to_owned(),
            retryable: error.retryable(),
            message: error.message().to_owned(),
            details: error.details().cloned().unwrap_or_default(),
        }
    }

    /// The failure of a fetch: a transport's failure is classed by what
    /// its hop met, never by its error's text; a browser's own failure
    /// and any other are the error the fetch tier ended with.
    pub(crate) fn of_fetch(failure: &FetchFailure) -> Self {
        let error = failure.error();
        let entered = Self::entered(
            error.code().to_owned(),
            error.retryable(),
            error.message().to_owned(),
            error.details().cloned().unwrap_or_default(),
        );
        let transport = match failure.transport() {
            // The browser's own error is an error object entering the
            // crawl, classed as one: its transport's failure names no
            // network error of its own.
            None | Some(TransportFailure::Browser(_)) => return entered,
            Some(transport) => transport,
        };
        let why = match class(transport.met()) {
            Class::BackOff => Why::Unreachable,
            Class::ProxyRefusal => Why::Proxy,
            Class::TooManyRequests | Class::Done => match entered.why {
                // The codes a transport's failure ends with that say more
                // than its class: a proxy's own, never a site's.
                Why::Proxy => Why::Proxy,
                _ => Why::Other,
            },
        };
        Self { why, ..entered }
    }

    /// Why it failed.
    pub(crate) fn why(&self) -> &Why {
        &self.why
    }

    /// The time its site asked for no request before, when it named one.
    pub(crate) fn retry_after(&self) -> Option<SystemTime> {
        self.retry_after
    }

    /// The cause its site blocked the request for, when it did.
    pub(crate) fn block_cause(&self) -> Option<BlockCause> {
        match self.why {
            Why::Blocked(cause) => Some(cause),
            _ => None,
        }
    }
}

/// Why an error object with `code` and `details` says its attempt failed.
fn read(code: &str, details: &Map<String, Value>) -> Why {
    let text = |name: &str| details.get(name).and_then(Value::as_str);
    let status = || {
        details
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
    };
    if code == BLOCKED.name {
        return details
            .get("cause")
            .and_then(|cause| serde_json::from_value(cause.clone()).ok())
            .map_or(Why::Other, Why::Blocked);
    }
    if code == RESPONSE_UNAVAILABLE.name {
        return Why::Unavailable(status());
    }
    if code == NAVIGATION_FAILED.name {
        let backs_off =
            text("error").is_some_and(|error| class(Met::Failure { error }) == Class::BackOff);
        return if backs_off {
            Why::Unreachable
        } else {
            Why::Other
        };
    }
    if code == DEADLINE_EXCEEDED.name {
        let waited_for_site = details
            .get("limit")
            .and_then(|limit| limit.get("key"))
            .and_then(Value::as_str)
            .is_some_and(|key| [TIMEOUT_FIRST_BYTE.name, TIMEOUT_NAVIGATION.name].contains(&key));
        return if waited_for_site {
            Why::TimedOut
        } else {
            Why::Other
        };
    }
    if code == PAGE_CHECK_FAILED.name {
        let noindex = details.get("noindex").and_then(Value::as_bool) == Some(true);
        return Why::PageCheckFailed {
            noindex: noindex.then(|| {
                (
                    text("url").unwrap_or_default().to_owned(),
                    status().unwrap_or_default(),
                )
            }),
        };
    }
    if code == PROXY_AUTH_FAILED.name || code == PROXY_UNAVAILABLE.name {
        return Why::Proxy;
    }
    if code == HANDLER_FAILED.name {
        return Why::Handler;
    }
    // A wrapped code is retried as its own retryable flag says.
    if code == HOOK_FAILED.name && details.get("wrapped_code").is_none_or(Value::is_null) {
        return Why::Hook;
    }
    Why::Other
}

/// A handler's failure as the Run's state keeps it: no value of `masks`,
/// those the Run resolved, stays in its message or details (spec #564).
pub(super) fn kept_failure(
    masks: &Masks,
    code: String,
    message: &str,
    details: Map<String, Value>,
) -> Failure {
    let mut kept = Value::Object(details);
    masks.apply_value(&mut kept, Scope::Kept);
    let Value::Object(details) = kept else {
        unreachable!("invariant: masking an object leaves an object")
    };
    Failure {
        code,
        message: masks.apply(message, Scope::Kept).into_owned(),
        details,
    }
}

/// How `failure`, which failed for `why`, on a work item's `attempt`th
/// attempt since its latest admission ends it under `policy`: retried
/// while it is `retryable` and retries are left, failed otherwise; a
/// handler's own failure, or a Hook's own exception, is retryable only as
/// `crawl.retry.handler_failed` says (ADR 0104), and one the engine's panic ended only as
/// `crawl.retry.engine_panicked` says (ADR 0148).
pub(super) fn retried_or_failed(
    policy: CrawlPolicy,
    attempt: u32,
    failure: Failure,
    why: &Why,
    retryable: bool,
) -> Unsuccessful {
    let retryable = match why {
        Why::Handler | Why::Hook => policy.retry_handler_failed,
        Why::Panicked => policy.retry_engine_panicked,
        _ => retryable,
    };
    if retryable && policy.may_retry(attempt) {
        Unsuccessful::Retried(failure)
    } else {
        Unsuccessful::Failed(failure)
    }
}

#[cfg(test)]
mod tests;
