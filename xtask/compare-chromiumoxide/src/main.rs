//! One trial of the Collie scenario of `just compare` with chromiumoxide
//! (#2444), a Rust CDP library, attached to the Chrome that `xtask compare`
//! launched, through the counting relay at `--cdp`. It carries out the plan
//! `--plan` names as `xtask/compare-collie/plan.ts` does for the scenario's
//! other tools, the same loop step for step:
//!
//! - each page of `pages` opens in the trial's one Session, goes to its URL
//!   and takes its steps, then closes; one after another, or all at once
//!   when `at_once` is set;
//! - with `sessions` set, that many Sessions open one after another, each
//!   with one page, and close;
//! - with `openings` set, that many pages open one after another in the
//!   trial's one Session, each kept open until the last is.
//!
//! Around the measured part it asks `--marks` for `/start` and `/end`.
//! Connecting stays outside the marks. A Session is a browser context
//! (`Browser::create_browser_context`), closed with
//! `Browser::dispose_browser_context`. chromiumoxide runs at its defaults
//! (`HandlerConfig::default()`), as it ships.
//!
//! A step is chromiumoxide's own calls, none of which waits for its target
//! nor checks it: `Page::find_element` (`DOM.querySelector`, which fails at
//! once when nothing matches), then `Element::click()` (scroll into view,
//! a clickable point from the content quads, the mouse events),
//! `Element::click()` then `Element::type_str()` for a fill, as its own
//! documentation types into a field, and `Element::focus()` then
//! `Element::press_key()` for a press. A page's status is its navigation's
//! response, which `Page::http_future` resolves once the page has loaded.
//!
//! It prints one JSON line of what each page came to, as `plan.ts` does.
//!
//!   goodfetch-compare-chromiumoxide --plan plan.json \
//!     --cdp ws://127.0.0.1:<relay>/devtools/browser/<id> \
//!     --marks http://127.0.0.1:<relay>

use std::process::ExitCode;

use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
use chromiumoxide::cdp::browser_protocol::target::{
    CreateBrowserContextParams, CreateTargetParams,
};
use chromiumoxide::{Browser, Page};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};

/// What the page reads: its title, which the served page sets, as every
/// other tool of the scenario reads it (`READ` in `plan.ts`).
const READ: &str = "document.title";

const USAGE: &str =
    "usage: goodfetch-compare-chromiumoxide --plan <plan.json> --cdp <ws url> --marks <http url>";

/// One step on a page, as xtask writes it.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum Step {
    Click { selector: String },
    Fill { selector: String, value: String },
    Press { selector: String, key: String },
    Read,
}

/// One page of the plan: a Proving ground task's page and its steps.
#[derive(Deserialize)]
struct PlannedPage {
    task: String,
    url: String,
    steps: Vec<Step>,
}

/// What xtask asks of the trial. Its `action_timeout` is unread: no call
/// a step makes waits for its target.
#[derive(Deserialize)]
struct Plan {
    pages: Vec<PlannedPage>,
    at_once: bool,
    sessions: u64,
    openings: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Success,
    Failure,
}

/// What one page came to.
#[derive(Serialize)]
struct PageResult {
    task: String,
    outcome: Outcome,
    status: Option<i64>,
    steps: u64,
    read: Option<String>,
    error: Option<String>,
}

/// The line the trial prints.
#[derive(Serialize)]
struct Printed {
    pages: Vec<PageResult>,
    sessions: u64,
    opened: u64,
}

struct Arguments {
    plan: String,
    cdp: String,
    marks: String,
}

fn arguments() -> Result<Arguments, String> {
    let mut plan = None;
    let mut cdp = None;
    let mut marks = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(USAGE)?;
        match name.as_str() {
            "--plan" => plan = Some(value),
            "--cdp" => cdp = Some(value),
            "--marks" => marks = Some(value),
            _ => return Err(USAGE.to_owned()),
        }
    }
    match (plan, cdp, marks) {
        (Some(plan), Some(cdp), Some(marks)) => Ok(Arguments { plan, cdp, marks }),
        _ => Err(USAGE.to_owned()),
    }
}

/// A Session: a browser context of its own.
struct Session<'a> {
    browser: &'a Browser,
    context: BrowserContextId,
}

impl<'a> Session<'a> {
    async fn open(browser: &'a Browser) -> Result<Self, String> {
        let context = browser
            .create_browser_context(CreateBrowserContextParams::default())
            .await
            .map_err(|error| format!("opening a Session: {error}"))?;
        Ok(Self { browser, context })
    }

    async fn new_page(&self) -> Result<Page, String> {
        let mut params = CreateTargetParams::new("about:blank");
        params.browser_context_id = Some(self.context.clone());
        self.browser
            .new_page(params)
            .await
            .map_err(|error| format!("opening a page: {error}"))
    }

    async fn close(self) -> Result<(), String> {
        self.browser
            .dispose_browser_context(self.context)
            .await
            .map_err(|error| format!("closing a Session: {error}"))
    }
}

