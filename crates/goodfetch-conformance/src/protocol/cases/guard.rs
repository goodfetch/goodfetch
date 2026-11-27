//! The guard's cases (ADRs 0019 and 0085), Page API cases on the Proving
//! ground's actionability pages: each opens a page of its own Run on a task,
//! acts through the locator actions, and asks the task's oracle which
//! control received which event, and whether it was trusted. A click waits
//! out a covering element and a re-render's transient duplicate, a locator
//! survives a re-render that replaces its element, typed text and a key
//! reach the page as trusted input, and a target still ambiguous or never
//! actionable when `timeout.action` passes fails typed, having done nothing,
//! also on a page whose main thread is too busy to answer. One case writes
//! its own blank page instead: two fills sent at once on it type their own
//! text while reads sent with them answer.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use goodfetch_protocol::connection::DEADLINE_EXCEEDED;
use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::read::PAGE_READ;
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

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    ClickWaitsOutACover,
    TransientDuplicateIsWaitedOut,
    LocatorSurvivesARerender,
    TypedTextReachesAControlledField,
    KeyReachesAField,
    FillsTakeTurns,
    StillAmbiguous,
    NeverActionable,
    BusyPage,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-click-on-a-covered-target-lands-once-the-cover-leaves" => Self::ClickWaitsOutACover,
            "an-ambiguous-target-that-becomes-unique-is-acted-on" => {
                Self::TransientDuplicateIsWaitedOut
            }
            "a-locator-survives-a-re-render-that-replaces-its-element" => {
                Self::LocatorSurvivesARerender
            }
            "typed-text-reaches-a-controlled-field-as-trusted-input" => {
                Self::TypedTextReachesAControlledField
            }
            "a-pressed-key-reaches-a-field-as-a-trusted-key" => Self::KeyReachesAField,
            "two-fills-sent-at-once-type-their-own-text-while-reads-answer" => Self::FillsTakeTurns,
            "a-target-still-ambiguous-raises-target-not-unique" => Self::StillAmbiguous,
            "a-target-never-actionable-raises-deadline-exceeded-naming-the-step" => {
                Self::NeverActionable
            }
            "an-action-on-a-busy-page-raises-deadline-exceeded-at-its-deadline" => Self::BusyPage,
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
        match self {
            Self::ClickWaitsOutACover => lands(
                connection,
                site,
                "actionability/2",
                "conformance-cover",
                &[Step::Click("#target")],
                checks,
            ),
            Self::TransientDuplicateIsWaitedOut => lands(
                connection,
                site,
                "actionability/27",
                "conformance-transient-duplicate",
                &[Step::Click("#target")],
                checks,
            ),
            Self::LocatorSurvivesARerender => lands(
                connection,
                site,
                "actionability/6",
                "conformance-rerender",
                &[Step::Click("#target")],
                checks,
            ),
            Self::TypedTextReachesAControlledField => lands(
                connection,
                site,
                "actionability/28",
                "conformance-controlled-field",
                &[Step::Fill("#name", "Goodfetch"), Step::Click("#save")],
                checks,
            ),
            Self::KeyReachesAField => lands(
                connection,
                site,
                "actionability/15",
                "conformance-key",
                &[Step::Press("#field", "Enter")],
                checks,
            ),
            Self::FillsTakeTurns => fills_take_turns(connection, site, checks),
            Self::StillAmbiguous => still_ambiguous(connection, site, checks),
            Self::NeverActionable => never_actionable(connection, site, checks),
            Self::BusyPage => busy_page(connection, site, checks),
        }
    }
}

/// One locator action.
#[derive(Clone, Copy)]
enum Step {
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
fn lands(
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
fn still_ambiguous(
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
fn never_actionable(
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
            format!(
                "it names the step it waited for, `target_enabled`, and `{}`",
                TIMEOUT_ACTION.name
            ),
            data["waited_for"] == "target_enabled" && data["limit"]["key"] == TIMEOUT_ACTION.name,
        );
        checks.check(
            "the oracle sees no click",
            outcome(site, task, "failure") == Some(OracleOutcome::Verified),
        );
    }
    finish_run(connection, run_id)
}

/// How many reads [`fills_take_turns`] sends with its two fills.
const READS: usize = 10;

/// What [`fills_take_turns`] writes into its blank page: two fields, and
/// [`READS`] paragraphs, the one at `index` reading `item <index>`.
const TWO_FIELDS: &str = "() => { document.body.innerHTML = '<input id=\"first\"><input id=\"second\">' \
    + Array.from({ length: 10 }, (_, index) => `<p id=\"item-${index}\">item ${index}</p>`).join(''); }";

/// The texts [`fills_take_turns`] types into its first and second field.
const TYPED: [&str; 2] = ["typed into the first field", "typed into the second field"];

/// Two fills sent at once on two fields of one page each type their own
/// text, none of the other's, while ten reads of an element's text sent
/// with them each answer their element's (ADR 0085).
fn fills_take_turns(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-fills-take-turns";
    start_run(
        connection,
        site,
        run_id,
        &json!({ "timeout": { "action": LONG_WAIT } }),
        checks,
    )?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let page = json!(page.page_id);
        let evaluate =
            |function: &str| json!({ "page_id": page, "function": function, "world": "main" });
        let written = connection.call(PAGE_EVALUATE.name, &evaluate(TWO_FIELDS))?;
        checks.check(
            format!(
                "`{}` writes two fields and ten paragraphs",
                PAGE_EVALUATE.name
            ),
            written.get("result").is_some(),
        );
        let mut sent = Vec::new();
        for (selector, text) in ["#first", "#second"].into_iter().zip(TYPED) {
            let fill =
                json!({ "locator": { "page_id": page, "selector": selector }, "value": text });
            sent.push(connection.request(LOCATOR_FILL.name, &fill)?);
        }
        for index in 0..READS {
            let field =
                json!({ "locator": { "kind": "css", "selector": format!("#item-{index}") } });
            let read = json!({ "page_id": page, "fields": { "text": field } });
            sent.push(connection.request(PAGE_READ.name, &read)?);
        }
        let answers = answers(connection, &sent)?;
        let (fills, reads) = sent.split_at(TYPED.len());
        checks.check(
            "each fill answers an empty result",
            fills.iter().all(|id| answers[id]["result"] == json!({})),
        );
        checks.check(
            "each read answers its paragraph's text",
            reads.iter().enumerate().all(|(index, id)| {
                answers[id]["result"]["values"]["text"] == format!("item {index}")
            }),
        );
        let values = connection.call(
            PAGE_EVALUATE.name,
            &evaluate("() => ['#first', '#second'].map((selector) => document.querySelector(selector).value)"),
        )?;
        checks.check(
            "each field holds its own fill's text, none of the other's",
            values["result"]["value"] == json!(TYPED),
        );
    }
    finish_run(connection, run_id)
}

/// The answers to the requests `sent`, by their ID, in whichever order the
/// engine gives them, each within a browser's deadline.
fn answers(connection: &mut Connection, sent: &[i64]) -> Result<BTreeMap<i64, Value>, Gone> {
    let mut answered = BTreeMap::new();
    for id in sent {
        if answered.contains_key(id) {
            continue;
        }
        let answer = connection.browser_answer(*id)?;
        for other in answer.others {
            if let (None, Some(other_id)) = (other.get("method"), other["id"].as_i64()) {
                answered.insert(other_id, other);
            }
        }
        answered.insert(*id, answer.message);
    }
    Ok(answered)
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
fn busy_page(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
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
