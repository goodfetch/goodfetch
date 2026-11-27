//! The page suite's Page check cases (ADRs 0006, 0079 and 0092, ticket
//! #710; pitfall S10): every page of a Session judged once its navigation
//! reached its document. With `session.page_check` set, the portal's login
//! page answering 200 raises `logged_out`, and so does the page its expired
//! login answers with a 200, while a page that is neither the expected page
//! nor the login page raises `page_check_failed` from the `goto` that
//! reached it; an expected page answers its response. Each form of a page
//! match, a URL glob or regular expression, a CSS or `XPath` selector and a
//! text, judges the same three pages. A page a click reached is judged too,
//! raised by the wait for its URL. Without a Page check the built-in
//! detector judges a Session only once it holds a login (ADR 0006's owner
//! decision of 2026-11-10, ticket #2447): before that, a redirect to a
//! login path or a login form at a page's own URL answers its response
//! while Provenance still marks each judged page as weakly checked; after a
//! password field is filled and a document outside a login path arrives, a
//! redirect to a login path or a login form at a page's own URL raises
//! `logged_out` judged by `built_in`, while a failed login that returns to
//! a login path gives no login.

use goodfetch_protocol::page::events::PAGE_WAIT_FOR_REQUEST_FINISHED;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_URL;
use goodfetch_protocol::page::{
    LOCATOR_CLICK, LOCATOR_FILL, LOGGED_OUT, PAGE_CHECK_FAILED, PageOpened,
};
use serde_json::{Value, json};

use super::navigation::url_on;
use super::{Checks, Site, finish_run, new_page, run_trace, start_run};
use crate::protocol::connection::{Connection, Gone, responded_with};
use crate::protocol::target::EngineTarget;
use crate::proving_ground::TaskAccount;

/// The portal's task whose login expires after four pages and answers
/// with its login form and a 200.
const EXPIRY_TASK: &str = "portal/2";

/// The catalog's task whose start page is its listing.
const CATALOG_TASK: &str = "catalog/1";

/// The Provenance action of a page the built-in detector judged.
const WEAKLY_CHECKED: &str = "page_weakly_checked";

/// One Page check case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    LostLogin,
    EachForm,
    BuiltIn,
    FreshSession,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-login-page-answering-200-is-a-lost-login-and-a-page-that-is-neither-fails-the-page-check" => {
                Self::LostLogin
            }
            "each-form-of-a-page-match-judges-the-expected-page-the-login-page-and-neither" => {
                Self::EachForm
            }
            "without-a-page-check-the-built-in-detector-judges-and-each-page-is-weakly-checked" => {
                Self::BuiltIn
            }
            "on-a-fresh-session-a-login-form-at-a-pages-own-url-is-no-lost-login" => {
                Self::FreshSession
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
            Self::LostLogin => lost_login(connection, site, checks),
            Self::EachForm => each_form(connection, site, checks),
            Self::BuiltIn => built_in(connection, target, site, checks),
            Self::FreshSession => fresh_session(connection, site, checks),
        }
    }
}

/// Whether `answer` raised `code` for the document at a URL ending with
/// `path`, answered with `status`.
fn raised(answer: &Value, code: &str, path: &str, status: u16) -> bool {
    let data = &answer["error"]["data"];
    data["code"] == json!(code)
        && data["url"].as_str().is_some_and(|url| url.ends_with(path))
        && data["status"] == json!(status)
}

/// The portal's Page check, by CSS selectors: its login form, and the
/// reports page.
fn portal_check() -> Value {
    json!({
        "login_page": { "kind": "css", "selector": "form#login" },
        "expected_page": { "kind": "css", "selector": "main#records" },
    })
}

