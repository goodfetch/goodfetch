//! What each CDP failure raises (ADR 0127): the CDP client types every
//! error and disconnection of the browser's as a [`Failure`]; this module
//! is where each variant becomes the Error code of the call that met it.
//! Any failure but a deadline while a launched browser comes up is
//! `browser_launch_failed`, naming how the browser ended when it did. A
//! lost browser or page is `page_crashed`, with how the browser ended
//! when it is the browser that was lost; a command the browser leaves
//! unanswered past the call's deadline is `deadline_exceeded` naming the
//! call's timeout setting (ADR 0080); any other failure is the call's
//! generic code: `navigation_failed` for a navigation and for reading the
//! history it goes through, as ADR 0085 names it, `browser_launch_failed`
//! for opening a page, `page_crashed` for reading a live page's document,
//! which only a broken renderer refuses, and an internal error for a
//! locator action or a page function, which have no generic code.

use goodfetch_cdp::{BROWSER_REDIRECT_CHAIN, CertificateError, Failure, NetError, Refusal};
use goodfetch_engine::CallError;
use goodfetch_engine::deadline::Deadline;
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CallCancelledDetails, ClosedValue, INTERNAL, InternalDetails, StoppedBy,
};
use goodfetch_protocol::governor::{PAGE_MEMORY_EXCEEDED, PageMemoryExceededDetails};
use goodfetch_protocol::jsonrpc::{ErrorData, ProtocolFailure};
use goodfetch_protocol::page::{
    BROWSER_LAUNCH_FAILED, BrowserExit, BrowserLaunchFailedDetails, NAVIGATION_FAILED,
    NAVIGATION_INTERRUPTED, NavigationFailedDetails, NavigationInterruptedDetails, PAGE_CRASHED,
    PageCrashedDetails, PageId, RESPONSE_TRUNCATED, RedirectLimit, ResponseTruncatedDetails,
    SITE_PREFERENCES_REFUSED, SitePreferencesRefusedDetails, TOO_MANY_REDIRECTS,
    TooManyRedirectsDetails,
};

use goodfetch_protocol::settings::SettingLimit;
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};

use crate::page_value::Refusal as GateRefusal;

/// The call a failure met.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    /// Bringing up a browser the engine launched: its first answer, the
    /// identity a headless one shows, and the attach to its workers. Holds
    /// the browser's executable.
    Launch {
        /// The browser's executable.
        executable: String,
    },
    /// Opening a page: a browser context, a target and its domains, in a
    /// browser that came up. Holds the browser's executable.
    OpenPage {
        /// The browser's executable.
        executable: String,
    },
    /// Reading the page's document, for its formats.
    Document,
    /// A navigation to `url`.
    Navigation {
        /// Its URL.
        url: String,
    },
    /// Reading the page's history, before a reload or a step back or
    /// forward, whose URL the history names.
    History,
    /// A locator action's commands, the guard's checks and its input, or
    /// a page function's.
    Action {
        /// The operation, such as `locator.click` or `page.evaluate`.
        operation: &'static str,
    },
}

/// What a call whose command the browser left unanswered waited for, as
/// its `deadline_exceeded` names it.
const BROWSER_ANSWER: &str = "browser_answer";

