//! The page suite's app-state wait cases, the engine's Time conformance
//! lines (ADR 0080, ticket #705; pitfalls B02, B03 and B04): a URL waiter
//! armed before a `pushState`, a `replaceState` and a hash change passes for
//! each, on the Proving ground's single-page app; `networkidle` comes on the
//! live board, whose WebSocket and server-sent events never end, and on the
//! late board only once its request, started at `load` and held past the
//! 500 ms of quiet, finished; a
//! response waiter sent before the click that triggers the response catches
//! it, though the response came before its answer was read; the locator
//! waits and the function wait resolve on the actionability pages that
//! re-render; and each wait ends at `timeout.wait` with `deadline_exceeded`
//! naming what it waited for. Durations are short through settings, never a
//! test-only clock (ADR 0059).

use std::time::Duration;

use goodfetch_protocol::connection::{CALL_CANCELLED, DEADLINE_EXCEEDED};
use goodfetch_protocol::page::events::PAGE_WAIT_FOR_REQUEST_FINISHED;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::route::PAGE_ROUTE;
use goodfetch_protocol::page::session_pages::PAGE_CLOSE;
use goodfetch_protocol::page::waits::{
    LOCATOR_WAIT_FOR, LOCATOR_WAIT_FOR_COUNT, LOCATOR_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_FUNCTION,
    PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_REQUEST, PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL,
};
use goodfetch_protocol::page::{LOCATOR_CLICK, PAGE_GOTO, PATTERN_UNSUPPORTED};
use goodfetch_protocol::settings::TIMEOUT_WAIT;
use serde_json::{Value, json};

use super::exchange::Exchange;
use super::{Checks, Site, clean_up, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};

/// `timeout.wait` where a case expects its waits to pass: 10 s.
const LONG_WAIT: u64 = 10_000;

/// `timeout.wait` where a case expects its waits to end at it: 1 s.
const SHORT_WAIT: u64 = 1_000;

/// How long a waiter is watched for an answer it must not give yet.
const QUIET: Duration = Duration::from_millis(300);

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    UrlWithinTheDocument,
    NetworkIdle,
    ResponseBeforeAwaited,
    ReRenderingPages,
    Deadlines,
    UnsupportedPatterns,
    OwnTimeouts,
    PageCloses,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-url-waiter-armed-before-a-push-state-a-replace-state-and-a-hash-change-passes-for-each" => {
                Self::UrlWithinTheDocument
            }
            "network-idle-comes-on-a-polling-page-and-only-once-a-late-request-finished" => {
                Self::NetworkIdle
            }
            "a-response-waiter-sent-before-the-click-catches-the-response-it-triggers" => {
                Self::ResponseBeforeAwaited
            }
            "locator-and-function-waits-resolve-on-re-rendering-pages" => Self::ReRenderingPages,
            "each-wait-ends-at-timeout-wait-naming-what-it-waited-for" => Self::Deadlines,
            "a-route-or-a-wait-whose-pattern-the-engine-cannot-match-is-refused-at-the-call" => {
                Self::UnsupportedPatterns
            }
            "a-waits-own-timeout-is-its-deadline-at-the-run-layer-and-null-is-none" => {
                Self::OwnTimeouts
            }
            "a-wait-in-flight-when-its-page-closes-raises-call-cancelled-naming-the-page" => {
                Self::PageCloses
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        let mut exchange = Exchange::new(connection);
        match self {
            Self::UrlWithinTheDocument => url_within_the_document(&mut exchange, site, checks),
            Self::NetworkIdle => network_idle(&mut exchange, site, checks),
            Self::ResponseBeforeAwaited => response_before_awaited(&mut exchange, site, checks),
            Self::ReRenderingPages => re_rendering_pages(&mut exchange, site, checks),
            Self::Deadlines => deadlines(&mut exchange, site, checks),
            Self::UnsupportedPatterns => unsupported_patterns(&mut exchange, site, checks),
            Self::OwnTimeouts => own_timeouts(&mut exchange, site, checks),
            Self::PageCloses => page_closes(&mut exchange, site, checks),
        }
    }
}

