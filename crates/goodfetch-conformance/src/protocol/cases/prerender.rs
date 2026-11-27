//! The page suite's Speculation Rules prerendering case (ADR 0121, ticket
//! #730), under `page_states.prerendering`.
//!
//! Outcome on the pinned Chromium build (Chrome 156): over a potentially
//! trustworthy origin (`https://`), with `Prerender2` on, Chrome creates a
//! separate `prerender` target under tab targets and fires a speculative
//! `Sec-Purpose: prefetch;prerender` request even under page targets where
//! prerender itself cancels with `PrerenderingDisabledByDevTools`. Collie
//! therefore switches both `Prerender2` and
//! `Prerender2FallbackPrefetchSpecRules` off at launch under the
//! back/forward cache's rule, with no setting: `/` sends no speculative
//! request for `/next`, and clicking `#next` loads `/next` as a normal
//! navigation (`activationStart == 0`) that passes `page.wait_for_url`, is
//! judged by `session.page_check`, and consults the navigation gate.

use std::time::Duration;

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_URL;
use goodfetch_protocol::page::{LOCATOR_CLICK, LOGGED_OUT, PAGE_GOTO, PageId};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::{Value, json};

use super::consent::walls;
use super::{Checks, Site, finish_run, new_page, poll, start_run};
use crate::protocol::connection::{Connection, Gone, code_of};
use crate::protocol::target::EngineTarget;
use crate::proving_ground::Submission;

/// The hand-made `navigation/8` task host whose `/` declares a Speculation
/// Rules prerender of `/next`.
const PRERENDER_HOST: &str = "8.navigation.test";

/// The task ID of the prerender page.
const TASK: &str = "navigation/8";

/// The copy of the Proving ground's authority trusted for `PRERENDER_HOST`.
const AUTHORITY_FILE: &str = "conformance-prerender-authority.pem";

/// How long the oracle is asked again for `/next`'s `arrive` Effect.
const EFFECT_WAIT: Duration = Duration::from_secs(10);

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    PrerenderDisabledLoadsNormally,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        match id {
            "a-speculation-rules-link-loads-as-a-normal-navigation-passing-wait-for-url-and-page-check" => {
                Some(Self::PrerenderDisabledLoadsNormally)
            }
            _ => None,
        }
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::PrerenderDisabledLoadsNormally => {
                prerender_loads_normally(connection, target, site, checks)
            }
        }
    }
}

/// Counts Proving ground observations of `/next` on [`PRERENDER_HOST`].
fn next_request_count(site: &Site, checks: &mut Checks) -> Option<usize> {
    checks
        .needs(
            "the Proving ground's observations list requests for the prerender host",
            site.proving_ground.received_requests(),
        )
        .map(|requests| {
            requests
                .of_the_case()
                .filter(|request| request.host == PRERENDER_HOST && request.target == "/next")
                .count()
        })
}

/// Navigates `page_id` to `url` and lets the browser settle long enough
/// that an enabled prerender or fallback prefetch would fire.
fn load_and_settle(
    connection: &mut Connection,
    page_id: &PageId,
    url: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let went = connection.call(PAGE_GOTO.name, &json!({ "page_id": page_id, "url": url }))?;
    checks.check(
        format!("`{}` reaches `{url}`", PAGE_GOTO.name),
        went["result"]["response"]["status"] == 200,
    );
    let _ = connection.call(
        PAGE_EVALUATE.name,
        &json!({
            "page_id": page_id,
            "function": "() => new Promise((resolve) => setTimeout(resolve, 600))",
            "world": "main",
        }),
    )?;
    Ok(())
}

