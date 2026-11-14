//! The Collie scenario of `xtask compare` (issue 2282): Collie against
//! Playwright on the Proving ground's `actionability.test` and
//! `portal.test` pages, in six cells (click, form, navigation, Sessions
//! opened, pages at once, pages opened), each tool carrying out the same
//! plan through `xtask/compare-collie/act.ts`; beside them the arms,
//! Puppeteer's two rows, Stagehand with no model and agent-browser's
//! command line, through `xtask/compare-collie-arms/act.ts` (issues 2443,
//! 2441 and 2442), and chromiumoxide, through the Rust program
//! `xtask/compare-chromiumoxide` builds (issue 2444), each read against the
//! Collie row the pages it got right place it at; and the page-load cell
//! ([`page_load`], issue 2319), in which each tool launches its own browser.
//!
//! In the six cells every tool attaches over CDP to one Chrome for Testing
//! that xtask launched, which no tool launched: through the counting relay
//! ([`super::relay`]), each tool's CDP commands, and the rounds it waited
//! on the browser, are counted the same way, from outside it, and the
//! browser's own processes are left out of each tool's CPU and memory.
//! Stagehand's extension runs in the browser's processes, so its CPU and
//! memory are not Stagehand's, while its CDP commands pass the relay.
//! agent-browser's commands each start a process that hands the command to
//! a daemon the first one started; the driver adopts the daemon, so its
//! CPU and memory are agent-browser's, and the CPU of each command's
//! process counts once the driver has waited for it.
//! What that leaves out of the Sessions cell is the browser's launch, which
//! `just counts` times for Collie at its defaults.
//!
//! Each trial runs one tool in a fresh process, against a fresh Proving
//! ground and a fresh browser, and takes its figures between the marks the
//! driver asks for once attached: CDP commands, the CPU time of the tool's
//! process tree and the wall time, each per action taken, beside the peak
//! resident memory of the tool's process tree. The oracle judges each
//! `actionability.test` page, which gives the pages each tool got right.
//! The same-work check holds every trial to its cell's plan and every page
//! to the status and the title most trials saw; the report gives
//! `collie.playwright_command_ratio` from the click and form cells, and
//! says when it is over the register row's Today.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial runs the browser and the tool as blocking child processes and waits by sleeping (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::result_document::OracleOutcome;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::check::Finding;

use super::relay::{Mark, Relay};
use super::sampler::{self, Sampler};
use super::trial::{self, Outcome};
use super::trials::{CollieScenario, CollieTrial, Contender, Ended, Exercise, PageDone};

pub(crate) mod page_load;
mod section;
mod tools;

use tools::adopt_orphans;
pub(crate) use tools::{Tools, browser_flags, skipped};

pub(crate) use section::{Section, markdown, section};

/// The cells, in the order the report gives them.
pub(crate) const EXERCISES: [Exercise; 6] = [
    Exercise::Click,
    Exercise::Form,
    Exercise::Navigation,
    Exercise::Sessions,
    Exercise::AtOnce,
    Exercise::Opening,
];
/// How long an action waits for its target, in both tools.
pub(crate) const ACTION_TIMEOUT_MILLISECONDS: u64 = 1000;
/// The Sessions the Sessions cell opens, after one outside the marks.
pub(crate) const SESSIONS: u64 = 10;
/// The pages the opening cell opens, after one outside the marks.
pub(crate) const OPENINGS: u64 = 10;
/// The register's row the click and form cells give (ADR 0140).
pub(crate) const RATIO_ROW: &str = "collie.playwright_command_ratio";
/// The cells whose command ratio the row reads.
const RATIO_CELLS: [Exercise; 2] = [Exercise::Click, Exercise::Form];
/// The variable Collie's configuration names its run-state key in: a trial
/// keeps no Run, so any key does.
pub(crate) const RUN_STATE_KEY_VARIABLE: &str = "COMPARE_RUN_STATE_KEY";
const RUN_STATE_KEY: &str = "00112233445566778899aabbccddeeff";
/// The longest a trial's tool may take.
const BOUND_SECONDS: u64 = 300;
/// The longest the browser may take to listen.
const BROWSER_START_SECONDS: u64 = 30;

