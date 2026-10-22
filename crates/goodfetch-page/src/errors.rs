//! What each CDP failure raises (ADR 0127): the CDP client types every
//! error and disconnection of the browser's as a [`Failure`]; this module
//! is where each variant becomes the Error code of the call that met it.
//! A lost browser or page is `page_crashed`, with how the browser ended
//! when it is the browser that was lost; any other failure is the call's
//! generic code: `navigation_failed` for a navigation and for reading the
//! history it goes through, as ADR 0085 names it, `browser_launch_failed`
//! for opening a page, `page_crashed` for reading a live page's document,
//! which only a broken renderer refuses or leaves unanswered, and an
//! internal error for a locator action, which has no generic code.

use goodfetch_cdp::{Failure, Refusal};
use goodfetch_engine::CallError;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    BROWSER_LAUNCH_FAILED, BrowserExit, BrowserLaunchFailedDetails, NAVIGATION_FAILED,
    NAVIGATION_INTERRUPTED, NavigationFailedDetails, NavigationInterruptedDetails, PAGE_CRASHED,
    PageCrashedDetails, RESPONSE_TRUNCATED, RedirectLimit, ResponseTruncatedDetails,
    TOO_MANY_REDIRECTS, TooManyRedirectsDetails,
};

/// The call a failure met.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    /// Opening a page: the launch's last steps, a browser context, a
    /// target and its domains. Holds the browser's executable.
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
    /// A locator action's commands: the guard's checks and its input.
    Action {
        /// The operation, such as `locator.click`.
        operation: &'static str,
    },
}

/// The Error code `failure` raises from `call`; `exit` says how the browser
/// ended, when it did.
pub(crate) fn call_error(call: &Call, failure: &Failure, exit: Option<BrowserExit>) -> CallError {
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
                url: url.clone(),
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
                url: String::new(),
                error: None,
            },
        ),
        Call::OpenPage { executable } => CallError::new(
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

/// `page_crashed` for a page whose renderer crashed, its browser still up.
pub(crate) fn renderer_crashed() -> CallError {
    CallError::new(
        &PAGE_CRASHED,
        "the page's renderer crashed",
        &PageCrashedDetails { browser_exit: None },
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

/// The browser's redirect count (Chromium's `kMaxRedirects`, ADR 0118): a
/// navigation's chain holds at most this many requests, its first
/// included, so the pinned Chromium follows 19 redirects and fails the
/// navigation at the next.
const BROWSER_REDIRECTS: u32 = 20;

/// `too_many_redirects` for the navigation to `url`, whose request went to
/// each URL of `chain` in turn, the first included.
pub(crate) fn too_many_redirects(url: &str, chain: Vec<String>) -> CallError {
    let mut chain = chain.into_iter();
    let first = chain.next().unwrap_or_else(|| url.to_owned());
    let hops: Vec<String> = chain.collect();
    CallError::new(
        &TOO_MANY_REDIRECTS,
        format!(
            "the navigation to {first} was redirected again after {} redirects, past the browser's count of {BROWSER_REDIRECTS}",
            hops.len()
        ),
        &TooManyRedirectsDetails {
            url: first.clone(),
            hops,
            limit: RedirectLimit::Browser {
                count: BROWSER_REDIRECTS,
            },
        },
    )
}

/// `response_truncated` for the document of the navigation to `url`, whose
/// body broke off after `received` bytes of the `expected`.
pub(crate) fn truncated(url: &str, received: u64, expected: Option<u64>) -> CallError {
    let of = expected.map_or_else(String::new, |expected| format!(" of {expected}"));
    CallError::new(
        &RESPONSE_TRUNCATED,
        format!("the document of {url} broke off after {received} bytes{of}"),
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
            url: url.to_owned(),
            error: Some(error.to_owned()),
        },
    )
}

#[cfg(test)]
mod tests;
