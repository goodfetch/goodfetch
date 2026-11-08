//! The guard's cases (ADRs 0019 and 0085), Page API cases on the Proving
//! ground's actionability pages: each opens a page of its own Run on a task,
//! acts through the locator actions, and asks the task's oracle which
//! control received which event, and whether it was trusted. A click waits
//! out a covering element and a re-render's transient duplicate, a locator
//! survives a re-render that replaces its element, typed text and a key
//! reach the page as trusted input, and a target still ambiguous or never
//! actionable when `timeout.action` passes fails typed, having done nothing,
//! also on a page whose main thread is too busy to answer.

use std::time::{Duration, Instant};

use goodfetch_protocol::connection::DEADLINE_EXCEEDED;
use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::{
    Acted, LOCATOR_CLICK, LOCATOR_FILL, LOCATOR_PRESS, PAGE_GOTO, TARGET_NOT_UNIQUE,
    TargetNotUniqueDetails,
};
use goodfetch_protocol::result_document::OracleOutcome;
use goodfetch_protocol::settings::TIMEOUT_ACTION;
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, poll, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::Submission;

/// How long a case lets an action wait when it expects it to fail: 1 s.
const SHORT_WAIT: u64 = 1_000;

/// How long it lets an action wait when it expects it to land: 10 s.
const LONG_WAIT: u64 = 10_000;

/// How long the oracle is asked again for an Effect a page posts after its
/// event.
const EFFECT_WAIT: Duration = Duration::from_secs(10);

/// One locator action.
#[derive(Clone, Copy)]
pub(crate) enum Step {
    Click(&'static str),
    Fill(&'static str, &'static str),
    Press(&'static str, &'static str),
}

/// A page of the Run `run_id` on the task `task`, `timeout.action` being
/// `action_timeout` milliseconds.
fn on_task(
    connection: &mut Connection,
    site: &Site,
    task: &str,
    run_id: &str,
    action_timeout: u64,
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    let Some(url) = site.start_url(task).map(str::to_owned) else {
        checks.check(
            format!("the Proving ground serves the task `{task}`"),
            false,
        );
        return Ok(None);
    };
    start_run(
        connection,
        site,
        run_id,
        &json!({ "timeout": { "action": action_timeout } }),
        checks,
    )?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(None);
    };
    let went = connection.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    checks.check(
        format!("`{}` reaches the task's page", PAGE_GOTO.name),
        went["result"]["response"]["status"] == 200,
    );
    Ok(Some(json!(page.page_id)))
}