/// The Error code `failure` raises from `call`; `exit` says how the browser
/// ended, when it did, and `deadline` is the call's, which bounded the
/// command. A command the page's lease check withheld raises the error that
/// check gives, which its driver asks it for.
pub(crate) fn call_error(
    call: &Call,
    failure: &Failure,
    exit: Option<BrowserExit>,
    deadline: &Deadline,
) -> CallError {
    // The browser left the command unanswered past the call's own deadline,
    // which bounds each command it sends (ADR 0080).
    if let Failure::TimedOut { .. } = failure {
        return deadline.exceeded(BROWSER_ANSWER, deadline.duration());
    }
    // A browser lost or refusing before it came up never started: it is
    // the launch that failed, however it ended (ADR 0115).
    if let Call::Launch { executable } = call {
        return CallError::new(
            &BROWSER_LAUNCH_FAILED,
            format!("the browser at {executable} did not come up: {failure}"),
            &BrowserLaunchFailedDetails {
                executable: executable.clone(),
                exit,
            },
        );
    }
    let lost = match failure {
        Failure::Disconnected => Some(exit.clone()),
        Failure::Refused {
            refusal: Refusal::TargetGone,
            ..
        } => Some(None),
        _ => None,
    };
    if let Some(browser_exit) = lost {
        let message = match &browser_exit {
            Some(_) => "the page's browser exited while the call ran",
            None => "the page is gone: its renderer or its browser was lost",
        };
        return CallError::new(&PAGE_CRASHED, message, &PageCrashedDetails { browser_exit });
    }
    match call {
        Call::Navigation { url } => CallError::new(
            &NAVIGATION_FAILED,
            format!("the navigation to {url} failed: {failure}"),
            &NavigationFailedDetails {
                url: Some(url.clone()),
                error: None,
            },
        ),
        Call::Document => CallError::new(
            &PAGE_CRASHED,
            format!("the page did not give its document: {failure}"),
            &PageCrashedDetails { browser_exit: None },
        ),
        Call::History => CallError::new(
            &NAVIGATION_FAILED,
            format!(
                "the page's history, which the navigation goes through, did not read: {failure}"
            ),
            &NavigationFailedDetails {
                url: None,
                error: None,
            },
        ),
        Call::Launch { executable } | Call::OpenPage { executable } => CallError::new(
            &BROWSER_LAUNCH_FAILED,
            format!("the browser did not open the page: {failure}"),
            &BrowserLaunchFailedDetails {
                executable: executable.clone(),
                exit,
            },
        ),
        // ADR 0085 names no code of its own for an action's other
        // failures: the browser answered the guard in a way it never does.
        Call::Action { operation } => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the browser failed `{operation}`'s command: {failure}"),
        ),
    }
}

/// `site_preferences_refused` for the navigation or request to `url` that
/// `refusal` refused at the page's navigation gate (ADRs 0086 and 0093).
pub(crate) fn site_preferences_refused(url: &str, refusal: &GateRefusal) -> CallError {
    CallError::new(
        &SITE_PREFERENCES_REFUSED,
        format!("the site's preferences refused {url}: {}", refusal.message),
        &SitePreferencesRefusedDetails {
            url: url.to_owned(),
            reason: refusal.reason,
        },
    )
}

/// `site_preferences_refused` as a failed request carries it, for the
/// request to `url` that `refusal` refused.
pub(crate) fn site_preferences_failure(url: &str, refusal: &GateRefusal) -> ErrorData {
    let error = site_preferences_refused(url, refusal);
    ErrorData {
        code: SITE_PREFERENCES_REFUSED.name.to_owned(),
        retryable: SITE_PREFERENCES_REFUSED.retryable,
        details: error.details().cloned().unwrap_or_default(),
    }
}

/// `page_memory_exceeded` for the page `page_id`, which held `memory` past
/// `limit`, `governor.browser.page_memory_cap`, while memory was critical
/// (ADR 0040).
pub(crate) fn page_memory_exceeded(
    page_id: &PageId,
    memory: u64,
    limit: SettingLimit,
) -> CallError {
    CallError::new(
        &PAGE_MEMORY_EXCEEDED,
        format!(
            "memory was critical and the page `{}` held {memory} bytes, past `governor.browser.page_memory_cap`: it ended",
            page_id.0
        ),
        &PageMemoryExceededDetails {
            page_id: page_id.clone(),
            memory,
            limit,
        },
    )
}

/// `page_crashed` for a page whose renderer crashed, its browser still up.
pub(crate) fn renderer_crashed() -> CallError {
    CallError::new(
        &PAGE_CRASHED,
        "the page's renderer crashed",
        &PageCrashedDetails { browser_exit: None },
    )
}

/// `page_crashed` for a page whose browser's process exited abnormally,
/// `exit` saying how when it was seen.
pub(crate) fn browser_crashed(exit: Option<BrowserExit>) -> CallError {
    CallError::new(
        &PAGE_CRASHED,
        "the page's browser exited abnormally",
        &PageCrashedDetails {
            browser_exit: Some(exit.unwrap_or(BrowserExit {
                code: None,
                signal: None,
            })),
        },
    )
}

/// `call_cancelled` for a call in flight that `by` stopped by closing
/// `closed` under it (ADR 0113).
pub(crate) fn cancelled(by: StoppedBy, closed: ClosedValue) -> CallError {
    let message = match (by, closed) {
        (StoppedBy::Browser, ClosedValue::Page) => {
            "the page's window was closed in the browser, outside every Client: open a page and act again"
        }
        (StoppedBy::Browser, _) => "the page's browser quit, and its Session ended with it",
        (_, ClosedValue::Session) => "the page's Session was closed while the call was in flight",
        (_, _) => "the page was closed while the call was in flight",
    };
    CallError::new(
        &CALL_CANCELLED,
        message,
        &CallCancelledDetails {
            by,
            closed: Some(closed),
        },
    )
}