/// Signs `page`, on the portal's login page, in with `username` and
/// `password`, waiting for a response matching `pattern`; answers the
/// finished request's response status.
fn submit_login(
    connection: &mut Connection,
    page: &PageOpened,
    username: &str,
    password: &str,
    pattern: &str,
) -> Result<Value, Gone> {
    for (selector, value) in [
        ("input[name=user]", username),
        ("input[name=password]", password),
    ] {
        connection.call(
            LOCATOR_FILL.name,
            &json!({ "locator": { "page_id": page.page_id, "selector": selector }, "value": value }),
        )?;
    }
    let waiting = connection.request(
        PAGE_WAIT_FOR_REQUEST_FINISHED.name,
        &json!({ "page_id": page.page_id, "url": { "kind": "glob", "pattern": pattern } }),
    )?;
    let clicked = connection.request(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "button[type=submit]" } }),
    )?;
    let (_, finished) = connection.both(clicked, waiting)?;
    Ok(finished)
}

/// Signs `page`, on the portal's login page, in as `account` through its
/// form; whether the report page the sign-in serves answered.
fn sign_in(
    connection: &mut Connection,
    page: &PageOpened,
    account: &TaskAccount,
) -> Result<bool, Gone> {
    let signed_in = submit_login(
        connection,
        page,
        &account.username,
        &account.password,
        "**/reports*",
    )?;
    Ok(signed_in["result"]["response"]["status"] == json!(200))
}

/// Clicks the report page's `Next` link on `page` and answers the wait,
/// armed before it, for the URL of report page `number`.
fn click_next(connection: &mut Connection, page: &PageOpened, number: u32) -> Result<Value, Gone> {
    let waiting = connection.request(
        PAGE_WAIT_FOR_URL.name,
        &json!({ "page_id": page.page_id, "url": { "kind": "glob", "pattern": format!("**/reports?page={number}") } }),
    )?;
    let clicked = connection.request(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "a.next" } }),
    )?;
    let (_, waited) = connection.both(clicked, waiting)?;
    Ok(waited)
}