/// Sends `step` on `page`, and answers the engine's answer.
fn act(connection: &mut Connection, page: &Value, step: Step) -> Result<Value, Gone> {
    let locator = |selector: &str| json!({ "page_id": page, "selector": selector });
    let (method, params) = match step {
        Step::Click(selector) => (LOCATOR_CLICK.name, json!({ "locator": locator(selector) })),
        Step::Fill(selector, value) => (
            LOCATOR_FILL.name,
            json!({ "locator": locator(selector), "value": value }),
        ),
        Step::Press(selector, key) => (
            LOCATOR_PRESS.name,
            json!({ "locator": locator(selector), "key": key }),
        ),
    };
    let id = connection.request(method, &params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// The oracle's outcome for `task` reported as `status`, asked again until
/// it verifies or [`EFFECT_WAIT`] passes, since a page posts its Effect
/// after its event; none when the oracle cannot judge it, which fails the
/// case naming why.
fn outcome(site: &Site, task: &str, status: &str) -> Option<OracleOutcome> {
    let id = TaskId::try_from(task.to_owned()).expect("invariant: a case names its task by ID");
    poll::try_until(
        EFFECT_WAIT,
        Duration::from_millis(100),
        || {
            site.proving_ground
                .judge::<Value>(&Submission {
                    task: &id,
                    status,
                    records: &[],
                })
                .map(|answer| answer.outcome)
        },
        |outcome| *outcome == OracleOutcome::Verified,
    )
    .map_err(|why| {
        site.unread(format!(
            "the Proving ground's oracle judges `{task}` ({why})"
        ));
    })
    .ok()
}

/// The task `task` done through `steps`: each action answers an empty
/// result, no handle on an element, and the oracle verifies the task.
pub(crate) fn lands(
    connection: &mut Connection,
    site: &Site,
    task: &str,
    run_id: &str,
    steps: &[Step],
    checks: &mut Checks,
) -> Result<(), Gone> {
    if let Some(page) = on_task(connection, site, task, run_id, LONG_WAIT, checks)? {
        for step in steps {
            let answer = act(connection, &page, *step)?;
            checks.check(
                "the action answers an empty result, no handle on an element",
                answer
                    .get("result")
                    .and_then(|result| serde_json::from_value::<Acted>(result.clone()).ok())
                    .is_some_and(|_| answer["result"] == json!({})),
            );
        }
        checks.check(
            format!("the oracle verifies `{task}`: one trusted event on the right control"),
            outcome(site, task, "success") == Some(OracleOutcome::Verified),
        );
    }
    finish_run(connection, run_id)
}

/// A click on the two buttons of task 9 still raises `target_not_unique`
/// when `timeout.action` passes, listing both, and clicks neither.
pub(crate) fn still_ambiguous(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (task, run_id) = ("actionability/9", "conformance-still-ambiguous");
    if let Some(page) = on_task(connection, site, task, run_id, SHORT_WAIT, checks)? {
        let answer = act(connection, &page, Step::Click("#target"))?;
        let data = &answer["error"]["data"];
        checks.check(
            format!("the click raises `{}`", TARGET_NOT_UNIQUE.name),
            data["code"] == TARGET_NOT_UNIQUE.name,
        );
        let details = serde_json::from_value::<TargetNotUniqueDetails>(data.clone()).ok();
        checks.check(
            "its details list both matches, and `limit` names `timeout.action` at the run layer",
            details.is_some_and(|details| {
                details.matches.len() == 2
                    && details.limit.key == TIMEOUT_ACTION.name
                    && details.limit.layer.as_str() == "run"
            }),
        );
        checks.check(
            "the oracle sees no click on either button",
            outcome(site, task, "failure") == Some(OracleOutcome::Verified),
        );
    }
    finish_run(connection, run_id)
}

/// A click on task 7's button, never enabled, raises `deadline_exceeded`
/// naming the step it waited for, and clicks nothing.
pub(crate) fn never_actionable(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (task, run_id) = ("actionability/7", "conformance-never-actionable");
    if let Some(page) = on_task(connection, site, task, run_id, SHORT_WAIT, checks)? {
        let answer = act(connection, &page, Step::Click("#target"))?;
        let data = &answer["error"]["data"];
        checks.check(
            format!("the click raises `{}`", DEADLINE_EXCEEDED.name),
            data["code"] == DEADLINE_EXCEEDED.name,
        );
        checks.check(
            "it names the step it waited for, `target_enabled`, and `timeout.action`",
            data["waited_for"] == "target_enabled" && data["limit"]["key"] == TIMEOUT_ACTION.name,
        );
        checks.check(
            "the oracle sees no click",
            outcome(site, task, "failure") == Some(OracleOutcome::Verified),
        );
    }
    finish_run(connection, run_id)
}

/// How long the busy page's script keeps its main thread busy: far past
/// the action's [`SHORT_WAIT`].
const BUSY: u64 = 8_000;

/// How long the case lets the busy script start before it acts.
const BUSY_START: Duration = Duration::from_millis(300);

/// A click on a page whose main thread a script keeps busy past
/// `timeout.action` raises `deadline_exceeded` naming `timeout.action` once
/// the deadline passes, not once the page answers again (ADR 0080).
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness times the call on its own thread, outside any async worker"
)]
pub(crate) fn busy_page(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (task, run_id) = ("actionability/7", "conformance-busy-page");
    if let Some(page) = on_task(connection, site, task, run_id, SHORT_WAIT, checks)? {
        connection.request(
            PAGE_EVALUATE.name,
            &json!({
                "page_id": page,
                "function": format!("() => {{ const end = Date.now() + {BUSY}; while (Date.now() < end) {{}} }}"),
                "world": "main",
            }),
        )?;
        std::thread::sleep(BUSY_START);
        let started = Instant::now();
        let answer = act(connection, &page, Step::Click("#target"))?;
        let waited = started.elapsed();
        let data = &answer["error"]["data"];
        checks.check(
            format!("the click raises `{}`", DEADLINE_EXCEEDED.name),
            data["code"] == DEADLINE_EXCEEDED.name,
        );
        checks.check(
            "it names `timeout.action`",
            data["limit"]["key"] == TIMEOUT_ACTION.name,
        );
        checks.check(
            "it raises once its deadline passed, while the page is still busy",
            waited < Duration::from_millis(BUSY / 2),
        );
    }
    finish_run(connection, run_id)
}
