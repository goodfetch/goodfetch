//! The page suite's blocking-off cases (ADR 0047, ticket #716) on the
//! Proving ground's `clean-up.test` pages: a page with no Block cause that
//! fails its Page check while the Filter lists are on is loaded once more
//! with them off, and so is a challenge whose requests the lists blocked;
//! blocking is learned off for the page's domain, for every Session of the
//! Run, only when that retry passes. A page whose content its own listed
//! script renders passes on the retry, so the lists are off for its domain
//! from then on; a page that fails for another reason is retried once and
//! still raises `page_check_failed`, leaving the lists on; and a challenge
//! whose check script a list blocks clears on the retry; a retry whose load
//! fails is recorded all the same. Each is judged by
//! the Proving ground's own record of the requests it received, by the
//! Errors the engine answers and by the Run's Provenance log, which names
//! each retry, why it ran and whether blocking was learned off, and each
//! page later loaded with blocking off.

use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_ABORT, ROUTE_CONTINUE};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{NAVIGATION_FAILED, PAGE_CHECK_FAILED, PageOpened};
use serde_json::{Value, json};

use super::clean_up::{AD, ANNOYANCE, HOST, TRACKER, counts, finish, provenance, received, visit};
use super::navigation::{answered, call};
use super::routes::{Act, Routes, glob, goto};
use super::{Checks, Site, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The Provenance action of a page loaded with blocking off.
const BLOCKING_OFF: &str = "blocking_off";

/// The article whose text its own listed script renders, and the script.
const NEEDS_SCRIPT: &str = "/needs-script";
const RENDERING_SCRIPT: &str = "/js/ads.bundle.js";

/// The challenge whose check script a Filter list blocks, and the script.
const LISTED_CHALLENGE: &str = "/listed-challenge";
const CLEARING_SCRIPT: &str = "/js/fingerprint2.min.js";

/// One blocking-off case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    LearnedOff,
    NotLearnedOff,
    RetryFailed,
    Challenge,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-page-whose-content-a-listed-script-renders-passes-its-retry-with-blocking-off-which-is-learned-off" => {
                Self::LearnedOff
            }
            "a-page-failing-its-page-check-for-another-reason-is-retried-once-and-raises-page-check-failed" => {
                Self::NotLearnedOff
            }
            "a-retry-with-blocking-off-whose-load-fails-is-recorded-in-provenance" => {
                Self::RetryFailed
            }
            "a-challenge-the-filter-lists-blocked-a-request-of-is-retried-once-with-blocking-off" => {
                Self::Challenge
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::LearnedOff => learned_off(connection, target, site, checks),
            Self::NotLearnedOff => not_learned_off(connection, target, site, checks),
            Self::RetryFailed => retry_failed(connection, target, site, checks),
            Self::Challenge => challenge(connection, target, site, checks),
        }
    }
}

/// A Page check whose expected page shows `#content`.
fn expects_content() -> Value {
    json!({ "session": { "page_check": { "expected_page": { "kind": "css", "selector": "#content" } } } })
}