/// On a Run whose Session checks the portal's pages: a `goto` to the login
/// page, answered with a 200, raises `logged_out`, and the page stays on
/// it, so the program signs in through the form; the four report pages the
/// login serves pass, the last reached by a click; the fifth, which the
/// expired login answers with the login form and a 200, reached by a click
/// on its `Next` link, raises `logged_out` from the wait for its URL
/// (golden scenario 04, failure point 2); and an error page, neither the
/// expected page nor the login page, raises `page_check_failed`.
fn lost_login(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-check-lost-login";
    let (Some(login), Some(account), Some(not_found)) = (
        url_on(site, EXPIRY_TASK, "/login"),
        site.account(EXPIRY_TASK).cloned(),
        url_on(site, "statuses/1", "/errors/404"),
    ) else {
        checks.check(
            format!(
                "the Proving ground serves `{EXPIRY_TASK}` with its account and the statuses' 404"
            ),
            false,
        );
        return Ok(());
    };
    let settings = json!({ "session": { "page_check": portal_check() } });
    start_run(connection, site, run_id, &settings, checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let went = connection.goto(&page, &login)?;
    checks.check(
        "a `goto` to the login page answering 200 raises `logged_out` judged by the Page check",
        raised(&went, LOGGED_OUT.name, "/login", 200)
            && went["error"]["data"]["judged_by"] == json!("page_check"),
    );
    let signed_in = sign_in(connection, &page, &account)?;
    checks.check(
        "the program signs in from the login page the page stayed on",
        signed_in,
    );
    // The sign-in served the first report page; the login serves four.
    let mut served = true;
    for number in 2..=3 {
        let report = format!("/reports?page={number}");
        let url = url_on(site, EXPIRY_TASK, &report)
            .expect("invariant: the case checked that the Proving ground serves the task");
        served &= responded_with(&connection.goto(&page, &url)?, 200);
    }
    checks.check(
        "each report page the login serves is the expected page and answers its response",
        served,
    );
    let fourth = click_next(connection, &page, 4)?;
    checks.check(
        "a report page a click reached is the expected page, and the wait for its URL answers",
        fourth.get("result").is_some(),
    );
    let expired = click_next(connection, &page, 5)?;
    checks.check(
        "the login form an expired login answers with a 200 to a click raises `logged_out` from the wait for its URL",
        raised(&expired, LOGGED_OUT.name, "/reports?page=5", 200),
    );
    let went = connection.goto(&page, &not_found)?;
    checks.check(
        "a page that is neither the expected page nor the login page raises `page_check_failed`",
        raised(&went, PAGE_CHECK_FAILED.name, "/errors/404", 404),
    );
    finish_run(connection, run_id)
}

/// Each form of a page match, as a Session's own `session.page_check`:
/// the catalog's listing, its expected page, answers its response; the
/// portal's login page raises `logged_out`; and the statuses' 404, neither,
/// raises `page_check_failed`.
fn each_form(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-page-check-forms";
    let (Some(listing), Some(login), Some(not_found)) = (
        site.start_url(CATALOG_TASK).map(str::to_owned),
        url_on(site, EXPIRY_TASK, "/login"),
        url_on(site, "statuses/1", "/errors/404"),
    ) else {
        checks.check(
            "the Proving ground serves the catalog's listing, the portal's login page and the statuses' 404",
            false,
        );
        return Ok(());
    };
    let forms = [
        (
            "glob",
            json!({ "kind": "glob", "pattern": "**/login" }),
            json!({ "kind": "glob", "pattern": "**/catalog/*" }),
        ),
        (
            "regex",
            json!({ "kind": "regex", "pattern": "/login$" }),
            json!({ "kind": "regex", "pattern": r"/catalog/\d+$" }),
        ),
        (
            "css",
            json!({ "kind": "css", "selector": "form#login" }),
            json!({ "kind": "css", "selector": "main#catalog" }),
        ),
        (
            "xpath",
            json!({ "kind": "xpath", "selector": "//form[@id='login']" }),
            json!({ "kind": "xpath", "selector": "//main[@id='catalog']" }),
        ),
        (
            "text",
            json!({ "kind": "text", "text": "Sign in" }),
            json!({ "kind": "text", "text": "Catalog" }),
        ),
    ];
    start_run(connection, site, run_id, &json!({}), checks)?;
    for (form, login_page, expected_page) in forms {
        let settings = json!({
            "session": { "page_check": { "login_page": login_page, "expected_page": expected_page } },
        });
        let opened = connection.call(
            RUN_NEW_SESSION.name,
            &json!({ "run_id": run_id, "settings": settings }),
        )?;
        let session = opened["result"]["session_id"].clone();
        let paged = connection.call(SESSION_NEW_PAGE.name, &json!({ "session_id": session }))?;
        let Ok(page) = serde_json::from_value::<PageOpened>(paged["result"].clone()) else {
            checks.check(
                format!("a Session whose own Page check matches by {form} opens a page"),
                false,
            );
            continue;
        };
        let expected = connection.goto(&page, &listing)?;
        checks.check(
            format!("by {form}, the expected page answers its response"),
            responded_with(&expected, 200),
        );
        let lost = connection.goto(&page, &login)?;
        checks.check(
            format!("by {form}, the login page raises `logged_out`"),
            raised(&lost, LOGGED_OUT.name, "/login", 200),
        );
        let neither = connection.goto(&page, &not_found)?;
        checks.check(
            format!("by {form}, a page that is neither raises `page_check_failed`"),
            raised(&neither, PAGE_CHECK_FAILED.name, "/errors/404", 404),
        );
    }
    finish_run(connection, run_id)
}

/// Triggers a script navigation to `account` (which redirects to `/login`)
/// and answers the page's next call (`document.title`).
fn script_redirect_next_call(
    connection: &mut Connection,
    page: &PageOpened,
    account: &str,
    checks: &mut Checks,
) -> Result<Value, Gone> {
    let finished = connection.request(
        PAGE_WAIT_FOR_REQUEST_FINISHED.name,
        &json!({ "page_id": page.page_id, "url": { "kind": "glob", "pattern": "**/login" } }),
    )?;
    let sent = connection.request(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page.page_id, "function": format!("() => {{ location.href = {}; }}", json!(account)) }),
    )?;
    let (sent, finished) = connection.both(sent, finished)?;
    checks.answered(PAGE_EVALUATE.name, &sent);
    checks.answered(PAGE_WAIT_FOR_REQUEST_FINISHED.name, &finished);
    connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page.page_id, "function": "() => document.title" }),
    )
}

