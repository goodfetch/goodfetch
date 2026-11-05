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

use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_engine::CallError;
use goodfetch_page::url_pattern::UrlPattern;
use goodfetch_protocol::page::{
    JudgedBy, LOGGED_OUT, LoggedOutDetails, PAGE_CHECK_FAILED, PageCheckFailedDetails,
};
use goodfetch_protocol::settings::SESSION_PAGE_CHECK;
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};

use crate::document::{CONTROLS, matched_in};

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
/// login page first, then its expected page; `html` parsed for this call
/// alone, at its first CSS selector, as a handler's page's check is. A
/// crawl request's own document is judged once, by [`judge_with`] on its
/// untouched tree.
pub(crate) fn judge(check: &PageCheck, url: &str, html: &str, work: &CountedWork) -> Verdict {
    let mut parsed = None;
    judge_with(check, url, |selector| {
        let matcher = crate::document::matcher(selector, work).ok()?;
        let document = parsed.get_or_insert_with(|| work.parse_html(Row::PageParses, html));
        let controls = work.literal_selector(Row::PageSelectorCompilations, CONTROLS);
        Some(!matched_in(document, &matcher, &controls).is_empty())
    })
}

/// The verdict of `check` on the document reached at `url`, its login page
/// first, then its expected page, whose markup `css` reads: whether the
/// CSS selector it is given finds an element, `None` for one the engine
/// does not read.
pub(crate) fn judge_with(
    check: &PageCheck,
    url: &str,
    mut css: impl FnMut(&str) -> Option<bool>,
) -> Verdict {
    let mut found = |matcher: &PageMatch| found(matcher, url, &mut css);
    match check.login_page.as_ref().map(&mut found) {
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

/// Each CSS selector `check` names, in its order: what its markup reads.
pub(crate) fn selectors(check: &PageCheck) -> impl Iterator<Item = &str> {
    [check.login_page.as_ref(), check.expected_page.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|matcher| match matcher {
            PageMatch::Css { selector } => Some(selector.as_str()),
            PageMatch::Glob { .. }
            | PageMatch::Regex { .. }
            | PageMatch::Xpath { .. }
            | PageMatch::Text { .. } => None,
        })
}

/// Whether `matcher` finds the document reached at `url`, whose markup
/// `css` reads; `None` when its markup cannot decide.
fn found(
    matcher: &PageMatch,
    url: &str,
    css: &mut impl FnMut(&str) -> Option<bool>,
) -> Option<bool> {
    match matcher {
        PageMatch::Glob { pattern } => UrlPattern::glob(pattern).ok().map(|glob| glob.matches(url)),
        PageMatch::Regex { pattern } => UrlPattern::regex(pattern)
            .ok()
            .map(|regex| regex.matches(url)),
        PageMatch::Css { selector } => css(selector),
        PageMatch::Xpath { .. } | PageMatch::Text { .. } => None,
    }
}

/// Whether the document `html`, answered with `headers`, is marked
/// `noindex` (ADR 0035), `html` parsed for this call alone.
pub(crate) fn noindex(headers: &BTreeMap<String, String>, html: &str) -> bool {
    goodfetch_page::noindex::marked(headers, &robots(html))
}

/// The `content` of each robots `meta` element of `html`.
#[expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn robots(html: &str) -> Vec<String> {
    let document = dom_query::Document::from(html);
    let named =
        dom_query::Matcher::new(ROBOTS).expect("invariant: a selector written in the code is CSS");
    robots_in(&document, &named)
}

/// What names a document's robots `meta` elements, among others.
pub(crate) const ROBOTS: &str = "meta[name]";

/// The `content` of each robots `meta` element of `document`, `named`
/// being [`ROBOTS`]' matcher.
pub(crate) fn robots_in(document: &ParsedHtml, named: &dom_query::Matcher) -> Vec<String> {
    document
        .select_matcher(named)
        .nodes()
        .iter()
        .filter(|meta| {
            meta.attr("name")
                .is_some_and(|name| name.eq_ignore_ascii_case("robots"))
        })
        .map(|meta| {
            meta.attr("content")
                .map(|content| content.to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// `page_check_failed` for the document reached at `url` with `status`,
/// neither its Label's expected page nor its login page, marked `noindex`
/// or not.
pub(crate) fn failed(url: &str, status: u16, noindex: bool) -> CallError {
    CallError::new(
        &PAGE_CHECK_FAILED,
        format!(
            "The page `{url}`, answered with {status}, is neither the expected page nor the login page of its Label's Page check, and its tier is forced, so it does not escalate to the browser: set `fetch.tier` to `auto` or `browser` for a page its script completes, or widen `{}`'s `expected_page`.",
            SESSION_PAGE_CHECK.name
        ),
        &PageCheckFailedDetails {
            url: url.to_owned(),
            status,
            noindex,
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