/// One step on a page, as the driver reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub(crate) enum Step {
    Click {
        selector: &'static str,
    },
    Fill {
        selector: &'static str,
        value: &'static str,
    },
    Press {
        selector: &'static str,
        key: &'static str,
    },
    /// Reads the page's title, in a script the page runs: its text would
    /// race the scripts that fill some portal pages after their load.
    Read,
}

/// One page of a cell: its Proving ground task and its steps.
pub(crate) struct Planned {
    pub(crate) task: String,
    pub(crate) steps: Vec<Step>,
}

/// What a cell asks of each trial.
pub(crate) enum Cell {
    /// Pages opened in one Session, each taking its steps.
    Pages {
        pages: Vec<Planned>,
        /// Whether they act all at once rather than one after another.
        at_once: bool,
    },
    /// So many Sessions opened one after another, each with one page.
    Sessions(u64),
    /// So many pages opened one after another in one Session, each kept
    /// open until the last is.
    Openings(u64),
}

impl Cell {
    /// Its pages; none in the Sessions and opening cells.
    pub(crate) fn pages(&self) -> &[Planned] {
        match self {
            Self::Pages { pages, .. } => pages,
            Self::Sessions(_) | Self::Openings(_) => &[],
        }
    }

    /// The Sessions it opens; none in the other cells.
    pub(crate) fn sessions(&self) -> u64 {
        match self {
            Self::Pages { .. } | Self::Openings(_) => 0,
            Self::Sessions(sessions) => *sessions,
        }
    }

    /// The pages it opens with nothing to do on them; none in the other
    /// cells.
    pub(crate) fn openings(&self) -> u64 {
        match self {
            Self::Pages { .. } | Self::Sessions(_) => 0,
            Self::Openings(openings) => *openings,
        }
    }

    /// Its actions: each step, each Session opened or each page opened.
    pub(crate) fn actions(&self) -> u64 {
        match self {
            Self::Pages { pages, .. } => pages.iter().map(|page| page.steps.len() as u64).sum(),
            Self::Sessions(sessions) => *sessions,
            Self::Openings(openings) => *openings,
        }
    }

    /// The plan the driver reads: each page at its task's start URL in
    /// `urls`, by task.
    pub(crate) fn plan(&self, urls: &BTreeMap<String, String>) -> Result<Value> {
        let pages = self
            .pages()
            .iter()
            .map(|page| {
                let url = urls
                    .get(&page.task)
                    .with_context(|| format!("the Proving ground lists no task {}", page.task))?;
                Ok(json!({ "task": page.task, "url": url, "steps": page.steps }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({
            "pages": pages,
            "at_once": matches!(self, Self::Pages { at_once: true, .. }),
            "sessions": self.sessions(),
            "openings": self.openings(),
            "action_timeout": ACTION_TIMEOUT_MILLISECONDS,
        }))
    }
}

const TARGET: Step = Step::Click {
    selector: "#target",
};

/// Page 26's counter, clicked so many times.
fn counter() -> Vec<Step> {
    vec![
        Step::Click {
            selector: "#counter"
        };
        50
    ]
}

/// The lab's form pages: 20 to 24 take the whole form, 28 saves a name.
fn form(number: u32) -> Vec<Step> {
    let name = Step::Fill {
        selector: "#name",
        value: "Goodfetch",
    };
    if number == 28 {
        return vec![name, Step::Click { selector: "#save" }];
    }
    vec![
        name,
        Step::Press {
            selector: "#region",
            key: "e",
        },
        Step::Click {
            selector: "#consent",
        },
        Step::Click {
            selector: "#submit",
        },
    ]
}

/// The steps of `actionability.test`'s page `number` in these cells.
fn actionability(number: u32) -> Vec<Step> {
    match number {
        15 => vec![Step::Press {
            selector: "#field",
            key: "Enter",
        }],
        20..=24 | 28 => form(number),
        26 => counter(),
        _ => vec![TARGET],
    }
}

fn pages(family: &str, numbers: &[u32], steps: fn(u32) -> Vec<Step>) -> Vec<Planned> {
    numbers
        .iter()
        .map(|&number| Planned {
            task: format!("{family}/{number}"),
            steps: steps(number),
        })
        .collect()
}

/// What `exercise`'s cell asks. Page 25, whose form is in a frame, is in
/// none: Collie's actions do not reach into frames yet.
pub(crate) fn cell(exercise: Exercise) -> Cell {
    let (pages, at_once) = match exercise {
        Exercise::Click => (
            pages(
                "actionability",
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 18, 19, 26, 27],
                actionability,
            ),
            false,
        ),
        Exercise::Form => (
            pages("actionability", &[20, 21, 22, 23, 24, 28], actionability),
            false,
        ),
        Exercise::Navigation => (
            pages("portal", &[6, 7, 8, 9, 10, 11, 12, 13], |_| {
                vec![Step::Read]
            }),
            false,
        ),
        Exercise::Sessions => return Cell::Sessions(SESSIONS),
        Exercise::Opening => return Cell::Openings(OPENINGS),
        Exercise::AtOnce => (
            pages(
                "actionability",
                &[1, 2, 3, 4, 5, 6, 15, 18, 26, 27, 20, 21, 22, 23, 24, 28],
                actionability,
            ),
            true,
        ),
    };
    Cell::Pages { pages, at_once }
}

impl Exercise {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Form => "form",
            Self::Navigation => "navigation",
            Self::Sessions => "sessions",
            Self::AtOnce => "at_once",
            Self::Opening => "opening",
        }
    }
}