/// Without a Page check, the built-in detector judges a Session only once
/// it holds a login (ADR 0006, ticket #2447): before login (and after a
/// failed login that returns to a login path) redirects to `/login` answer
/// their response while Provenance still writes `page_weakly_checked` with
/// verdict `login_page`; once the program fills a password field and a
/// document outside a login path arrives, a redirect to `/login` raises
/// `logged_out` judged by `built_in` from `goto` or the page's next call,
/// while a `goto` to `/login` itself still answers its response.
fn built_in(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-check-built-in";
    let (Some(listing), Some(account), Some(login), Some(portal_account)) = (
        site.start_url(CATALOG_TASK).map(str::to_owned),
        url_on(site, "login/2", "/account"),
        url_on(site, EXPIRY_TASK, "/login"),
        site.account(EXPIRY_TASK).cloned(),
    ) else {
        checks.check(
            "the Proving ground serves the catalog's listing, the login site's account page and the portal's login page with its account",
            false,
        );
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let listed = connection.goto(&page, &listing)?;
    checks.check(
        "an ordinary page answers its response",
        responded_with(&listed, 200),
    );
    let sent = connection.goto(&page, &account)?;
    checks.check(
        "on a fresh Session with no login, a redirect to `/login` answers the login page's response",
        responded_with(&sent, 200)
            && sent["result"]["response"]["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/login")),
    );
    let went = connection.goto(&page, &login)?;
    checks.check(
        "a `goto` to a login page itself answers its response",
        responded_with(&went, 200),
    );
    let next = script_redirect_next_call(connection, &page, &account, checks)?;
    checks.check(
        "on a fresh Session with no login, a script redirect to `/login` does not raise from the page's next call",
        next.get("result").is_some(),
    );
    check_failed_then_succeeded_login(
        connection,
        &page,
        (&account, &login, &portal_account),
        checks,
    )?;
    finish_run(connection, run_id)?;
    check_built_in_provenance(target, run_id, &page, checks);
    Ok(())
}

/// Checks that a wrong password returning `/login` gives no login, and that
/// filling the right password and landing outside a login path makes later
/// redirects to `/login` raise `logged_out` judged by `built_in`.
fn check_failed_then_succeeded_login(
    connection: &mut Connection,
    page: &PageOpened,
    (account, login, portal_account): (&str, &str, &TaskAccount),
    checks: &mut Checks,
) -> Result<(), Gone> {
    let went = connection.goto(page, login)?;
    checks.check(
        "navigating to the portal's login page for a wrong-password attempt answers 200",
        responded_with(&went, 200),
    );
    let failed = submit_login(
        connection,
        page,
        &portal_account.username,
        "wrong-password",
        "**/login",
    )?;
    checks.check(
        "a wrong password answers the login page with 401",
        failed["result"]["response"]["status"] == json!(401),
    );
    let after_wrong = connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page.page_id, "function": "() => document.title" }),
    )?;
    checks.check(
        "the page's next call after a wrong password answers its value",
        after_wrong.get("result").is_some(),
    );
    let still_fresh = connection.goto(page, account)?;
    checks.check(
        "after a wrong password returned a login path, a redirect to `/login` still answers its response",
        responded_with(&still_fresh, 200),
    );
    let went = connection.goto(page, login)?;
    let signed_in = responded_with(&went, 200) && sign_in(connection, page, portal_account)?;
    checks.check(
        "the program fills the password field and reaches a page outside a login path",
        signed_in,
    );
    let sent = connection.goto(page, account)?;
    checks.check(
        "after a login, a redirect to `/login` raises `logged_out` judged by the built-in detector",
        raised(&sent, LOGGED_OUT.name, "/login", 200)
            && sent["error"]["data"]["judged_by"] == json!("built_in"),
    );
    let went = connection.goto(page, login)?;
    checks.check(
        "after a login, a `goto` to `/login` itself still answers its response",
        responded_with(&went, 200),
    );
    let next = script_redirect_next_call(connection, page, account, checks)?;
    checks.check(
        "after a login, a script redirect to `/login` raises `logged_out` from the page's next call",
        raised(&next, LOGGED_OUT.name, "/login", 200)
            && next["error"]["data"]["judged_by"] == json!("built_in"),
    );
    Ok(())
}