/// Opens a page of a new Session of the Run `run_id` with its own
/// `settings`.
fn open_in_session(
    connection: &mut Connection,
    run_id: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<Option<PageOpened>, Gone> {
    let opened = connection.call(
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": settings }),
    )?;
    let session = opened["result"]["session_id"].clone();
    let paged = connection.call(SESSION_NEW_PAGE.name, &json!({ "session_id": session }))?;
    let page = serde_json::from_value::<PageOpened>(paged["result"].clone()).ok();
    checks.check(
        "a Session with its own Page check opens a page",
        page.is_some(),
    );
    Ok(page)
}

/// The Run `run_id`'s Provenance entries of pages loaded with blocking off
/// whose URL ends with `path`.
fn blocking_off(target: &EngineTarget, run_id: &str, path: &str) -> Vec<Value> {
    provenance(target, run_id)
        .into_iter()
        .filter(|line| {
            line["action"] == json!(BLOCKING_OFF)
                && line["url"].as_str().is_some_and(|url| url.ends_with(path))
        })
        .collect()
}

/// Whether `entry` names the page `page`, the pages' domain, `reason` and,
/// for a retry, whether blocking was `learned` off.
fn names(entry: &Value, page: &PageOpened, reason: &str, learned: Option<bool>) -> bool {
    entry["page_id"] == json!(page.page_id)
        && entry["session_id"] == json!(page.session_id)
        && entry["domain"] == json!(HOST)
        && entry["reason"] == json!(reason)
        && learned.is_none_or(|learned| entry["learned"] == json!(learned))
}

/// `/needs-script`'s text is rendered by its own script, which EasyList
/// blocks: under a Page check expecting `#content`, the first load fails
/// it, the retry with blocking off passes and answers the `goto`, and
/// blocking is learned off for the domain, so the Run's default Session
/// then loads `/ads`'s ad, tracker and annoyance; Provenance names the
/// retry and the later page.
fn learned_off(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocking-off-learned";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(checked) = open_in_session(connection, run_id, &expects_content(), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, [NEEDS_SCRIPT, RENDERING_SCRIPT]);
    let (url, answer) = visit(connection, site, &checked, NEEDS_SCRIPT)?;
    answered(
        &answer,
        "the `goto` to the page its listed script renders",
        &url,
        200,
        checks,
    );
    checks.check(
        "the page was loaded twice, and its script only on the second load",
        counts(site, [NEEDS_SCRIPT, RENDERING_SCRIPT]) == [before[0] + 2, before[1] + 1],
    );
    let Some(other) = new_page(connection, run_id, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let blocked = counts(site, [AD, TRACKER, ANNOYANCE]);
    let (url, answer) = visit(connection, site, &other, "/ads")?;
    answered(
        &answer,
        "the default Session's `goto` to the ads page",
        &url,
        200,
        checks,
    );
    checks.check(
        "blocking is off for the domain in the Run's other Session: the ad, tracker and annoyance load",
        counts(site, [AD, TRACKER, ANNOYANCE]) == blocked.map(|count| count + 1),
    );
    finish(connection, run_id)?;
    checks.check(
        "Provenance records the retry with blocking off, after the failed Page check, and that blocking was learned off",
        blocking_off(target, run_id, NEEDS_SCRIPT)
            .iter()
            .any(|entry| names(entry, &checked, "page_check_failed", Some(true))),
    );
    checks.check(
        "Provenance records that the later page was loaded with blocking learned off",
        blocking_off(target, run_id, "/ads")
            .iter()
            .any(|entry| names(entry, &other, "learned", None)),
    );
    Ok(())
}

/// `/ads` fails a Page check expecting `#content`, so it is loaded once
/// more with blocking off, and a route aborts that second load: the `goto`
/// raises the load's failure, `navigation_failed`, and Provenance still
/// records the retry, blocking not learned off.
fn retry_failed(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocking-off-retry-failed";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(checked) = open_in_session(connection, run_id, &expects_content(), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let mut loads = 0;
    let mut routes = Routes::new(move |_| {
        loads += 1;
        let act = if loads == 1 {
            Act::Call(ROUTE_CONTINUE.name, json!({}))
        } else {
            Act::Call(ROUTE_ABORT.name, json!({}))
        };
        vec![act, Act::End]
    });
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": checked.page_id, "url": glob("**/ads?retry=fails") }),
    )?;
    let url = site.named_url(HOST, "/ads?retry=fails");
    let answer = goto(&mut routes, connection, &checked, &url)?;
    checks.check(
        format!(
            "the `goto` whose retry with blocking off is aborted raises `{}`",
            NAVIGATION_FAILED.name
        ),
        answer["error"]["data"]["code"] == json!(NAVIGATION_FAILED.name),
    );
    checks.check(
        "the route saw the document's two loads",
        routes.handled.len() == 2,
    );
    finish(connection, run_id)?;
    checks.check(
        "Provenance records the failed retry with blocking off, blocking not learned off",
        blocking_off(target, run_id, "/ads?retry=fails")
            .iter()
            .any(|entry| names(entry, &checked, "page_check_failed", Some(false))),
    );
    Ok(())
}

/// `/ads` holds no `#content`, whatever blocking does: under a Page check
/// expecting one, it is loaded once more with blocking off, which loads its
/// ad, and still failing raises `page_check_failed`; blocking is not
/// learned off, so the Run's default Session's `/ads` keeps its ad blocked.
fn not_learned_off(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocking-off-not-learned";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(checked) = open_in_session(connection, run_id, &expects_content(), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, ["/ads", AD]);
    let (_, answer) = visit(connection, site, &checked, "/ads")?;
    checks.check(
        format!(
            "the `goto` to a page that is not the expected page raises `{}`",
            PAGE_CHECK_FAILED.name
        ),
        answer["error"]["data"]["code"] == json!(PAGE_CHECK_FAILED.name),
    );
    checks.check(
        "the page was loaded twice, its ad only on the second load, with blocking off",
        counts(site, ["/ads", AD]) == [before[0] + 2, before[1] + 1],
    );
    let Some(other) = new_page(connection, run_id, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let (url, answer) = visit(connection, site, &other, "/ads")?;
    answered(
        &answer,
        "the default Session's `goto` to the ads page",
        &url,
        200,
        checks,
    );
    checks.check(
        "blocking was not learned off: the default Session's ads page keeps its ad blocked",
        received(site, AD) == before[1] + 1,
    );
    finish(connection, run_id)?;
    let entries = blocking_off(target, run_id, "/ads");
    checks.check(
        "Provenance records the retry with blocking off, after the failed Page check, and that blocking was not learned off",
        entries
            .iter()
            .any(|entry| names(entry, &checked, "page_check_failed", Some(false))),
    );
    checks.check(
        "Provenance records no page of the default Session loaded with blocking off",
        !entries
            .iter()
            .any(|entry| entry["page_id"] == json!(other.page_id)),
    );
    Ok(())
}

/// `/listed-challenge` is a challenge whose check script EasyPrivacy
/// blocks: it is loaded once more at once with blocking off, and its
/// script clears it, so blocking is learned off for the domain and a later
/// `goto` to it loads its script on the first load.
fn challenge(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocking-off-challenge";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, [LISTED_CHALLENGE, CLEARING_SCRIPT]);
    let (url, answer) = visit(connection, site, &page, LISTED_CHALLENGE)?;
    answered(
        &answer,
        "the `goto` to the challenge the lists touch",
        &url,
        403,
        checks,
    );
    checks.check(
        "the challenge was loaded twice, its check script only on the second load",
        counts(site, [LISTED_CHALLENGE, CLEARING_SCRIPT]) == [before[0] + 2, before[1] + 1],
    );
    let content = call(
        connection,
        PAGE_EVALUATE.name,
        &json!(page.page_id),
        &json!({ "function": "() => document.querySelector('#content') !== null" }),
    )?;
    checks.check(
        "the page shows what the challenge let through",
        content["result"]["value"] == json!(true),
    );
    let (url, answer) = visit(connection, site, &page, LISTED_CHALLENGE)?;
    answered(
        &answer,
        "a later `goto` to the challenge",
        &url,
        403,
        checks,
    );
    checks.check(
        "with blocking learned off, the later `goto` loads the challenge once, with its script",
        counts(site, [LISTED_CHALLENGE, CLEARING_SCRIPT]) == [before[0] + 3, before[1] + 2],
    );
    finish(connection, run_id)?;
    checks.check(
        "Provenance records the retry with blocking off, after the lists touched a challenge, and that blocking was learned off",
        blocking_off(target, run_id, LISTED_CHALLENGE)
            .iter()
            .any(|entry| names(entry, &page, "challenge", Some(true))),
    );
    Ok(())
}
