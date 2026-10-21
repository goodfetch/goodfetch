//! What each CDP failure raises (ADR 0127): the CDP client types every
//! error and disconnection of the browser's as a [`Failure`]; this module
//! is where each variant becomes the Error code of the call that met it.
//! A lost browser or page is `page_crashed`, with how the browser ended
//! when it is the browser that was lost; any other failure is the call's
//! generic code: `navigation_failed` for a navigation, as ADR 0085 names
//! it, and `browser_launch_failed` for opening a page.

use goodfetch_cdp::{Failure, Refusal};
use goodfetch_engine::CallError;
use goodfetch_protocol::page::{
    BROWSER_LAUNCH_FAILED, BrowserExit, BrowserLaunchFailedDetails, NAVIGATION_FAILED,
    NavigationFailedDetails, PAGE_CRASHED, PageCrashedDetails,
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
    /// A navigation to `url`.
    Navigation {
        /// Its URL.
        url: String,
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
        Call::OpenPage { executable } => CallError::new(
            &BROWSER_LAUNCH_FAILED,
            format!("the browser did not open the page: {failure}"),
            &BrowserLaunchFailedDetails {
                executable: executable.clone(),
                exit,
            },
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