/// The host of `url`, without its port.
fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    rest.split([':', '/']).next()
}

/// The version line whose folder of the Goodfetch home Collie's engine
/// reads its user configuration from: xtask's version is the workspace's,
/// as the engine's is.
pub(crate) fn version_line() -> Result<String> {
    let version = goodfetch_protocol::connection::ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .context("reading the workspace's version")?;
    Ok(version.line().to_string())
}

/// Collie's user configuration for a trial of the pages at `urls`: each
/// page's host resolves to loopback, where the Proving ground listens, and
/// is let through the egress guard's private ranges; an action waits
/// [`ACTION_TIMEOUT_MILLISECONDS`] as Playwright's does; the run-state key
/// is read from [`RUN_STATE_KEY_VARIABLE`].
pub(crate) fn configuration(urls: &[String]) -> Result<String> {
    let mut hosts = Vec::new();
    for url in urls {
        let host = host(url).with_context(|| format!("a page's URL has no host: {url}"))?;
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    // A JSON string is a TOML basic string, its escapes included.
    let quoted = |text: &str| Value::from(text).to_string();
    let mut text = String::from("version = \"0.1\"\n\n[settings.network.resolve]\n");
    for host in &hosts {
        let _ = writeln!(text, "{} = [\"127.0.0.1\"]", quoted(host));
    }
    let ranges: Vec<String> = hosts.iter().map(|host| quoted(host)).collect();
    let _ = write!(
        text,
        "\n[settings.egress]\nprivate_ranges = [{}]\n\n[settings.timeout]\naction = {ACTION_TIMEOUT_MILLISECONDS}\n\n[settings.run_state]\nkey_reference = \"env:{RUN_STATE_KEY_VARIABLE}\"\n",
        ranges.join(", ")
    );
    Ok(text)
}

/// A browser xtask launched, its whole tree stopped when dropped.
struct Browser {
    child: Child,
}

impl Drop for Browser {
    fn drop(&mut self) {
        trial::kill_tree(self.child.id());
        let _ = self.child.wait();
    }
}

/// Launches Chrome for Testing from `folder`, headless, with its profile
/// in `profile`, every `.test` host resolved to loopback and `flags`
/// besides; returns it with its remote-debugging port and the browser
/// endpoint's path.
fn launch(folder: &Path, profile: &Path, flags: &[&str]) -> Result<(Browser, u16, String)> {
    let child = Command::new(folder.join(trial::CHROME))
        .args([
            "--headless",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "--host-resolver-rules=MAP *.test 127.0.0.1",
        ])
        .args(flags)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting Chrome for Testing")?;
    let mut browser = Browser { child };
    let file = profile.join("DevToolsActivePort");
    let started = trial::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(&file)
            && let [port, path, ..] = text.lines().collect::<Vec<_>>()[..]
            && let Ok(port) = port.parse()
        {
            return Ok((browser, port, path.to_owned()));
        }
        if let Some(status) = browser.child.try_wait().context("waiting for Chrome")? {
            bail!("Chrome for Testing exited before it listened: {status}");
        }
        if started.elapsed() >= Duration::from_secs(BROWSER_START_SECONDS) {
            bail!("Chrome for Testing did not listen within {BROWSER_START_SECONDS} s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Each task's start URL, from the Proving ground at `address`.
fn start_urls(address: &str) -> Result<BTreeMap<String, String>> {
    let list: Value = ureq::get(&format!("http://{address}/tasks"))
        .call()
        .context("asking the Proving ground's tasks")?
        .body_mut()
        .read_json()
        .context("parsing the Proving ground's tasks")?;
    let tasks = list["tasks"]
        .as_array()
        .context("the Proving ground's task list holds no tasks")?;
    Ok(tasks
        .iter()
        .filter_map(|task| {
            Some((
                task["id"].as_str()?.to_owned(),
                task["start_url"].as_str()?.to_owned(),
            ))
        })
        .collect())
}

/// The oracle's outcome for `page`, for the status the tool reported.
fn oracle_outcome(address: &str, page: &PageDone) -> Result<OracleOutcome> {
    let submission = json!({
        "task": page.task,
        "status": match page.ended {
            Ended::Success => "success",
            Ended::Failure => "failure",
        },
        "records": [],
    });
    let answer: Value = ureq::post(&format!("http://{address}/oracle"))
        .send_json(&submission)
        .with_context(|| format!("asking the oracle about {}", page.task))?
        .body_mut()
        .read_json()
        .context("parsing the oracle's answer")?;
    serde_json::from_value(answer["outcome"].clone())
        .with_context(|| format!("the oracle's answer has no outcome: {answer}"))
}

/// What the driver printed: each page, the Sessions it opened and the
/// pages it opened with nothing to do on them.
#[derive(serde::Deserialize)]
struct Printed {
    pages: Vec<DriverPage>,
    sessions: u64,
    opened: u64,
}

#[derive(serde::Deserialize)]
struct DriverPage {
    task: String,
    outcome: Ended,
    steps: u64,
    status: Option<u16>,
    read: Option<String>,
    error: Option<String>,
}

/// What the trial keeps of `page`: the digest of the text it read, in place
/// of the text, and on an `actionability.test` page the outcome of the
/// oracle of the Proving ground at `address`.
fn kept(page: DriverPage, address: &str) -> Result<PageDone> {
    let mut kept = PageDone {
        task: page.task,
        ended: page.outcome,
        steps: page.steps,
        status: page.status,
        read: page
            .read
            .map(|read| format!("{:x}", Sha256::digest(read.as_bytes()))),
        error: page.error,
        outcome: None,
    };
    if kept.task.starts_with("actionability/") {
        kept.outcome = Some(oracle_outcome(address, &kept)?);
    }
    Ok(kept)
}

/// Waits for the tool `child`, killing its tree once it has run
/// [`BOUND_SECONDS`], which stops the trial.
fn waited(child: &mut Child) -> Result<Outcome<std::process::ExitStatus>> {
    let started = trial::now();
    loop {
        if let Some(status) = child.try_wait().context("waiting for the tool")? {
            return Ok(Outcome::Done(status));
        }
        if started.elapsed() >= Duration::from_secs(BOUND_SECONDS) {
            trial::kill_tree(child.id());
            child.wait().context("waiting for the stopped tool")?;
            return Ok(Outcome::Stopped(format!(
                "the tool ran past {BOUND_SECONDS} s, the longest a trial may take"
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Points `command` at a Goodfetch home in `folder` whose user
/// configuration is `configuration`, at Collie's engine and at the
/// run-state key.
fn collie_home(
    command: &mut Command,
    folder: &Path,
    tools: &Tools<'_>,
    configuration: &str,
) -> Result<()> {
    let home = folder.join("home");
    let lined = home.join(version_line()?);
    std::fs::create_dir_all(&lined).context("creating the Goodfetch home")?;
    std::fs::write(lined.join("configuration.toml"), configuration)
        .context("writing Collie's configuration")?;
    command
        .env("GOODFETCH_HOME", &home)
        .env("GOODFETCH_COLLIE_ENGINE_PATH", tools.engine)
        .env(RUN_STATE_KEY_VARIABLE, RUN_STATE_KEY);
    Ok(())
}

/// What a driver run left: its one line of output, and the relay's marks
/// with the peak memory of its process tree.
struct Driven {
    printed: String,
    start: Mark,
    end: Mark,
    peak_bytes: u64,
}

/// Runs the driver `command` in `folder`, its process tree followed by a
/// sampler and by `relay`, and returns what it printed between the marks
/// it asked `relay` for; or why the comparison stopped it. `subject` names
/// the trial when the driver fails.
fn drive(
    mut command: Command,
    folder: &Path,
    relay: &Relay,
    subject: &str,
) -> Result<Outcome<Driven>> {
    let printed = folder.join("printed.json");
    let errors = folder.join("errors.txt");
    let watcher = Sampler::start(trial::now(), false);
    let mut child = command
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&printed).context("creating the tool's output")?)
        .stderr(std::fs::File::create(&errors).context("creating the tool's errors")?)
        .spawn()
        .context("running the driver")?;
    watcher.follow(child.id());
    relay.follow(child.id());
    let status = waited(&mut child);
    let samples = watcher.finish().unwrap_or_default();
    let status = match status? {
        Outcome::Done(status) => status,
        Outcome::Stopped(reason) => return Ok(Outcome::Stopped(reason)),
    };
    if !status.success() {
        let stderr = std::fs::read_to_string(&errors).unwrap_or_default();
        bail!("{subject} failed ({status}): {}", stderr.trim());
    }
    let text = std::fs::read_to_string(&printed).context("reading the tool's output")?;
    let (Some(start), Some(end)) = relay.marks() else {
        bail!("the driver did not ask for both marks");
    };
    Ok(Outcome::Done(Driven {
        printed: text.trim().to_owned(),
        start,
        end,
        peak_bytes: sampler::figures(&samples, &[]).peak_bytes,
    }))
}

/// One trial of `exercise` by `tool`: a fresh Proving ground and browser,
/// the tool attached through the counting relay, its figures between its
/// marks, and the oracle's outcome on each `actionability.test` page; or
/// why the comparison stopped it.
pub(crate) fn trial(
    tools: &Tools<'_>,
    exercise: Exercise,
    tool: Contender,
    number: u32,
    load: Option<f64>,
) -> Result<Outcome<CollieTrial>> {
    let serving = crate::rss::crawl::serve(tools.proving_ground, "none")?;
    let urls = start_urls(&serving.address)?;
    let cell = cell(exercise);
    let plan = cell.plan(&urls)?;
    let folder = tempfile::tempdir().context("creating the trial's folder")?;
    let plan_file = folder.path().join("plan.json");
    std::fs::write(&plan_file, serde_json::to_vec(&plan)?).context("writing the plan")?;
    let (browser, port, path) = launch(
        tools.browser,
        &folder.path().join("profile"),
        browser_flags(tool),
    )?;
    let relay = Relay::start(port)?;
    let relayed = relay.address();
    let mut command = tools.command(tool);
    adopt_orphans(&mut command);
    command
        .arg("--plan")
        .arg(&plan_file)
        .arg("--cdp")
        .arg(format!("ws://{relayed}{path}"))
        .arg("--marks")
        .arg(format!("http://{relayed}"));
    if tool == Contender::Collie {
        let page_urls: Vec<String> = cell
            .pages()
            .iter()
            .filter_map(|page| urls.get(&page.task).cloned())
            .collect();
        collie_home(
            &mut command,
            folder.path(),
            tools,
            &configuration(&page_urls)?,
        )?;
    }
    let subject = format!("{} {} trial {number}", tool.name(), exercise.name());
    let driven = match drive(command, folder.path(), &relay, &subject)? {
        Outcome::Done(driven) => driven,
        Outcome::Stopped(reason) => return Ok(Outcome::Stopped(reason)),
    };
    let done: Printed = serde_json::from_str(&driven.printed)
        .with_context(|| format!("parsing the tool's output: {}", driven.printed))?;
    drop(relay);
    drop(browser);
    let pages = done
        .pages
        .into_iter()
        .map(|page| kept(page, &serving.address))
        .collect::<Result<Vec<_>>>()?;
    let actions = match &cell {
        Cell::Pages { .. } => pages.iter().map(|page| page.steps).sum(),
        Cell::Sessions(_) => done.sessions,
        Cell::Openings(_) => done.opened,
    };
    let (start, end) = (driven.start, driven.end);
    Ok(Outcome::Done(CollieTrial {
        exercise,
        tool,
        number,
        load,
        actions,
        sessions: done.sessions,
        commands: end.commands - start.commands,
        rounds: end.rounds - start.rounds,
        cpu_seconds: (end.cpu_seconds - start.cpu_seconds).max(0.0),
        wall_seconds: end.at.duration_since(start.at).as_secs_f64(),
        peak_bytes: driven.peak_bytes,
        pages,
        retried: None,
    }))
}

/// A trial's name in a finding.
fn subject(trial: &CollieTrial) -> String {
    format!(
        "collie {} {} trial {}",
        trial.exercise.name(),
        trial.tool.name(),
        trial.number
    )
}

/// What a page's work is: its status and the digest of what it read.
type Work = (Option<u16>, Option<String>);

/// The steps `done` took past its plan's `planned`: more than planned, or
/// fewer once every one succeeded.
fn other_steps(done: &PageDone, planned: &Planned) -> bool {
    let steps = planned.steps.len() as u64;
    done.steps > steps || done.ended == Ended::Success && done.steps != steps
}

/// How `trial` acted otherwise than its cell `planned` plans: other
/// Sessions, other pages opened, or a page's other steps
/// ([`other_steps`]); none when it acted as planned.
fn other_actions(trial: &CollieTrial, planned: &Cell) -> Option<String> {
    let stepped: Vec<String> = trial
        .pages
        .iter()
        .zip(planned.pages())
        .filter(|(done, plan)| other_steps(done, plan))
        .map(|(done, plan)| {
            format!(
                "{} took {} steps of {}",
                done.task,
                done.steps,
                plan.steps.len()
            )
        })
        .collect();
    let opened = matches!(planned, Cell::Openings(_)) && trial.actions != planned.actions();
    if stepped.is_empty() && trial.sessions == planned.sessions() && !opened {
        return None;
    }
    let mut parts = vec![format!(
        "{} Sessions, where its cell plans {}",
        trial.sessions,
        planned.sessions()
    )];
    if opened {
        parts.push(format!(
            "{} pages opened, where it plans {}",
            trial.actions,
            planned.actions()
        ));
    }
    parts.extend(stepped);
    Some(parts.join("; "))
}

/// Each finding that keeps the Collie scenario's figures from counting: a
/// cell short of or past its declared trials of a tool the run ran, unless
/// a trial of it was stopped twice or the tool runs none there
/// ([`skipped`]); a trial whose pages or Sessions are not its
/// cell's, whose page took other steps than its plan's (more, or fewer
/// though they all succeeded), or that counted no command; and a trial
/// whose page saw another status or text than most trials of both tools
/// saw, every trial named on a tie; and the page-load cell's
/// ([`page_load::check`]).
pub(crate) fn check(scenario: &CollieScenario) -> Vec<Finding> {
    let CollieScenario::Measured {
        trials,
        tools,
        trials_run,
        stopped,
        loads,
        loads_stopped,
    } = scenario
    else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for exercise in EXERCISES {
        let planned = self::cell(exercise);
        let tasks: Vec<&str> = planned
            .pages()
            .iter()
            .map(|page| page.task.as_str())
            .collect();
        let mut ran: Vec<&CollieTrial> = trials_run
            .iter()
            .filter(|trial| trial.exercise == exercise)
            .collect();
        ran.sort_by_key(|trial| (trial.tool, trial.number));
        for &tool in tools {
            let count = ran.iter().filter(|trial| trial.tool == tool).count();
            let was_stopped = stopped
                .iter()
                .any(|run| run.exercise == exercise && run.tool == tool);
            let declared = if skipped(tool, exercise).is_some() {
                0
            } else {
                *trials
            };
            if !was_stopped && u32::try_from(count).ok() != Some(declared) {
                findings.push(Finding::new(
                    "trial_count",
                    format!("collie {} {}", exercise.name(), tool.name()),
                    format!("{count} trials, where {trials} were declared"),
                ));
            }
        }
        let mut whole = Vec::new();
        for trial in &ran {
            let done: Vec<&str> = trial.pages.iter().map(|page| page.task.as_str()).collect();
            if done != tasks {
                findings.push(Finding::new(
                    "different_pages",
                    subject(trial),
                    format!("its pages were {done:?}, where its cell plans {tasks:?}"),
                ));
                continue;
            }
            if let Some(detail) = other_actions(trial, &planned) {
                findings.push(Finding::new("different_actions", subject(trial), detail));
                continue;
            }
            if trial.commands == 0 {
                findings.push(Finding::new(
                    "nothing_counted",
                    subject(trial),
                    "the counting relay counted no CDP command, so the trial measured nothing",
                ));
                continue;
            }
            whole.push(*trial);
        }
        findings.extend(other_work(&whole));
    }
    findings.extend(page_load::check(*trials, loads, loads_stopped));
    findings
}

/// Each trial of `trials`, all of one cell's pages, whose page saw another
/// status or text than most of them.
fn other_work(trials: &[&CollieTrial]) -> Vec<Finding> {
    let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Some(first) = trials.first() else {
        return Vec::new();
    };
    for (index, page) in first.pages.iter().enumerate() {
        let work = |trial: &CollieTrial| -> Work {
            let page = &trial.pages[index];
            (page.status, page.read.clone())
        };
        let mut votes: BTreeMap<Work, usize> = BTreeMap::new();
        for trial in trials {
            *votes.entry(work(trial)).or_default() += 1;
        }
        let most = votes.values().copied().max().unwrap_or(0);
        let leaders: Vec<&Work> = votes
            .iter()
            .filter(|(_, count)| **count == most)
            .map(|(work, _)| work)
            .collect();
        for trial in trials {
            let own = work(trial);
            let differs = leaders.len() > 1 || !leaders.contains(&&own);
            if differs {
                named.entry(subject(trial)).or_default().push(format!(
                    "{} saw status {} and read {}",
                    page.task,
                    own.0
                        .map_or_else(|| "none".to_owned(), |status| status.to_string()),
                    own.1.as_deref().unwrap_or("nothing")
                ));
            }
        }
    }
    trials
        .iter()
        .filter_map(|trial| {
            let subject = subject(trial);
            let pages = named.remove(&subject)?;
            Some(Finding::new(
                "different_work",
                subject,
                format!(
                    "{}, where most trials of both tools saw otherwise",
                    pages.join("; ")
                ),
            ))
        })
        .collect()
}

/// Every trial of the Collie scenario, `trials` of each of `tools` in each
/// cell it runs ([`skipped`]), in one random order drawn from `state` by
/// `next`.
pub(crate) fn order(
    tools: &[Contender],
    trials: u32,
    state: &mut u64,
    next: fn(&mut u64) -> u64,
) -> Vec<(Exercise, Contender, u32)> {
    let order: Vec<_> = EXERCISES
        .into_iter()
        .flat_map(|exercise| {
            tools
                .iter()
                .copied()
                .filter(move |&tool| skipped(tool, exercise).is_none())
                .flat_map(move |tool| (1..=trials).map(move |number| (exercise, tool, number)))
        })
        .collect();
    shuffled(order, state, next)
}

/// `order` in one random order drawn from `state` by `next`
/// (Fisher–Yates).
fn shuffled<T>(mut order: Vec<T>, state: &mut u64, next: fn(&mut u64) -> u64) -> Vec<T> {
    for last in (1..order.len()).rev() {
        let pick = usize::try_from(next(state) % (last as u64 + 1))
            .expect("invariant: an index below the trial count fits a usize");
        order.swap(last, pick);
    }
    order
}

#[cfg(test)]
mod tests;
