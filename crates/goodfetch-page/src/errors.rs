//! The Error codes the page's own logic raises (ADR 0127): the call a
//! failure met, which names the generic code the CDP module's failures
//! raise; a navigation's ends, such as `navigation_interrupted`,
//! `too_many_redirects` and `response_truncated`, and a certificate the
//! browser refused, named from the browser's network error; the gate's
//! refusals; and a call's cancellation, the page's close and a panic.

use futures_util::FutureExt as _;
use goodfetch_engine::CallError;
use goodfetch_engine::crash_report;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::net_error::{BROWSER_REDIRECT_CHAIN, CertificateError, NetError};
use goodfetch_engine::run::limit_reached;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CLOSED, CallCancelledDetails, CancelledBy, ClosedValue, EnginePanickedDetails,
    INTERNAL, InternalDetails, PanicEnded,
};
use goodfetch_protocol::governor::{PAGE_MEMORY_EXCEEDED, PageMemoryExceededDetails};
use goodfetch_protocol::jsonrpc::ErrorData;
use goodfetch_protocol::page::events::PAGE_EVENTS_LOST;
use goodfetch_protocol::page::{
    BrowserExit, NAVIGATION_FAILED, NAVIGATION_INTERRUPTED, NavigationFailedDetails,
    NavigationInterruptedDetails, PAGE_CRASHED, PageCrashedDetails, PageId, RESPONSE_TRUNCATED,
    RedirectLimit, ResponseTruncatedDetails, SITE_PREFERENCES_REFUSED, SessionId,
    SitePreferencesRefusedDetails, TOO_MANY_REDIRECTS, TooManyRedirectsDetails,
};

use goodfetch_protocol::run::StopReason;
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

/// Why a function did not answer in a world the engine keeps.
pub(crate) enum Unanswered {
    /// The document it ran in went away, by a navigation: the world is made
    /// again in the next document.
    WorldGone,
    /// The call fails with this Error code.
    Failed(CallError),
}

/// The error of the navigation or request to `url` the gate refused,
/// `site_preferences_refused` or `response_limit_exceeded`, that
/// `refusal` refused at the page's navigation gate (ADRs 0086 and 0093).
pub(crate) fn gate_refused(url: &str, refusal: &GateRefusal) -> CallError {
    match refusal {
        GateRefusal::Preference { reason, message } => CallError::new(
            &SITE_PREFERENCES_REFUSED,
            format!("the site's preferences refused {url}: {message}"),
            &SitePreferencesRefusedDetails {
                url: url.to_owned(),
                reason: *reason,
            },
        ),
        GateRefusal::PastALimit(limit) => limit.error(),
    }
}

