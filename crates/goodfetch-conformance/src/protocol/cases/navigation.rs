//! The page suite's navigation cases (ADRs 0085 and 0121, ticket #698): a
//! navigation answers its document's `Response`, a 404 or a 500 included;
//! going back or forward loads the document again, since the back/forward
//! cache is off, also across sites, whose renderers the browser swaps; an
//! unreachable site fails with the browser's network error; a navigation
//! superseded before it committed is interrupted; and one whose document
//! does not load within `timeout.navigation` ends with `deadline_exceeded`
//! naming the setting (ADR 0080).

use std::time::Duration;

use goodfetch_protocol::connection::DEADLINE_EXCEEDED;
use goodfetch_protocol::page::waits::{PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL};
use goodfetch_protocol::page::{
    NAVIGATION_FAILED, NAVIGATION_INTERRUPTED, Navigated, PAGE_GO_BACK, PAGE_GO_FORWARD, PAGE_GOTO,
    PAGE_RELOAD, PageResponse,
};
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};

/// A time step at which the `pacing` family holds each answer about two
/// seconds (`proving-ground/README.md`).
const SLOW_STEP: u64 = 95;

/// How long a superseding navigation waits after the one it supersedes, so
/// the browser has started the first, whose answer the Proving ground holds.
const SUPERSEDING_DELAY: Duration = Duration::from_millis(300);

/// A host name no resolver answers (RFC 6761).
const UNRESOLVED_URL: &str = "http://nowhere.invalid/";

