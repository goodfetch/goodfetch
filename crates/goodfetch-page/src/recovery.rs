//! The Page check (spec #577, ticket #710; ADRs 0006, 0079 and 0092;
//! pitfall S10): every page of a Session is judged once its navigation
//! reached its document, whatever its HTTP status, so a login page
//! answering 200 is a lost login rather than a success. A page matching the
//! Session's `session.page_check` login page raises `logged_out`; one
//! matching neither its login page nor, when it gives one, its expected
//! page raises `page_check_failed`; both from the call that waited for the
//! navigation. A Session without a Page check is judged by ADR 0006's
//! built-in detector, and Provenance marks each page it judged as weakly
//! checked. The verdict is an effect of the judgement, not a change of the
//! page's state; the recovery ladder that acts on a login page is a later
//! ticket's (#747).
//!
//! The built-in detector reads a login page from two signs: a navigation
//! redirected to a login path (a last path segment such as `login` or
//! `sign-in`), or a page outside a login path showing one password field
//! that takes no new password. A navigation sent to a login path itself is
//! the program going to log in, never a lost login.

use goodfetch_engine::CallError;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Values;
use goodfetch_protocol::page::{
    JudgedBy, LOGGED_OUT, LoggedOutDetails, PAGE_CHECK_FAILED, PageCheckFailedDetails, PageId,
    PageResponse, SessionId,
};
use goodfetch_protocol::settings::page_check::{PageCheck, PageMatch};
use goodfetch_protocol::settings::{
    SESSION_PAGE_CHECK, SETTING_INVALID, SettingInvalidDetails, SettingsLayer,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::driver::PageDriver;
use crate::trace;
use crate::url_pattern::UrlPattern;

/// What the Page check reads of the document, in the guard's world.
const READ: &str = include_str!("recovery/page_check.js");

/// The Provenance action of a page the built-in detector judged.
const WEAKLY_CHECKED: &str = "page_weakly_checked";

/// The last path segments the built-in detector reads as a login page's,
/// any extension aside.
const LOGIN_SEGMENTS: [&str; 8] = [
    "login",
    "log-in",
    "log_in",
    "logon",
    "signin",
    "sign-in",
    "sign_in",
    "authenticate",
];

/// A Session's Page check, and the settings layer that set it.
#[derive(Clone, Debug)]
pub(crate) struct Declared {
    pub(crate) check: PageCheck,
    pub(crate) layer: SettingsLayer,
}

impl Declared {
    /// The Page check a Session of `run` with its own settings `own`
    /// declares: its own `session.page_check`, else the Run's, if either
    /// is set.
    pub(crate) fn of(run: &Run, own: &Values) -> Option<Self> {
        let (value, layer) = if let Some(value) = own.get(SESSION_PAGE_CHECK.name) {
            (value.clone(), SettingsLayer::Run)
        } else {
            let effective = run.settings()?.resolved().get(SESSION_PAGE_CHECK.name)?;
            (effective.value.clone(), effective.layer)
        };
        // The setting's shape was checked where it was set.
        let check = serde_json::from_value::<Option<PageCheck>>(value).ok()??;
        Some(Self { check, layer })
    }
}

/// One judgement: the page, its Session and Run, its Page check if it
/// declares one, and the URL its navigation was sent to, when it named one.
pub(crate) struct Judging<'a> {
    pub(crate) run: &'a Run,
    pub(crate) session: &'a SessionId,
    pub(crate) page_id: &'a PageId,
    pub(crate) driver: &'a PageDriver,
    pub(crate) declared: Option<&'a Declared>,
    pub(crate) requested: Option<&'a str>,
    /// The operation whose navigation is judged, which names a failure to
    /// read the document.
    pub(crate) operation: &'static str,
}

/// What the document showed the Page check's read.
#[derive(Default, Deserialize)]
struct Read {
    /// Whether each element or text match found anything, in the order
    /// asked; `None` for one the document refused.
    found: Vec<Option<bool>>,
    /// Whether it shows one password field that takes no new password.
    password: bool,
}

/// Judges the document `response` answered for `judging`'s navigation:
/// `logged_out` for a login page, `page_check_failed` for a page that is
/// neither, nothing for an expected page.
///
/// # Cancel safety
///
/// Dropping the future may lose a Provenance entry.
pub(crate) async fn judge(judging: &Judging<'_>, response: &PageResponse) -> Result<(), CallError> {
    match judging.declared {
        Some(declared) => by_page_check(judging, declared, response).await,
        None => by_built_in(judging, response).await,
    }
}

