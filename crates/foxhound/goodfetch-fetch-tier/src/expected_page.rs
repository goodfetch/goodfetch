//! A Label's Page check judged on a document a non-browser tier fetched
//! (spec #579's `session_pools`; ADRs 0006 and 0093): a page failing its
//! expected page there escalates to the browser under `fetch.tier: auto`,
//! and fails with `page_check_failed` under a forced tier, while a login
//! page is a lost login on every tier.
//!
//! The document is judged only where its markup alone decides as the
//! browser would: its URL against a glob or a regular expression, through
//! the engine's one URL matcher, and an element a CSS selector finds in its
//! markup, visible or not, as the browser's Page check finds one. An
//! `XPath` expression and a body text are read from the rendered document
//! in the browser (its `innerText` leaves out what is hidden), so a check
//! naming one, like a pattern or a selector this tier cannot read, leaves
//! the page unjudged, for the browser to judge.

use goodfetch_engine::CallError;
use goodfetch_page::url_pattern::UrlPattern;
use goodfetch_protocol::page::{
    JudgedBy, LOGGED_OUT, LoggedOutDetails, PAGE_CHECK_FAILED, PageCheckFailedDetails,
};
use goodfetch_protocol::settings::SESSION_PAGE_CHECK;
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};

use crate::document::matched;

/// How a document fared against a Page check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// It is the expected page, or no expected page is given.
    Expected,
    /// It is the login page.
    LoginPage,
    /// It is neither.
    NotExpected,
    /// Its markup cannot decide.
    Unjudged,
}

/// The verdict of `check` on the document `html` reached at `url`: its
/// login page first, then its expected page.
pub(crate) fn judge(check: &PageCheck, url: &str, html: &str) -> Verdict {
    let found = |matcher: &PageMatch| found(matcher, url, html);
    match check.login_page.as_ref().map(found) {
        Some(None) => return Verdict::Unjudged,
        Some(Some(true)) => return Verdict::LoginPage,
        Some(Some(false)) | None => {}
    }
    match check.expected_page.as_ref().map(found) {
        None | Some(Some(true)) => Verdict::Expected,
        Some(Some(false)) => Verdict::NotExpected,
        Some(None) => Verdict::Unjudged,
    }
}

/// Whether `matcher` finds the document `html` reached at `url`; `None`
/// when its markup cannot decide.
fn found(matcher: &PageMatch, url: &str, html: &str) -> Option<bool> {
    match matcher {
        PageMatch::Glob { pattern } => UrlPattern::glob(pattern).ok().map(|glob| glob.matches(url)),
        PageMatch::Regex { pattern } => UrlPattern::regex(pattern)
            .ok()
            .map(|regex| regex.matches(url)),
        PageMatch::Css { selector } => matched(html, selector)
            .ok()
            .map(|elements| !elements.is_empty()),
        PageMatch::Xpath { .. } | PageMatch::Text { .. } => None,
    }
}

/// `page_check_failed` for the document reached at `url` with `status`,
/// neither its Label's expected page nor its login page.
pub(crate) fn failed(url: &str, status: u16) -> CallError {
    CallError::new(
        &PAGE_CHECK_FAILED,
        format!(
            "The page `{url}`, answered with {status}, is neither the expected page nor the login page of its Label's Page check, and its tier is forced, so it does not escalate to the browser: set `fetch.tier` to `auto` or `browser` for a page its script completes, or widen `{}`'s `expected_page`.",
            SESSION_PAGE_CHECK.name
        ),
        &PageCheckFailedDetails {
            url: url.to_owned(),
            status,
        },
    )
}

/// `logged_out` for the login page reached at `url` with `status`.
pub(crate) fn logged_out(url: &str, status: u16) -> CallError {
    CallError::new(
        &LOGGED_OUT,
        format!(
            "The page `{url}`, answered with {status}, is a login page, judged by its Label's Page check, `{}`: the Session's login is lost.",
            SESSION_PAGE_CHECK.name
        ),
        &LoggedOutDetails {
            url: url.to_owned(),
            status,
            judged_by: JudgedBy::PageCheck,
        },
    )
}

#[cfg(test)]
mod tests;