/// What these cases do on the Connection besides reading it.
impl Exchange<'_> {
    /// The answer to the request `id` if it comes within [`QUIET`].
    fn answered_at_once(&mut self, id: i64) -> Result<Option<Value>, Gone> {
        self.answered_within(id, QUIET)
    }

    /// The value `page.evaluate` of `function` answers on `page`.
    fn evaluated(&mut self, page: &Value, function: &str) -> Result<Value, Gone> {
        let answer = self.call(
            PAGE_EVALUATE.name,
            &json!({ "page_id": page, "function": function }),
        )?;
        Ok(answer["result"]["value"].clone())
    }

    /// Navigates `page` to `url`, answering once its document committed
    /// when `commit`, else once it fired `load`; checks it answered 200.
    fn go(
        &mut self,
        page: &Value,
        url: &str,
        commit: bool,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        let mut params = json!({ "page_id": page, "url": url });
        if commit {
            params["wait_until"] = json!("commit");
        }
        let went = self.call(PAGE_GOTO.name, &params)?;
        checks.check(
            format!("`{}` reaches {url}", PAGE_GOTO.name),
            went["result"]["response"]["status"] == 200,
        );
        Ok(())
    }

    /// A Run `run_id` whose `timeout.wait` is `wait` milliseconds, and a
    /// page of it.
    fn page_of_run(
        &mut self,
        site: &Site,
        run_id: &str,
        wait: u64,
        checks: &mut Checks,
    ) -> Result<Option<Value>, Gone> {
        start_run(
            self.connection,
            site,
            run_id,
            &json!({ "timeout": { "wait": wait } }),
            checks,
        )?;
        Ok(new_page(self.connection, run_id, checks)?.map(|page| json!(page.page_id)))
    }

    fn finish(&mut self, run_id: &str) -> Result<(), Gone> {
        finish_run(self.connection, run_id)
    }
}

/// The start URL of the task `task`, checked as served.
fn task_url(site: &Site, task: &str, checks: &mut Checks) -> Option<String> {
    let url = site.start_url(task).map(str::to_owned);
    checks.check(
        format!("the Proving ground serves the task `{task}`"),
        url.is_some(),
    );
    url
}

/// B02: a URL waiter armed before a `pushState`, a `replaceState` and a hash
/// change passes for each, on the single-page app whose link routes with
/// `pushState`.
fn url_within_the_document(
    exchange: &mut Exchange<'_>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-waits-url";
    let Some(url) = task_url(site, "navigation/1", checks) else {
        return Ok(());
    };
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    exchange.go(&page, &url, false, checks)?;
    let pushed = exchange.request(
        PAGE_WAIT_FOR_URL.name,
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/detail" } }),
    )?;
    let early = exchange.answered_at_once(pushed)?;
    checks.check(
        "the URL waiter waits while the URL does not match",
        early.is_none(),
    );
    let clicked = exchange.call(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page, "selector": "#go" } }),
    )?;
    let pushed = match early {
        Some(answer) => answer,
        None => exchange.answer(pushed)?,
    };
    checks.check(
        "the waiter armed before the link's `pushState` passes",
        clicked.get("result").is_some() && pushed.get("result").is_some(),
    );
    for (name, pattern, function) in [
        (
            "a `replaceState`",
            json!({ "kind": "regex", "pattern": "/replaced$" }),
            "() => history.replaceState({}, '', '/replaced')",
        ),
        (
            "a hash change",
            json!({ "kind": "glob", "pattern": "**/replaced#section" }),
            "() => { location.hash = 'section'; }",
        ),
    ] {
        let waiting = exchange.request(
            PAGE_WAIT_FOR_URL.name,
            &json!({ "page_id": page, "url": pattern }),
        )?;
        exchange.evaluated(&page, function)?;
        let passed = exchange.answer(waiting)?;
        checks.check(
            format!("the waiter armed before {name} passes"),
            passed.get("result").is_some(),
        );
    }
    exchange.finish(run_id)
}