/// The first line of `error`'s message, as `plan.ts` keeps it.
fn first_line(error: &impl std::fmt::Display) -> String {
    error
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Takes `step` on `page`; the text it read, when it reads.
async fn step(page: &Page, step: &Step) -> chromiumoxide::Result<Option<String>> {
    match step {
        Step::Click { selector } => {
            page.find_element(selector.as_str()).await?.click().await?;
        }
        Step::Fill { selector, value } => {
            page.find_element(selector.as_str())
                .await?
                .click()
                .await?
                .type_str(value)
                .await?;
        }
        Step::Press { selector, key } => {
            page.find_element(selector.as_str())
                .await?
                .focus()
                .await?
                .press_key(key)
                .await?;
        }
        Step::Read => {
            let value: serde_json::Value = page.evaluate(READ).await?.into_value()?;
            return Ok(Some(value.as_str().unwrap_or_default().to_owned()));
        }
    }
    Ok(None)
}

/// Goes to `planned`'s URL on `page` and takes its steps.
async fn steps_on(page: &Page, planned: &PlannedPage) -> PageResult {
    let mut status = None;
    let mut read = None;
    let mut steps = 0;
    let mut error = None;
    match page.http_future(NavigateParams::new(planned.url.as_str())) {
        Err(failed) => error = Some(first_line(&failed)),
        Ok(navigation) => match navigation.await {
            Err(failed) => error = Some(first_line(&failed)),
            Ok(request) => {
                status = request.and_then(|request| request.response.as_ref().map(|r| r.status));
                for planned_step in &planned.steps {
                    steps += 1;
                    match step(page, planned_step).await {
                        Ok(Some(text)) => read = Some(text),
                        Ok(None) => {}
                        Err(failed) => {
                            error = Some(first_line(&failed));
                            break;
                        }
                    }
                }
            }
        },
    }
    PageResult {
        task: planned.task.clone(),
        outcome: if error.is_none() {
            Outcome::Success
        } else {
            Outcome::Failure
        },
        status,
        steps,
        read,
        error,
    }
}

/// Opens a page of `session` on `planned`, takes its steps and closes it.
async fn on_page(session: &Session<'_>, planned: &PlannedPage) -> Result<PageResult, String> {
    let page = session.new_page().await?;
    let result = steps_on(&page, planned).await;
    page.close()
        .await
        .map_err(|error| format!("closing a page: {error}"))?;
    Ok(result)
}

async fn mark(marks: &str, which: &str) -> Result<(), String> {
    let answer = reqwest::get(format!("{marks}/{which}"))
        .await
        .map_err(|error| format!("asking for the {which} mark: {error}"))?;
    if !answer.status().is_success() {
        return Err(format!("the {which} mark answered {}", answer.status()));
    }
    answer
        .bytes()
        .await
        .map_err(|error| format!("reading the {which} mark: {error}"))?;
    Ok(())
}

async fn carry_out(plan: &Plan, browser: &Browser, marks: &str) -> Result<Printed, String> {
    let mut pages = Vec::new();
    let mut sessions = 0;
    let mut opened = 0;
    if plan.openings > 0 {
        // One page first, outside the marks, as the Sessions cell's first
        // Session; the measured pages stay open until the end mark.
        let session = Session::open(browser).await?;
        let first = session.new_page().await?;
        mark(marks, "start").await?;
        let mut kept = Vec::new();
        while opened < plan.openings {
            kept.push(session.new_page().await?);
            opened += 1;
        }
        mark(marks, "end").await?;
        for page in kept {
            page.close()
                .await
                .map_err(|error| format!("closing a page: {error}"))?;
        }
        first
            .close()
            .await
            .map_err(|error| format!("closing a page: {error}"))?;
        session.close().await?;
    } else if plan.sessions > 0 {
        // One Session first, outside the marks, so the tool is attached to
        // the browser before the openings it measures.
        let first = Session::open(browser).await?;
        mark(marks, "start").await?;
        while sessions < plan.sessions {
            let session = Session::open(browser).await?;
            let page = session.new_page().await?;
            page.close()
                .await
                .map_err(|error| format!("closing a page: {error}"))?;
            session.close().await?;
            sessions += 1;
        }
        mark(marks, "end").await?;
        first.close().await?;
    } else {
        let session = Session::open(browser).await?;
        mark(marks, "start").await?;
        if plan.at_once {
            pages = futures::future::join_all(
                plan.pages.iter().map(|planned| on_page(&session, planned)),
            )
            .await
            .into_iter()
            .collect::<Result<_, _>>()?;
        } else {
            for planned in &plan.pages {
                pages.push(on_page(&session, planned).await?);
            }
        }
        mark(marks, "end").await?;
        session.close().await?;
    }
    Ok(Printed {
        pages,
        sessions,
        opened,
    })
}

async fn trial(arguments: Arguments) -> Result<String, String> {
    let text = std::fs::read_to_string(&arguments.plan)
        .map_err(|error| format!("reading {}: {error}", arguments.plan))?;
    let plan: Plan = serde_json::from_str(&text)
        .map_err(|error| format!("the plan is not the shape xtask writes: {error}"))?;
    let (browser, mut handler) = Browser::connect(arguments.cdp.as_str())
        .await
        .map_err(|error| format!("attaching to {}: {error}", arguments.cdp))?;
    // chromiumoxide's handler drives the connection: it runs until the
    // connection closes. A bare `tokio::spawn`, as Spider's side's: this
    // executable stands outside the workspace and has no tracker; the task
    // is aborted below, so none outlives the trial.
    let handling = tokio::spawn(async move { while handler.next().await.is_some() {} });
    let printed = carry_out(&plan, &browser, &arguments.marks).await;
    // Detaching leaves the browser running: xtask stops it.
    drop(browser);
    handling.abort();
    let printed = printed?;
    serde_json::to_string(&printed).map_err(|error| format!("writing the result: {error}"))
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = match arguments() {
        Ok(arguments) => arguments,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    match trial(arguments).await {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