/// Checks the `page_weakly_checked` Provenance entries written by [`built_in`].
fn check_built_in_provenance(
    target: &EngineTarget,
    run_id: &str,
    page: &PageOpened,
    checks: &mut Checks,
) {
    let lines = checks.run_trace(run_trace::provenance(target, run_id));
    let weakly: Vec<&Value> = lines
        .iter()
        .filter(|line| {
            line["action"] == json!(WEAKLY_CHECKED) && line["page_id"] == json!(page.page_id)
        })
        .collect();
    let marked = |path: &str, verdict: &str| {
        weakly.iter().any(|line| {
            line["verdict"] == json!(verdict)
                && line["url"].as_str().is_some_and(|url| url.ends_with(path))
        })
    };
    checks.check(
        "Provenance marks the ordinary page as weakly checked and passed",
        marked("/catalog/1", "passed"),
    );
    checks.check(
        "Provenance marks the post-login report page as weakly checked and passed",
        marked("/reports?page=1", "passed"),
    );
    let login_verdicts = weakly
        .iter()
        .filter(|line| {
            line["verdict"] == json!("login_page")
                && line["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/login"))
        })
        .count();
    checks.check(
        "Provenance marks every redirected `/login` page (before and after login) as weakly checked with verdict `login_page`",
        login_verdicts == 5,
    );
    checks.check(
        "Provenance writes a `page_weakly_checked` entry for every judged page",
        weakly.len() == 12,
    );
}

/// Without a Page check, on a fresh Session, a login form at a page's own
/// URL answers its response before the Session holds a login; once the
/// program fills a password field and reaches a page outside a login path,
/// an expired login returning its login form at a report page's own URL
/// raises `logged_out` judged by the built-in detector (ADR 0006, #2447).
fn fresh_session(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-page-check-fresh-session";
    let (Some(forms), Some(login), Some(account)) = (
        url_on(site, "access/1", "/page-events/forms"),
        url_on(site, EXPIRY_TASK, "/login"),
        site.account(EXPIRY_TASK).cloned(),
    ) else {
        checks.check(
            format!("the Proving ground serves `access/1` and `{EXPIRY_TASK}` with its account"),
            false,
        );
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let before = connection.goto(&page, &forms)?;
    checks.check(
        "before login, a login form at a non-login URL answers its response on a fresh Session",
        responded_with(&before, 200),
    );
    let went = connection.goto(&page, &login)?;
    let signed_in = responded_with(&went, 200) && sign_in(connection, &page, &account)?;
    checks.check(
        "the program signs in through the login page of a fresh Session",
        signed_in,
    );
    let mut served = true;
    for number in 2..=4 {
        let url = url_on(site, EXPIRY_TASK, &format!("/reports?page={number}"))
            .expect("invariant: the case checked that the Proving ground serves the task");
        served &= responded_with(&connection.goto(&page, &url)?, 200);
    }
    checks.check(
        "each report page the login serves answers its response",
        served,
    );
    let expired = url_on(site, EXPIRY_TASK, "/reports?page=5")
        .expect("invariant: the case checked that the Proving ground serves the task");
    let went = connection.goto(&page, &expired)?;
    checks.check(
        "after login, a login form at a report page's own URL raises `logged_out` judged by the built-in detector",
        raised(&went, LOGGED_OUT.name, "/reports?page=5", 200)
            && went["error"]["data"]["judged_by"] == json!("built_in"),
    );
    finish_run(connection, run_id)
}