/// B03: `networkidle` comes on the live board, whose WebSocket and
/// server-sent events stay open, once its periodic requests stopped, and
/// on the late board only once its request, started at `load` and held
/// past the 500 ms of quiet, finished,
/// on a page framing another site, whose frame the browser loads in a
/// process of its own, and at once on a committed `about:blank`.
fn network_idle(exchange: &mut Exchange<'_>, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-waits-network-idle";
    let Some(live) = task_url(site, "live/1", checks) else {
        return Ok(());
    };
    let late = live.replace("/board", "/late");
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    for (name, url, updates) in [("live", &live, "6"), ("late", &late, "1")] {
        exchange.go(&page, url, false, checks)?;
        let idle = exchange.call(
            PAGE_WAIT_FOR_LOAD_STATE.name,
            &json!({ "page_id": page, "state": "networkidle" }),
        )?;
        let board = exchange.evaluated(
            &page,
            "() => ['state', 'updates', 'socket', 'events'].map(id => document.getElementById(id).textContent)",
        )?;
        checks.check(
            format!("`networkidle` comes on the {name} board"),
            idle.get("result").is_some(),
        );
        checks.check(
            format!("the {name} board's requests all finished before it, the last being {updates}"),
            board[0] == "settled" && board[1] == updates,
        );
        if name == "live" {
            checks.check(
                "the live board's WebSocket and event stream were open, and left out",
                board[2] != "0" && board[3] != "0",
            );
        }
    }
    let framing = site.named_url(clean_up::HOST, "/cross-site");
    exchange.go(&page, &framing, false, checks)?;
    let idle = exchange.call(
        PAGE_WAIT_FOR_LOAD_STATE.name,
        &json!({ "page_id": page, "state": "networkidle" }),
    )?;
    let framed = exchange.evaluated(&page, "() => document.querySelector('iframe').src")?;
    checks.check(
        "`networkidle` comes on a page framing another site, whose frame runs in a process of its own",
        idle.get("result").is_some()
            && framed
                .as_str()
                .is_some_and(|src| src.contains(clean_up::FRAME_HOST)),
    );
    let blank = exchange.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page, "url": "about:blank" }),
    )?;
    let idle = exchange.call(
        PAGE_WAIT_FOR_LOAD_STATE.name,
        &json!({ "page_id": page, "state": "networkidle" }),
    )?;
    checks.check(
        "`networkidle` comes on a committed `about:blank`",
        blank.get("result").is_some() && idle.get("result").is_some(),
    );
    exchange.finish(run_id)
}

/// B04: a response waiter and a request waiter sent before the click whose
/// effect posts a request catch it, though the response came before either
/// answer is read.
fn response_before_awaited(
    exchange: &mut Exchange<'_>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-waits-response";
    let Some(url) = task_url(site, "navigation/1", checks) else {
        return Ok(());
    };
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    exchange.go(&page, &url, false, checks)?;
    let effects = json!({ "kind": "glob", "pattern": "**/effects" });
    let responded = exchange.request(
        PAGE_WAIT_FOR_RESPONSE.name,
        &json!({ "page_id": page, "url": effects }),
    )?;
    let requested = exchange.request(
        PAGE_WAIT_FOR_REQUEST.name,
        &json!({ "page_id": page, "url": effects }),
    )?;
    let clicked = exchange.call(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page, "selector": "#go" } }),
    )?;
    // The page shows `done` once its post was answered: the response came
    // before the program reads either waiter's answer.
    let done = exchange.call(
        PAGE_WAIT_FOR_FUNCTION.name,
        &json!({ "page_id": page, "function": "() => document.querySelector('#done').textContent === 'done'" }),
    )?;
    let response = exchange.answer(responded)?;
    let request = exchange.answer(requested)?;
    checks.check(
        "the click answers, and the page saw its post answered",
        clicked.get("result").is_some() && done["result"]["value"] == true,
    );
    let posted = |url: &Value| url.as_str().is_some_and(|url| url.ends_with("/effects"));
    checks.check(
        "the response waiter answers the post's request and response",
        response["result"]["request"]["method"] == "POST"
            && posted(&response["result"]["response"]["url"]),
    );
    checks.check(
        "the request waiter answers the post",
        request["result"]["method"] == "POST" && posted(&request["result"]["url"]),
    );
    exchange.finish(run_id)
}