/// The verdict of the Session's own Page check.
async fn by_page_check(
    judging: &Judging<'_>,
    declared: &Declared,
    response: &PageResponse,
) -> Result<(), CallError> {
    let PageCheck {
        login_page,
        expected_page,
    } = &declared.check;
    let in_document: Vec<&PageMatch> = [login_page.as_ref(), expected_page.as_ref()]
        .into_iter()
        .flatten()
        .filter(|matcher| !is_url(matcher))
        .collect();
    let read = if in_document.is_empty() {
        Read::default()
    } else {
        read(judging, &in_document).await?
    };
    let mut asked = 0;
    let mut matches = |matcher: &PageMatch| -> Result<bool, CallError> {
        let url = &response.url;
        match matcher {
            PageMatch::Glob { pattern } => Ok(UrlPattern::glob(pattern)?.matches(url)),
            PageMatch::Regex { pattern } => Ok(UrlPattern::regex(pattern)?.matches(url)),
            PageMatch::Css { .. } | PageMatch::Xpath { .. } | PageMatch::Text { .. } => {
                let found = read.found.get(asked).copied();
                asked += 1;
                match found {
                    Some(Some(found)) => Ok(found),
                    Some(None) => Err(refused(matcher, declared.layer)),
                    None => Ok(false),
                }
            }
        }
    };
    let login = login_page.as_ref().map(&mut matches).transpose()?;
    if login == Some(true) {
        return Err(logged_out(response, JudgedBy::PageCheck));
    }
    let expected = expected_page.as_ref().map(&mut matches).transpose()?;
    if expected == Some(false) {
        return Err(CallError::new(
            &PAGE_CHECK_FAILED,
            format!(
                "The page `{}`, answered with {}, is neither the expected page nor the login page of its Session's Page check: check where the program navigated, or widen `{}`'s `expected_page`.",
                response.url, response.status, SESSION_PAGE_CHECK.name
            ),
            &PageCheckFailedDetails {
                url: response.url.clone(),
                status: response.status,
            },
        ));
    }
    Ok(())
}

/// The verdict of the built-in detector, which Provenance marks as a weak
/// check.
async fn by_built_in(judging: &Judging<'_>, response: &PageResponse) -> Result<(), CallError> {
    let login = if login_path(&response.url) {
        judging
            .requested
            .is_some_and(|requested| !same_document(requested, &response.url))
    } else {
        read(judging, &[]).await?.password
    };
    trace::provenance(
        judging.run,
        &json!({
            "action": WEAKLY_CHECKED,
            "page_id": judging.page_id,
            "session_id": judging.session,
            "url": response.url,
            "verdict": if login { "login_page" } else { "passed" },
        }),
    )
    .await;
    if login {
        return Err(logged_out(response, JudgedBy::BuiltIn));
    }
    Ok(())
}

/// Whether `matcher` matches the document's URL rather than its content.
fn is_url(matcher: &PageMatch) -> bool {
    matches!(matcher, PageMatch::Glob { .. } | PageMatch::Regex { .. })
}

/// Reads the document for `matches`, its element and text matches; a
/// document gone or not there reads as showing nothing.
async fn read(judging: &Judging<'_>, matches: &[&PageMatch]) -> Result<Read, CallError> {
    let argument = json!({ "matches": matches });
    let read = judging
        .driver
        .read_in_page(READ, argument, judging.operation)
        .await?;
    Ok(read
        .and_then(|value: Value| serde_json::from_value(value).ok())
        .unwrap_or_default())
}

/// `logged_out` for the login page `response` answered, judged by `by`.
fn logged_out(response: &PageResponse, by: JudgedBy) -> CallError {
    let judged = match by {
        JudgedBy::PageCheck => format!("its Session's Page check, `{}`", SESSION_PAGE_CHECK.name),
        JudgedBy::BuiltIn => {
            "the built-in detector, since the Session declares no Page check".to_owned()
        }
    };
    CallError::new(
        &LOGGED_OUT,
        format!(
            "The page `{}`, answered with {}, is a login page, judged by {judged}: the Session's login is lost. Log in again on this page, or open the Session from a source that holds the login.",
            response.url, response.status
        ),
        &LoggedOutDetails {
            url: response.url.clone(),
            status: response.status,
            judged_by: by,
        },
    )
}

/// `setting_invalid` for `matcher`, a selector or an expression the
/// document refused, set in `layer`.
fn refused(matcher: &PageMatch, layer: SettingsLayer) -> CallError {
    let reason = format!(
        "the page match {} is no selector or expression a document takes",
        serde_json::to_string(matcher).unwrap_or_default()
    );
    CallError::new(
        &SETTING_INVALID,
        format!(
            "`{}` is invalid in {}: {reason}.",
            SESSION_PAGE_CHECK.name,
            layer.as_str()
        ),
        &SettingInvalidDetails {
            key: SESSION_PAGE_CHECK.name.to_owned(),
            layer: Some(layer),
            reason,
        },
    )
}

/// Whether `url`'s last path segment, any extension aside, is a login
/// page's.
fn login_path(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    let last = url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let stem = last.split('.').next().unwrap_or_default();
    LOGIN_SEGMENTS
        .iter()
        .any(|segment| stem.eq_ignore_ascii_case(segment))
}

/// Whether `requested` and `reached` name one document, their fragments
/// aside.
fn same_document(requested: &str, reached: &str) -> bool {
    match (url::Url::parse(requested), url::Url::parse(reached)) {
        (Ok(mut requested), Ok(mut reached)) => {
            requested.set_fragment(None);
            reached.set_fragment(None);
            requested == reached
        }
        _ => requested == reached,
    }
}