/// The answer to `method` on `page_id` with `params` added.
pub(super) fn call(
    connection: &mut Connection,
    method: &str,
    page_id: &Value,
    params: &Value,
) -> Result<Value, Gone> {
    let mut params = params.clone();
    params["page_id"] = page_id.clone();
    let id = connection.request(method, &params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// The document's response a navigation's answer holds, checked to be one.
pub(super) fn response(answer: &Value, what: &str, checks: &mut Checks) -> Option<PageResponse> {
    let navigated = answer
        .get("result")
        .and_then(|result| serde_json::from_value::<Navigated>(result.clone()).ok());
    checks.check(
        format!("{what} answers with what its result type holds"),
        navigated.is_some(),
    );
    let response = navigated.and_then(|navigated| navigated.response);
    checks.check(
        format!("{what} answers a document's response"),
        response.is_some(),
    );
    response
}

/// Checks that a navigation's answer is the response of `url` with `status`.
pub(super) fn answered(answer: &Value, what: &str, url: &str, status: u16, checks: &mut Checks) {
    if let Some(response) = response(answer, what, checks) {
        checks.check(
            format!("{what}: its status is {status}"),
            response.status == status,
        );
        checks.check(format!("{what}: its URL is {url}"), response.url == url);
    }
}

/// The URL of `path` on the host of the task `id`'s start URL.
pub(super) fn url_on(site: &Site, id: &str, path: &str) -> Option<String> {
    let start = site.start_url(id)?;
    let origin_end = start
        .find("://")
        .and_then(|scheme| start[scheme + 3..].find('/').map(|path| scheme + 3 + path))
        .unwrap_or(start.len());
    Some(format!("{}{path}", &start[..origin_end]))
}

/// Opens a page of a new Run `run_id`, checking that the Proving ground
/// serves `urls`; `None` when it does not or the page did not open.
pub(super) fn open(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    urls: &[&Option<String>],
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    let served = urls.iter().all(|url| url.is_some());
    checks.check("the Proving ground serves the case's pages", served);
    if !served {
        return Ok(None);
    }
    start_run(connection, site, run_id, &json!({}), checks)?;
    Ok(new_page(connection, run_id, checks)?.map(|page| json!(page.page_id)))
}

/// A 404 and a 500 are answers, not failures, of `goto` and `reload`.
pub(super) fn error_statuses(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-error-statuses";
    let not_found = url_on(site, "statuses/1", "/errors/404");
    let server_error = url_on(site, "statuses/1", "/errors/500");
    if let (Some(page), Some(not_found), Some(server_error)) = (
        open(
            connection,
            site,
            run_id,
            &[&not_found, &server_error],
            checks,
        )?,
        &not_found,
        &server_error,
    ) {
        let goto = call(
            connection,
            PAGE_GOTO.name,
            &page,
            &json!({ "url": not_found }),
        )?;
        answered(&goto, "`page.goto` to a 404", not_found, 404, checks);
        let goto = call(
            connection,
            PAGE_GOTO.name,
            &page,
            &json!({ "url": server_error }),
        )?;
        answered(&goto, "`page.goto` to a 500", server_error, 500, checks);
        let reload = call(connection, PAGE_RELOAD.name, &page, &json!({}))?;
        answered(
            &reload,
            "`page.reload` of the 500",
            server_error,
            500,
            checks,
        );
    }
    if not_found.is_some() {
        finish_run(connection, run_id)?;
    }
    Ok(())
}

/// Back and forward between two sites answer each document's response, and
/// the page still navigates after the browser swapped its renderer; going
/// forward from the last entry answers none.
pub(super) fn history_across_sites(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-history-across-sites";
    let first = site.page_url.clone();
    let second = url_on(site, "navigation/2", "/");
    let third = url_on(site, "statuses/1", "/errors/404");
    if let (Some(page), Some(first), Some(second), Some(third)) = (
        open(connection, site, run_id, &[&first, &second, &third], checks)?,
        &first,
        &second,
        &third,
    ) {
        call(connection, PAGE_GOTO.name, &page, &json!({ "url": first }))?;
        call(connection, PAGE_GOTO.name, &page, &json!({ "url": second }))?;
        let back = call(connection, PAGE_GO_BACK.name, &page, &json!({}))?;
        answered(
            &back,
            "`page.go_back` to the other site",
            first,
            200,
            checks,
        );
        let forward = call(connection, PAGE_GO_FORWARD.name, &page, &json!({}))?;
        answered(
            &forward,
            "`page.go_forward` to the second site",
            second,
            200,
            checks,
        );
        let goto = call(connection, PAGE_GOTO.name, &page, &json!({ "url": third }))?;
        answered(
            &goto,
            "`page.goto` after the sites' swaps",
            third,
            404,
            checks,
        );
        let last = call(connection, PAGE_GO_FORWARD.name, &page, &json!({}))?;
        checks.check(
            "`page.go_forward` from the last entry answers no response",
            last.get("result") == Some(&json!({ "response": null })),
        );
    }
    if first.is_some() && second.is_some() && third.is_some() {
        finish_run(connection, run_id)?;
    }
    Ok(())
}

/// Going back to a page Chromium's back/forward cache would restore loads
/// it again, so the call waits for its `load` event and answers its
/// response (ADR 0121; pitfall B21).
pub(super) fn back_loads_again(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-back-loads-again";
    let first = url_on(site, "navigation/2", "/");
    let second = url_on(site, "navigation/2", "/b");
    if let (Some(page), Some(first), Some(second)) = (
        open(connection, site, run_id, &[&first, &second], checks)?,
        &first,
        &second,
    ) {
        call(connection, PAGE_GOTO.name, &page, &json!({ "url": first }))?;
        call(connection, PAGE_GOTO.name, &page, &json!({ "url": second }))?;
        let back = call(connection, PAGE_GO_BACK.name, &page, &json!({}))?;
        answered(&back, "`page.go_back`", first, 200, checks);
    }
    if first.is_some() && second.is_some() {
        finish_run(connection, run_id)?;
    }
    Ok(())
}

/// A URL waiter for the first page, until its `load`, and a response
/// waiter for its document, both armed before going back to it, pass
/// with the Back navigation: the page loads again from the network and
/// fires `load`, never restored from the back/forward cache (ADR 0121;
/// pitfall B21).
pub(super) fn waits_armed_before_back(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-waits-armed-before-back";
    let first = url_on(site, "navigation/2", "/");
    let second = url_on(site, "navigation/2", "/b");
    if let (Some(page), Some(first), Some(second)) = (
        open(connection, site, run_id, &[&first, &second], checks)?,
        &first,
        &second,
    ) {
        call(connection, PAGE_GOTO.name, &page, &json!({ "url": first }))?;
        call(connection, PAGE_GOTO.name, &page, &json!({ "url": second }))?;
        let exact = json!({ "kind": "glob", "pattern": first });
        let by_url = connection.request(
            PAGE_WAIT_FOR_URL.name,
            &json!({ "page_id": page, "url": exact, "wait_until": "load" }),
        )?;
        let by_response = connection.request(
            PAGE_WAIT_FOR_RESPONSE.name,
            &json!({ "page_id": page, "url": exact }),
        )?;
        let back = connection.request(PAGE_GO_BACK.name, &json!({ "page_id": page }))?;
        let mut answers = answers_to(connection, &[by_url, by_response, back])?.into_iter();
        let (url_reached, response_caught, back) = (
            answers.next().unwrap_or_default(),
            answers.next().unwrap_or_default(),
            answers.next().unwrap_or_default(),
        );
        answered(
            &back,
            "`page.go_back` with waiters armed",
            first,
            200,
            checks,
        );
        checks.check(
            "the URL waiter armed before `page.go_back` passes once the page fired `load`",
            url_reached.get("result").is_some(),
        );
        checks.check(
            "the response waiter armed before `page.go_back` gets the document's response from the network",
            response_caught["result"]["response"]["status"] == json!(200),
        );
    }
    if first.is_some() && second.is_some() {
        finish_run(connection, run_id)?;
    }
    Ok(())
}

/// The answers to the requests `ids`, in that order, whichever came first.
fn answers_to(connection: &mut Connection, ids: &[i64]) -> Result<Vec<Value>, Gone> {
    let mut read: Vec<Value> = Vec::new();
    let mut answers = Vec::with_capacity(ids.len());
    for id in ids {
        let at = read
            .iter()
            .position(|message| message.get("method").is_none() && message["id"] == json!(id));
        if let Some(at) = at {
            answers.push(read.remove(at));
        } else {
            let answer = connection.browser_answer(*id)?;
            read.extend(answer.others);
            answers.push(answer.message);
        }
    }
    Ok(answers)
}

/// Checks that `answer` fails with `navigation_failed`, retryable, naming
/// the browser's network error `error`.
pub(super) fn failed_with(answer: &Value, what: &str, error: &str, checks: &mut Checks) {
    let data = &answer["error"]["data"];
    checks.check(
        format!("{what} fails with `{}`", NAVIGATION_FAILED.name),
        data["code"] == json!(NAVIGATION_FAILED.name),
    );
    checks.check(
        format!("{what}: a retry may succeed"),
        data["retryable"] == json!(true),
    );
    checks.check(
        format!("{what}: it names the browser's `{error}`"),
        data["error"] == json!(error),
    );
}

/// A name that does not resolve fails the navigation with the browser's
/// `net::ERR_NAME_NOT_RESOLVED`, told apart from a refused connection
/// (pitfall H01): one to the port of the Proving ground's `down` fixture,
/// which it holds with nothing listening while it runs, so no other
/// program can take it, as one could a port bound and released
/// (fix-queue 31 and 88).
pub(super) fn unresolved_name(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-unresolved-name";
    let refused = site
        .proxy("down")
        .and_then(|down| down.parse::<std::net::SocketAddr>().ok())
        .and_then(|down| {
            site.page_url
                .as_deref()
                .and_then(|url| url_on_port(url, down.port()))
        });
    checks.check(
        "the Proving ground holds a port that refuses connections",
        refused.is_some(),
    );
    if let (Some(page), Some(refused)) = (
        open(connection, site, run_id, &[&refused], checks)?,
        &refused,
    ) {
        let unresolved = call(
            connection,
            PAGE_GOTO.name,
            &page,
            &json!({ "url": UNRESOLVED_URL }),
        )?;
        failed_with(
            &unresolved,
            "`page.goto` to a name that does not resolve",
            "net::ERR_NAME_NOT_RESOLVED",
            checks,
        );
        let answer = call(
            connection,
            PAGE_GOTO.name,
            &page,
            &json!({ "url": refused }),
        )?;
        failed_with(
            &answer,
            "`page.goto` to a port that refuses connections",
            "net::ERR_CONNECTION_REFUSED",
            checks,
        );
    }
    if refused.is_some() {
        finish_run(connection, run_id)?;
    }
    Ok(())
}

/// `url` with its port replaced by `port`.
fn url_on_port(url: &str, port: u16) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    Some(format!("{scheme}://{host}:{port}{path}"))
}

/// A navigation that another supersedes before its document committed
/// raises `navigation_interrupted`, and the other answers its response
/// (ADR 0085). The Proving ground holds the first one's answer.
pub(super) fn superseded(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-superseded";
    let held = site.start_url("pacing/1").map(ToOwned::to_owned);
    let next = site.page_url.clone();
    let (Some(page), Some(held), Some(next)) = (
        open(connection, site, run_id, &[&held, &next], checks)?,
        &held,
        &next,
    ) else {
        if held.is_some() && next.is_some() {
            finish_run(connection, run_id)?;
        }
        return Ok(());
    };
    let slowed = site.proving_ground.set_time_step(SLOW_STEP).is_ok();
    checks.check("the Proving ground holds its pacing pages' answers", slowed);
    let first = connection.request(PAGE_GOTO.name, &json!({ "page_id": page, "url": held }))?;
    pause(SUPERSEDING_DELAY);
    let second = connection.request(PAGE_GOTO.name, &json!({ "page_id": page, "url": next }))?;
    let answers = both(connection, first, second);
    let restored = site.proving_ground.set_time_step(0).is_ok();
    checks.check("the Proving ground's time step is set back", restored);
    let (first, second) = answers?;
    let data = &first["error"]["data"];
    checks.check(
        format!(
            "the superseded `page.goto` fails with `{}`",
            NAVIGATION_INTERRUPTED.name
        ),
        data["code"] == json!(NAVIGATION_INTERRUPTED.name),
    );
    checks.check(
        "a retry is not said to succeed",
        data["retryable"] == json!(false),
    );
    checks.check("it names the superseded URL", data["url"] == json!(held));
    answered(&second, "the superseding `page.goto`", next, 200, checks);
    finish_run(connection, run_id)
}

/// The navigation deadline of the case whose document loads past it: well
/// below the about two seconds the Proving ground holds its answer.
const SHORT_NAVIGATION_TIMEOUT: u64 = 500;

/// A navigation whose document does not load within its `timeout`, the
/// call's own `timeout.navigation`, ends with `deadline_exceeded`,
/// retryable, whose `limit` names the setting, its value and the run
/// layer a call's own argument is, and whose `waited_for` names the state
/// the call waited for (ADR 0080); a `timeout` of 0 is refused. The
/// Proving ground holds the document's answer past the deadline. The
/// deadline is the call's alone, so opening the page waits under the
/// Run's default, as every other case's does, whatever the machine's load.
pub(super) fn past_its_deadline(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-navigation-deadline";
    let held = site.start_url("pacing/1").map(ToOwned::to_owned);
    checks.check("the Proving ground serves the case's pages", held.is_some());
    let Some(held) = held else {
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let refused = call(
        connection,
        PAGE_GOTO.name,
        &json!(page.page_id),
        &json!({ "url": held, "timeout": 0 }),
    )?;
    checks.check(
        "a `timeout` of 0 is refused with `invalid_params`",
        refused["error"]["code"] == json!(-32602),
    );
    let slowed = site.proving_ground.set_time_step(SLOW_STEP).is_ok();
    checks.check("the Proving ground holds its pacing pages' answers", slowed);
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &json!(page.page_id),
        &json!({ "url": held, "wait_until": "load", "timeout": SHORT_NAVIGATION_TIMEOUT }),
    );
    let restored = site.proving_ground.set_time_step(0).is_ok();
    checks.check("the Proving ground's time step is set back", restored);
    let data = &answer?["error"]["data"];
    checks.check(
        format!(
            "`page.goto` past `timeout.navigation` fails with `{}`",
            DEADLINE_EXCEEDED.name
        ),
        data["code"] == json!(DEADLINE_EXCEEDED.name),
    );
    checks.check("a retry may succeed", data["retryable"] == json!(true));
    checks.check(
        "its limit names `timeout.navigation`, the call's value and the run layer",
        data["limit"]
            == json!({ "key": "timeout.navigation", "value": SHORT_NAVIGATION_TIMEOUT, "layer": "run" }),
    );
    checks.check(
        "it waited for the `load` the call named",
        data["waited_for"] == json!("load"),
    );
    checks.check(
        "it waited at least as long as its limit",
        data["elapsed"]
            .as_u64()
            .is_some_and(|elapsed| elapsed >= SHORT_NAVIGATION_TIMEOUT),
    );
    finish_run(connection, run_id)
}

/// The answers to the requests `first` and `second`, in whichever order
/// they come.
fn both(connection: &mut Connection, first: i64, second: i64) -> Result<(Value, Value), Gone> {
    let answer = connection.browser_answer(first)?;
    if let Some(early) = answer
        .others
        .into_iter()
        .find(|message| message.get("method").is_none() && message["id"] == json!(second))
    {
        return Ok((answer.message, early));
    }
    Ok((answer.message, connection.browser_answer(second)?.message))
}

/// Waits `duration` on the harness's own thread.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness waits on its own thread, outside any async worker (ADR 0060's note for the harness)"
)]
fn pause(duration: Duration) {
    std::thread::sleep(duration);
}