/// `locator.wait_for` and `locator.wait_for_count` resolve on the
/// actionability pages that show, replace and de-duplicate their target
/// after they load, each waited for from the document's commit, and
/// `page.wait_for_function` and `locator.wait_for_function` answer the
/// first truthy value.
fn re_rendering_pages(
    exchange: &mut Exchange<'_>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-waits-re-rendering";
    let (Some(hidden), Some(rerender), Some(ambiguous)) = (
        task_url(site, "actionability/4", checks),
        task_url(site, "actionability/6", checks),
        task_url(site, "actionability/27", checks),
    ) else {
        return Ok(());
    };
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    let locator = |selector: &str| json!({ "page_id": page, "selector": selector });

    exchange.go(&page, &hidden, true, checks)?;
    let visible = exchange.call(
        LOCATOR_WAIT_FOR.name,
        &json!({ "locator": locator("#target"), "state": "visible" }),
    )?;
    let shown = exchange.evaluated(
        &page,
        "() => getComputedStyle(document.querySelector('#target')).display",
    )?;
    checks.check(
        "`wait_for` visible passes once the hidden target shows",
        visible.get("result").is_some() && shown != "none",
    );

    exchange.go(&page, &hidden, true, checks)?;
    let function = exchange.call(
        PAGE_WAIT_FOR_FUNCTION.name,
        &json!({
            "page_id": page,
            "function": "(id) => { const target = document.getElementById(id); return target !== null && getComputedStyle(target).display !== 'none' && target.textContent; }",
            "arg": "target",
            "polling_interval": 20,
        }),
    )?;
    checks.check(
        "`wait_for_function` answers its first truthy value",
        function["result"]["value"] == "Target",
    );

    exchange.go(&page, &rerender, true, checks)?;
    let detached = exchange.call(
        LOCATOR_WAIT_FOR.name,
        &json!({ "locator": locator("#target[disabled]"), "state": "detached" }),
    )?;
    let replaced = exchange.evaluated(
        &page,
        "() => document.querySelectorAll('#target[disabled]').length",
    )?;
    checks.check(
        "`wait_for` detached passes once the page replaced its disabled target",
        detached.get("result").is_some() && replaced == 0,
    );

    exchange.go(&page, &ambiguous, true, checks)?;
    let counted = exchange.call(
        LOCATOR_WAIT_FOR_COUNT.name,
        &json!({ "locator": locator("#target"), "count": { "min": 1, "max": 1 } }),
    )?;
    checks.check(
        "`wait_for_count` answers 1 once the transient duplicate left",
        counted["result"]["count"] == 1,
    );
    locator_function_waits(exchange, &page, (&hidden, &rerender, &ambiguous), checks)?;
    exchange.finish(run_id)
}

/// `locator.wait_for_function` hands its function the one element its
/// locator matches, resolved again at each check: it passes once the hidden
/// target shows, on the element a re-render put in place of the disabled
/// one, and once a transient duplicate left, never calling the function
/// while several match.
fn locator_function_waits(
    exchange: &mut Exchange<'_>,
    page: &Value,
    (hidden, rerender, ambiguous): (&str, &str, &str),
    checks: &mut Checks,
) -> Result<(), Gone> {
    let locator = |selector: &str| json!({ "page_id": page, "selector": selector });
    exchange.go(page, hidden, true, checks)?;
    let element = exchange.call(
        LOCATOR_WAIT_FOR_FUNCTION.name,
        &json!({
            "locator": locator("#target"),
            "function": "(target, display) => getComputedStyle(target).display !== display && target.textContent",
            "arg": "none",
            "polling_interval": 20,
        }),
    )?;
    checks.check(
        "`locator.wait_for_function` hands the element first and answers its first truthy value",
        element["result"]["value"] == "Target",
    );

    exchange.go(page, rerender, true, checks)?;
    let enabled = exchange.call(
        LOCATOR_WAIT_FOR_FUNCTION.name,
        &json!({
            "locator": locator("#target"),
            "function": "(target) => !target.disabled && target.isConnected",
            "polling_interval": 20,
        }),
    )?;
    checks.check(
        "`locator.wait_for_function` resolves its locator again at each check, so it passes on the element a re-render put in place of the disabled one",
        enabled["result"]["value"] == true,
    );
    exchange.go(page, ambiguous, true, checks)?;
    let unique = exchange.call(
        LOCATOR_WAIT_FOR_FUNCTION.name,
        &json!({
            "locator": locator("#target"),
            "function": "(target) => document.querySelectorAll('#target').length === 1 && target.id",
            "polling_interval": 20,
        }),
    )?;
    checks.check(
        "`locator.wait_for_function` checks nothing while several elements match, and passes once one does",
        unique["result"]["value"] == "target",
    );
    Ok(())
}