/// That error as a failed request carries it, for the request to `url` that
/// `refusal` refused.
pub(crate) fn gate_failure(url: &str, refusal: &GateRefusal) -> ErrorData {
    let error = gate_refused(url, refusal);
    ErrorData {
        code: error.code().to_owned(),
        retryable: error.retryable(),
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

/// `closed` for the page `page_id`.
pub(crate) fn page_closed(page_id: &PageId) -> CallError {
    CallError::new(
        &CLOSED,
        format!("the page `{}` is closed, or its Run ended", page_id.0),
        &serde_json::json!({ "page_id": page_id }),
    )
}

/// `page_events_lost` for the page `page_id`, which the engine closed once
/// its reader fell 4,096 events behind (ADR 0146): every call on it raises
/// this, which names it alone.
pub(crate) fn events_lost(page_id: &PageId) -> CallError {
    CallError::new(
        &PAGE_EVENTS_LOST,
        format!(
            "the engine fell too far behind the events of the page `{}` and closed it: open another page and act again",
            page_id.0
        ),
        &serde_json::json!({ "page_id": page_id }),
    )
}

/// `engine_panicked` for the page `page_id`, which a panic of one of its
/// owner's tasks closed, alone: its Session and its Run go on (ADR 0148).
/// It names the crash report the panic left, `crash_report`, when one was
/// written, and is counted on `metrics`, its Run's.
pub(crate) fn page_panicked(
    metrics: &Metrics,
    page_id: &PageId,
    crash_report: Option<&str>,
) -> CallError {
    crash_report::engine_panicked_told(
        metrics,
        &EnginePanickedDetails {
            ended: PanicEnded::Page,
            id: Some(page_id.0.clone()),
            crash_report: crash_report.map(str::to_owned),
        },
        &format!(
            "The engine hit a bug in Goodfetch while it served the page `{}`, which closed the page; its Session and its Run go on",
            page_id.0
        ),
    )
}

/// `engine_panicked` for a panic of one of the Session `session`'s
/// owner's tasks, which closed the Session with its pages, alone: its Run
/// goes on (ADRs 0146, 0148). It names the crash report the panic left,
/// `crash_report`, when one was written, and is counted on `metrics`, its
/// Run's.
pub(crate) fn session_panicked(
    metrics: &Metrics,
    session: &SessionId,
    crash_report: Option<&str>,
) -> CallError {
    crash_report::engine_panicked_told(
        metrics,
        &EnginePanickedDetails {
            ended: PanicEnded::Session,
            id: Some(session.0.clone()),
            crash_report: crash_report.map(str::to_owned),
        },
        &format!(
            "The engine hit a bug in Goodfetch while it served the Session `{}`, which closed the Session and its pages; its Run goes on",
            session.0
        ),
    )
}

/// `engine_panicked` for a panic of one of a browser's owner's tasks, which
/// the engine treats as that browser's crash: its pages raise
/// `page_crashed`, and its Run goes on (ADR 0148). It names the crash
/// report the panic left, `crash_report`, when one was written, and is
/// counted on `metrics`, its Run's.
pub(crate) fn browser_panicked(metrics: &Metrics, crash_report: Option<&str>) -> CallError {
    crash_report::engine_panicked_told(
        metrics,
        &EnginePanickedDetails {
            ended: PanicEnded::Browser,
            id: None,
            crash_report: crash_report.map(str::to_owned),
        },
        "The engine hit a bug in Goodfetch while it served a browser of the Run, which it treats as crashed; its Run goes on",
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
pub(crate) fn cancelled(by: CancelledBy, closed: ClosedValue) -> CallError {
    cancelled_for(by, closed, None)
}

/// As [`cancelled`], naming `person`, whom the stopping Client acts for
/// (ADR 0031), when known.
fn cancelled_for(by: CancelledBy, closed: ClosedValue, person: Option<String>) -> CallError {
    let message = match (&by, &closed) {
        (CancelledBy::Browser, ClosedValue::Page) => {
            "the page's window was closed in the browser, outside every Client: open a page and act again"
        }
        (_, ClosedValue::Run) => "the page's Run was stopped while the call was in flight",
        (CancelledBy::Browser, _) => "the page's browser quit, and its Session ended with it",
        (_, ClosedValue::Session) => "the page's Session was closed while the call was in flight",
        (_, _) => "the page was closed while the call was in flight",
    };
    CallError::new(
        &CALL_CANCELLED,
        message,
        &CallCancelledDetails {
            by,
            closed: Some(closed),
            person,
        },
    )
}

/// What a call in flight raises when `run` stops under it: the Limit the
/// Run reached, as the charge that reached it raised (ADR 0041), and
/// otherwise `call_cancelled` closing the Run (ADR 0113), by whoever
/// aborted it, or by the program for any other stop, such as a pause.
pub(crate) fn run_stopped(run: &Run) -> CallError {
    match run.stopping().now_or_never() {
        Some(StopReason::Limit(reached)) => limit_reached(&reached),
        Some(StopReason::Abort { by, person }) => {
            cancelled_for(by.into(), ClosedValue::Run, person)
        }
        _ => cancelled(CancelledBy::Program, ClosedValue::Run),
    }
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