/// `internal` for an event the table of `object` refused in a state that
/// cannot receive it, which only a bug causes (ADRs 0079 and 0128): logged
/// at `error` with the object, the state and the event.
pub(crate) fn invalid_transition(object: &str, state: &str, event: &str) -> CallError {
    tracing::error!(
        object,
        state,
        event,
        "an invalid transition was refused: only a bug in Goodfetch causes it"
    );
    CallError::new(
        &INTERNAL,
        format!(
            "the {object}'s table refused the event {event} in the state {state}: a bug in Goodfetch, please report it"
        ),
        &InternalDetails {
            object: object.to_owned(),
            state: state.to_owned(),
            event: event.to_owned(),
        },
    )
}

/// `navigation_interrupted` for the navigation to `url`, which another
/// superseded before it committed.
pub(crate) fn interrupted(url: &str) -> CallError {
    CallError::new(
        &NAVIGATION_INTERRUPTED,
        format!("the navigation to {url} was superseded by another before it committed"),
        &NavigationInterruptedDetails {
            url: url.to_owned(),
        },
    )
}

/// `too_many_redirects` for the navigation to `url`, whose request went to
/// each URL of `chain` in turn, the first included.
pub(crate) fn too_many_redirects(url: &str, chain: Vec<String>) -> CallError {
    let mut chain = chain.into_iter();
    let first = chain.next().unwrap_or_else(|| url.to_owned());
    let hops: Vec<String> = chain.collect();
    CallError::new(
        &TOO_MANY_REDIRECTS,
        format!(
            "the navigation to {first} was redirected again after {} redirects, past the browser's chain of {BROWSER_REDIRECT_CHAIN} requests",
            hops.len()
        ),
        &TooManyRedirectsDetails {
            url: first.clone(),
            hops,
            limit: RedirectLimit::Browser {
                count: BROWSER_REDIRECT_CHAIN,
            },
        },
    )
}

/// `response_truncated` for the document of the navigation to `url`, whose
/// body broke off after `received` bytes of the `expected`.
pub(crate) fn truncated(url: &str, received: u64, expected: Option<u64>) -> CallError {
    // `received` counts decoded bytes and `expected` the bytes as sent, so
    // a compressed body's count can pass its `Content-Length`: the two are
    // never read as a fraction.
    let of = expected.map_or_else(String::new, |expected| {
        format!("; its `Content-Length` declared {expected} as sent")
    });
    CallError::new(
        &RESPONSE_TRUNCATED,
        format!("the document of {url} broke off after {received} bytes once decoded{of}"),
        &ResponseTruncatedDetails {
            url: url.to_owned(),
            received,
            expected,
        },
    )
}

/// `navigation_failed` with the network error the browser named.
pub(crate) fn network_error(url: &str, error: &str) -> CallError {
    CallError::new(
        &NAVIGATION_FAILED,
        format!("the navigation to {url} failed: {error}"),
        &NavigationFailedDetails {
            url: Some(url.to_owned()),
            error: Some(error.to_owned()),
        },
    )
}

/// How Chromium refused a certificate it verified itself, on a host the
/// egress relay only tunnels, as the wire names the failure: none for a
/// network error that is no certificate's. Chromium names an expired
/// certificate and one not valid yet alike, and an unknown issuer and a
/// self-signed certificate alike; the relay's own look at the certificate
/// tells them apart where it can.
pub(crate) fn certificate_failure(error: &NetError) -> Option<CertificateFailure> {
    let NetError::Certificate(certificate) = error else {
        return None;
    };
    let known = |failure| Some(CertificateFailure::Known(failure));
    match certificate {
        CertificateError::NameMismatch => known(KnownCertificateFailure::NameMismatch),
        CertificateError::UnknownIssuer => known(KnownCertificateFailure::UnknownIssuer),
        CertificateError::Revoked => known(KnownCertificateFailure::Revoked),
        other => Some(CertificateFailure::Unrecognized(other.code())),
    }
}

#[cfg(test)]
mod tests;