/// Each wait ends at `timeout.wait`, on the board whose network never
/// idles, with `deadline_exceeded` naming what it waited for.
fn deadlines(exchange: &mut Exchange<'_>, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-waits-deadlines";
    let Some(board) = task_url(site, "live/2", checks) else {
        return Ok(());
    };
    let Some(page) = exchange.page_of_run(site, run_id, SHORT_WAIT, checks)? else {
        return Ok(());
    };
    exchange.go(&page, &board, false, checks)?;
    let mut sent = Vec::new();
    for (method, params, waited_for) in never_passing(&page) {
        sent.push((method, exchange.request(method, &params)?, waited_for));
    }
    for (method, id, waited_for) in sent {
        let answer = exchange.answer(id)?;
        let data = &answer["error"]["data"];
        checks.check(
            format!(
                "`{method}` ends with `{}` naming `{waited_for}` and `{}`, at its deadline",
                DEADLINE_EXCEEDED.name, TIMEOUT_WAIT.name
            ),
            data["code"] == DEADLINE_EXCEEDED.name
                && data["waited_for"] == waited_for
                && data["limit"]["key"] == TIMEOUT_WAIT.name
                && data["elapsed"]
                    .as_u64()
                    .is_some_and(|elapsed| elapsed >= SHORT_WAIT),
        );
    }
    exchange.finish(run_id)
}

/// Each wait, with what it waits for on `page`, the live board, where none
/// passes: its operation, its params and the `waited_for` its deadline
/// names.
fn never_passing(page: &Value) -> [(&'static str, Value, &'static str); 9] {
    let never = json!({ "kind": "glob", "pattern": "**/never" });
    let locator = |selector: &str| json!({ "page_id": page, "selector": selector });
    [
        (
            PAGE_WAIT_FOR_URL.name,
            json!({ "page_id": page, "url": never }),
            "url",
        ),
        (
            PAGE_WAIT_FOR_REQUEST.name,
            json!({ "page_id": page, "url": never }),
            "request",
        ),
        (
            PAGE_WAIT_FOR_RESPONSE.name,
            json!({ "page_id": page, "url": never }),
            "response",
        ),
        (
            PAGE_WAIT_FOR_REQUEST_FINISHED.name,
            json!({ "page_id": page, "url": never }),
            "finished request",
        ),
        (
            PAGE_WAIT_FOR_LOAD_STATE.name,
            json!({ "page_id": page, "state": "networkidle" }),
            "networkidle",
        ),
        (
            PAGE_WAIT_FOR_FUNCTION.name,
            json!({ "page_id": page, "function": "() => false" }),
            "function",
        ),
        (
            LOCATOR_WAIT_FOR_FUNCTION.name,
            json!({ "locator": locator("[data-id=board]"), "function": "(board) => board.id === 'never'" }),
            "function",
        ),
        (
            LOCATOR_WAIT_FOR.name,
            json!({ "locator": locator("#never"), "state": "attached" }),
            "target_attached",
        ),
        (
            LOCATOR_WAIT_FOR_COUNT.name,
            json!({ "locator": locator("[data-id=board]"), "count": { "min": 2 } }),
            "target_count",
        ),
    ]
}