/// Clicks `#next` while a `page.wait_for_url` is armed, checks that `/next`
/// loaded as a normal navigation (`activationStart == 0`) with no prior
/// speculative fetch, and verifies the task against the oracle.
fn verify_wait_and_normal_navigation(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    url: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(());
    };
    let before_load = next_request_count(site, checks).unwrap_or(0);
    load_and_settle(connection, &page.page_id, url, checks)?;
    let before_click = next_request_count(site, checks).unwrap_or(usize::MAX);
    checks.check(
        "with `Prerender2` disabled, `/` sends no speculative request for `/next` before the click",
        before_click == before_load,
    );

    let wait_id = connection.request(
        PAGE_WAIT_FOR_URL.name,
        &json!({
            "page_id": page.page_id,
            "url": { "kind": "glob", "pattern": "**/next" },
            "timeout": 5_000,
        }),
    )?;
    let click_id = connection.request(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "#next" } }),
    )?;
    let (waited, clicked) = connection.both(wait_id, click_id)?;
    checks.check(
        "clicking `#next` answers the click and resolves the armed `page.wait_for_url`",
        clicked.get("result").is_some() && waited.get("result").is_some(),
    );
    let activation = connection.call(
        PAGE_EVALUATE.name,
        &json!({
            "page_id": page.page_id,
            "function": "() => document.querySelector('#activation')?.textContent ?? ''",
            "world": "main",
        }),
    )?;
    checks.check(
        "`/next` loads as a normal navigation (`activationStart` is 0)",
        activation["result"]["value"] == json!("0"),
    );

    let id = TaskId::try_from(TASK.to_owned()).expect("invariant: valid task ID");
    let verified = poll::try_until(
        EFFECT_WAIT,
        Duration::from_millis(100),
        || {
            site.proving_ground
                .judge::<Value>(&Submission {
                    task: &id,
                    status: "success",
                    records: &[],
                })
                .map(|answer| answer.outcome)
        },
        |outcome| *outcome == OracleOutcome::Verified,
    )
    .map_err(|why| {
        site.unread(format!(
            "the Proving ground's oracle judges `{TASK}` ({why})"
        ));
    })
    .ok();
    checks.check(
        "the Proving ground oracle verifies `navigation/8`",
        verified == Some(OracleOutcome::Verified),
    );
    Ok(())
}

/// In a second Session with `session.page_check`, clicking `#next` raises
/// `logged_out`.
fn verify_page_check(
    connection: &mut Connection,
    run_id: &str,
    url: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let session = connection.call(
        RUN_NEW_SESSION.name,
        &json!({
            "run_id": run_id,
            "settings": {
                "session": {
                    "page_check": {
                        "login_page": { "kind": "css", "selector": "#arrived" }
                    }
                }
            }
        }),
    )?;
    let Some(session_id) = session["result"]["session_id"].as_str() else {
        checks.check("the second Session with `session.page_check` opens", false);
        return Ok(());
    };
    let opened = connection.call(SESSION_NEW_PAGE.name, &json!({ "session_id": session_id }))?;
    let Some(page_id) = opened["result"]["page_id"]
        .as_str()
        .map(|id| PageId(id.to_owned()))
    else {
        checks.check("the Page check Session opens a page", false);
        return Ok(());
    };
    load_and_settle(connection, &page_id, url, checks)?;
    let clicked = connection.call(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page_id, "selector": "#next" } }),
    )?;
    let after = connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page_id, "function": "() => document.title", "world": "main" }),
    )?;
    checks.check(
        "clicking `#next` is judged by `session.page_check` and raises `logged_out`",
        code_of(&clicked) == &json!(LOGGED_OUT.name) || code_of(&after) == &json!(LOGGED_OUT.name),
    );
    Ok(())
}

/// Over TLS (a potentially trustworthy origin where Chromium would
/// otherwise evaluate Speculation Rules), `navigation/8` sends no
/// speculative fetch before `#next` is clicked, loads `/next` as a normal
/// navigation passing `page.wait_for_url`, and is judged by
/// `session.page_check`.
fn prerender_loads_normally(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    if site.start_url(TASK).is_none() {
        checks.check(format!("the Proving ground serves `{TASK}`"), false);
        return Ok(());
    }
    let url = site.named_secure_url(PRERENDER_HOST, "/");
    walls::with_trusted_authority(
        connection,
        target,
        site,
        ("conformance-prerender-home", PRERENDER_HOST, AUTHORITY_FILE),
        checks,
        |connection, site, checks| {
            let run_id = "conformance-prerender";
            start_run(connection, site, run_id, &json!({}), checks)?;
            verify_wait_and_normal_navigation(connection, site, run_id, &url, checks)?;
            verify_page_check(connection, run_id, &url, checks)?;
            finish_run(connection, run_id)
        },
    )
}
