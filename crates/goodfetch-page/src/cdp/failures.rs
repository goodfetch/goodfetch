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

use goodfetch_cdp::{Failure, Refusal};
use goodfetch_engine::CallError;
use goodfetch_engine::deadline::Deadline;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    BROWSER_LAUNCH_FAILED, BrowserExit, BrowserLaunchFailedDetails, NAVIGATION_FAILED,
    NavigationFailedDetails, PAGE_CRASHED, PageCrashedDetails,
};

use crate::errors::Call;

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
        return deadline.exceeded(WaitedFor::BROWSER_ANSWER, deadline.duration());
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

#[cfg(test)]
mod tests;