/// A wait's own `timeout` is its deadline in place of `timeout.wait`, which
/// its `deadline_exceeded` names at the run layer with the call's value;
/// one whose `timeout` is `null` has none, and is still waiting once the
/// Run's `timeout.wait` passed.
fn own_timeouts(exchange: &mut Exchange<'_>, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-waits-own-timeouts";
    let Some(board) = task_url(site, "live/2", checks) else {
        return Ok(());
    };
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    exchange.go(&page, &board, false, checks)?;
    let mut sent = Vec::new();
    for (method, mut params, waited_for) in never_passing(&page) {
        params["timeout"] = json!(SHORT_WAIT);
        sent.push((method, exchange.request(method, &params)?, waited_for));
    }
    for (method, id, waited_for) in sent {
        let answer = exchange.answer_within(id, Duration::from_millis(LONG_WAIT / 2))?;
        let data = &answer["error"]["data"];
        checks.check(
            format!(
                "`{method}` given `timeout` ends with `{}` naming `{waited_for}` and `{}` at the run layer, at the call's deadline",
                DEADLINE_EXCEEDED.name, TIMEOUT_WAIT.name
            ),
            data["code"] == DEADLINE_EXCEEDED.name
                && data["waited_for"] == waited_for
                && data["limit"]
                    == json!({ "key": TIMEOUT_WAIT.name, "value": SHORT_WAIT, "layer": "run" })
                && data["elapsed"]
                    .as_u64()
                    .is_some_and(|elapsed| (SHORT_WAIT..LONG_WAIT).contains(&elapsed)),
        );
    }
    exchange.finish(run_id)?;

    let run_id = "conformance-waits-no-timeout";
    let Some(page) = exchange.page_of_run(site, run_id, SHORT_WAIT, checks)? else {
        return Ok(());
    };
    exchange.go(&page, &board, false, checks)?;
    let never = json!({ "kind": "glob", "pattern": "**/never" });
    let unbounded = exchange.request(
        PAGE_WAIT_FOR_URL.name,
        &json!({ "page_id": page, "url": never, "timeout": null }),
    )?;
    let bounded = exchange.request(
        PAGE_WAIT_FOR_URL.name,
        &json!({ "page_id": page, "url": never }),
    )?;
    let ended = exchange.answer(bounded)?;
    let waiting = exchange.answered_at_once(unbounded)?;
    checks.check(
        "a wait whose `timeout` is `null` is still waiting once `timeout.wait` ended another",
        ended["error"]["data"]["code"] == DEADLINE_EXCEEDED.name && waiting.is_none(),
    );
    let refused = exchange.call(
        PAGE_WAIT_FOR_URL.name,
        &json!({ "page_id": page, "url": never, "timeout": 0 }),
    )?;
    checks.check(
        "a `timeout` of 0 is refused with `invalid_params`",
        refused["error"]["code"] == json!(-32602),
    );
    exchange.finish(run_id)
}

/// Each wait in flight when its page closes ends with `call_cancelled`,
/// stopped by the program, naming the page as what closed.
fn page_closes(exchange: &mut Exchange<'_>, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-waits-page-closes";
    let Some(board) = task_url(site, "live/2", checks) else {
        return Ok(());
    };
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    exchange.go(&page, &board, false, checks)?;
    let mut sent = Vec::new();
    for (method, params, _) in never_passing(&page) {
        sent.push((method, exchange.request(method, &params)?));
    }
    let early = exchange.answered_at_once(sent[0].1)?;
    let closed = exchange.call(PAGE_CLOSE.name, &json!({ "page_id": page }))?;
    checks.check(
        "the waits are in flight, and the page closes",
        early.is_none() && closed.get("result").is_some(),
    );
    for (method, id) in sent {
        let answer = exchange.answer(id)?;
        checks.check(
            format!(
                "`{method}` in flight ends with `{}` by the program, naming the page",
                CALL_CANCELLED.name
            ),
            answer["error"]["data"]["code"] == CALL_CANCELLED.name
                && answer["error"]["data"]["by"] == "program"
                && answer["error"]["data"]["closed"] == "page",
        );
    }
    exchange.finish(run_id)
}

/// A route whose regular expression uses look-around and a URL wait whose
/// pattern uses a back-reference are refused when their calls arrive, with
/// `pattern_unsupported` naming the construct, never later when a request
/// or a navigation would have been matched (ADR 0081).
fn unsupported_patterns(
    exchange: &mut Exchange<'_>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-waits-unsupported-patterns";
    let Some(page) = exchange.page_of_run(site, run_id, LONG_WAIT, checks)? else {
        return Ok(());
    };
    let refusals = [
        (
            PAGE_ROUTE.name,
            json!({ "page_id": page, "url": { "kind": "regex", "pattern": "/items(?=x)" } }),
            "look_around",
        ),
        (
            PAGE_WAIT_FOR_URL.name,
            json!({ "page_id": page, "url": { "kind": "regex", "pattern": r"/(\w+)/\1" } }),
            "backreference",
        ),
    ];
    for (method, params, construct) in refusals {
        let id = exchange.request(method, &params)?;
        let refused = exchange.answered_at_once(id)?;
        let data = refused.as_ref().map(|refused| &refused["error"]["data"]);
        checks.check(
            format!(
                "`{method}` is refused at once with `{}` naming `{construct}`",
                PATTERN_UNSUPPORTED.name
            ),
            data.is_some_and(|data| {
                data["code"] == PATTERN_UNSUPPORTED.name && data["construct"] == construct
            }),
        );
    }
    exchange.finish(run_id)
}
