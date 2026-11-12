//! The Collie scenario of `xtask compare` (issue 2282): Collie against
//! Playwright on the Proving ground's `actionability.test` and
//! `portal.test` pages, in five cells (click, form, navigation, Sessions
//! opened, pages at once), each tool carrying out the same plan through
//! `xtask/compare-collie/act.ts`.
//!
//! Both tools attach over CDP to one Chrome for Testing that xtask
//! launched, which neither tool launched: through the counting relay
//! ([`super::relay`]), each tool's CDP commands are counted the same way,
//! from outside it, and the browser's own processes are left out of each
//! tool's CPU and memory. What that leaves out of the Sessions cell is the
//! browser's launch, which `just counts` times for Collie at its defaults
//! (issue 2319).
//!
//! Each trial runs one tool in a fresh process, against a fresh Proving
//! ground and a fresh browser, and takes its figures between the marks the
//! driver asks for once attached: CDP commands, the CPU time of the tool's
//! process tree and the wall time, each per action, beside the peak
//! resident memory of the tool's process tree. The oracle judges each
//! `actionability.test` page, which gives the pages each tool got right.
//! The same-work check holds every trial to its cell's plan and every page
//! to the status and the text most trials saw; the report gives
//! `collie.playwright_command_ratio` from the click and form cells, and
//! says when it is over the register row's Today.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial runs the browser and the tool as blocking child processes and waits by sleeping (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::check::Finding;

use super::relay::Relay;
use super::sampler::{self, Sampler};
use super::trial;
use super::trials::{CollieCell, CollieTrial, Driver, Ended, Exercise, PageDone};

mod section;

pub(crate) use section::{Section, markdown, section};

/// The cells, in the order the report gives them.
pub(crate) const EXERCISES: [Exercise; 5] = [
    Exercise::Click,
    Exercise::Form,
    Exercise::Navigation,
    Exercise::Sessions,
    Exercise::AtOnce,
];
/// How long an action waits for its target, in both tools.
pub(crate) const ACTION_TIMEOUT_MILLISECONDS: u64 = 1000;
/// The Sessions the Sessions cell opens, after one outside the marks.
pub(crate) const SESSIONS: u64 = 10;
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
    /// Reads the page's text.
    Read,
}

/// One page of a cell: its Proving ground task and its steps.
pub(crate) struct Planned {
    pub(crate) task: String,
    pub(crate) steps: Vec<Step>,
}

/// What a cell asks of each trial.
pub(crate) struct Cell {
    pub(crate) pages: Vec<Planned>,
    /// Whether the pages act all at once in one Session.
    pub(crate) at_once: bool,
    /// The Sessions opened one after another, for the Sessions cell.
    pub(crate) sessions: u64,
}

impl Cell {
    /// Its actions: each step, or each Session opened.
    pub(crate) fn actions(&self) -> u64 {
        if self.sessions > 0 {
            return self.sessions;
        }
        self.pages.iter().map(|page| page.steps.len() as u64).sum()
    }

    /// The plan the driver reads: each page at its task's start URL in
    /// `urls`, by task.
    pub(crate) fn plan(&self, urls: &BTreeMap<String, String>) -> Result<Value> {
        let pages = self
            .pages
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
            "at_once": self.at_once,
            "sessions": self.sessions,
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
    let (numbers, at_once, sessions): (&[u32], _, _) = match exercise {
        Exercise::Click => (&[1, 2, 3, 4, 5, 6, 7, 8, 9, 18, 19, 26, 27], false, 0),
        Exercise::Form => (&[20, 21, 22, 23, 24, 28], false, 0),
        Exercise::Navigation => {
            return Cell {
                pages: pages("portal", &[6, 7, 8, 9, 10, 11, 12, 13], |_| {
                    vec![Step::Read]
                }),
                at_once: false,
                sessions: 0,
            };
        }
        Exercise::Sessions => (&[], false, SESSIONS),
        Exercise::AtOnce => (
            &[1, 2, 3, 4, 5, 6, 15, 18, 26, 27, 20, 21, 22, 23, 24, 28],
            true,
            0,
        ),
    };
    Cell {
        pages: pages("actionability", numbers, actionability),
        at_once,
        sessions,
    }
}

impl Exercise {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Form => "form",
            Self::Navigation => "navigation",
            Self::Sessions => "sessions",
            Self::AtOnce => "at_once",
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

/// The executables and files a trial takes.
pub(crate) struct Tools<'a> {
    pub(crate) proving_ground: &'a Path,
    /// Collie's Attached engine.
    pub(crate) engine: &'a Path,
    /// `xtask/compare-collie/act.ts`, which Node.js runs.
    pub(crate) driver: &'a Path,
    /// Chrome for Testing's folder.
    pub(crate) browser: &'a Path,
    /// xtask's own `GOODFETCH_*` variables, which the tool runs without.
    pub(crate) goodfetch_variables: &'a [OsString],
}

/// A browser xtask launched, its whole tree stopped when dropped.
struct Browser {
    child: Child,
}

impl Drop for Browser {
    fn drop(&mut self) {
        for pid in sampler::tree(self.child.id()) {
            // One may have exited meanwhile.
            let _ = trial::kill(pid);
        }
        let _ = self.child.wait();
    }
}

/// Launches Chrome for Testing from `folder`, headless, with its profile
/// in `profile` and every `.test` host resolved to loopback; returns it with
/// its remote-debugging port and the browser endpoint's path.
fn launch(folder: &Path, profile: &Path) -> Result<(Browser, u16, String)> {
    let child = Command::new(folder.join(trial::CHROME))
        .args([
            "--headless",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "--host-resolver-rules=MAP *.test 127.0.0.1",
        ])
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

/// The oracle's outcome for `page`, as the tool reported it.
fn verdict(address: &str, page: &PageDone) -> Result<String> {
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
    answer["outcome"]
        .as_str()
        .map(str::to_owned)
        .context("the oracle's answer has no outcome")
}

/// What the driver printed: each page, and the Sessions it opened.
#[derive(serde::Deserialize)]
struct Printed {
    pages: Vec<DriverPage>,
    sessions: u64,
}

#[derive(serde::Deserialize)]
struct DriverPage {
    task: String,
    outcome: Ended,
    status: Option<u16>,
    read: Option<String>,
    error: Option<String>,
}

/// Waits for the tool `child`, killing its tree once it has run
/// [`BOUND_SECONDS`].
fn waited(child: &mut Child) -> Result<std::process::ExitStatus> {
    let started = trial::now();
    loop {
        if let Some(status) = child.try_wait().context("waiting for the tool")? {
            return Ok(status);
        }
        if started.elapsed() >= Duration::from_secs(BOUND_SECONDS) {
            for pid in sampler::tree(child.id()) {
                let _ = trial::kill(pid);
            }
            let _ = child.wait();
            bail!("the tool ran past {BOUND_SECONDS} s and was stopped");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One trial of `exercise` by `tool`: a fresh Proving ground and browser,
/// the tool attached through the counting relay, its figures between its
/// marks, and the oracle's verdict on each `actionability.test` page.
pub(crate) fn trial(
    tools: &Tools<'_>,
    exercise: Exercise,
    tool: Driver,
    number: u32,
    load: Option<f64>,
) -> Result<CollieTrial> {
    let serving = crate::rss::crawl::serve(tools.proving_ground)?;
    let urls = start_urls(&serving.address)?;
    let cell = cell(exercise);
    let plan = cell.plan(&urls)?;
    let folder = tempfile::tempdir().context("creating the trial's folder")?;
    let plan_file = folder.path().join("plan.json");
    std::fs::write(&plan_file, serde_json::to_vec(&plan)?).context("writing the plan")?;
    let (browser, port, path) = launch(tools.browser, &folder.path().join("profile"))?;
    let relay = Relay::start(port)?;
    let relayed = relay.address();
    let mut command = Command::new("node");
    command
        .arg(tools.driver)
        .args(["--tool", tool.name(), "--plan"])
        .arg(&plan_file)
        .arg("--cdp")
        .arg(format!("ws://{relayed}{path}"))
        .arg("--marks")
        .arg(format!("http://{relayed}"))
        .current_dir(folder.path());
    for name in tools.goodfetch_variables {
        command.env_remove(name);
    }
    if tool == Driver::Collie {
        let home = folder.path().join("home");
        let lined = home.join(version_line()?);
        std::fs::create_dir_all(&lined).context("creating the Goodfetch home")?;
        let page_urls: Vec<String> = cell
            .pages
            .iter()
            .filter_map(|page| urls.get(&page.task).cloned())
            .collect();
        std::fs::write(lined.join("configuration.toml"), configuration(&page_urls)?)
            .context("writing Collie's configuration")?;
        command
            .env("GOODFETCH_HOME", &home)
            .env("GOODFETCH_COLLIE_ENGINE_PATH", tools.engine)
            .env(RUN_STATE_KEY_VARIABLE, RUN_STATE_KEY);
    }
    let printed = folder.path().join("printed.json");
    let errors = folder.path().join("errors.txt");
    let watcher = Sampler::start(trial::now());
    let mut child = command
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&printed).context("creating the tool's output")?)
        .stderr(std::fs::File::create(&errors).context("creating the tool's errors")?)
        .spawn()
        .context("running the driver with Node.js")?;
    watcher.follow(child.id());
    relay.follow(child.id());
    let status = waited(&mut child);
    let samples = watcher.finish().unwrap_or_default();
    let status = status?;
    let stderr = std::fs::read_to_string(&errors).unwrap_or_default();
    if !status.success() {
        bail!(
            "{} {} trial {number} failed ({status}): {}",
            tool.name(),
            exercise.name(),
            stderr.trim()
        );
    }
    let text = std::fs::read_to_string(&printed).context("reading the tool's output")?;
    let done: Printed = serde_json::from_str(text.trim())
        .with_context(|| format!("parsing the tool's output: {}", text.trim()))?;
    let (Some(start), Some(end)) = relay.marks() else {
        bail!("the driver did not ask for both marks");
    };
    drop(relay);
    drop(browser);
    let mut pages = Vec::new();
    for page in done.pages {
        let mut kept = PageDone {
            task: page.task,
            ended: page.outcome,
            status: page.status,
            read: page
                .read
                .map(|read| format!("{:x}", Sha256::digest(read.as_bytes()))),
            error: page.error,
            verdict: None,
        };
        if kept.task.starts_with("actionability/") {
            kept.verdict = Some(verdict(&serving.address, &kept)?);
        }
        pages.push(kept);
    }
    Ok(CollieTrial {
        exercise,
        tool,
        number,
        load,
        actions: cell.actions(),
        sessions: done.sessions,
        commands: end.commands - start.commands,
        cpu_seconds: (end.cpu_seconds - start.cpu_seconds).max(0.0),
        wall_seconds: end.at.duration_since(start.at).as_secs_f64(),
        peak_bytes: sampler::figures(&samples, &[]).peak_bytes,
        pages,
    })
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

/// Each finding that keeps the Collie scenario's figures from counting: a
/// cell short of or past its declared trials of a tool; a trial whose pages
/// or actions are not its cell's, or that counted no command; and a trial
/// whose page saw another status or text than most trials of both tools
/// saw, every trial named on a tie.
pub(crate) fn check(cell: &CollieCell) -> Vec<Finding> {
    let CollieCell::Measured { trials, trials_run } = cell else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for exercise in EXERCISES {
        let planned = self::cell(exercise);
        let tasks: Vec<&str> = planned
            .pages
            .iter()
            .map(|page| page.task.as_str())
            .collect();
        let mut ran: Vec<&CollieTrial> = trials_run
            .iter()
            .filter(|trial| trial.exercise == exercise)
            .collect();
        ran.sort_by_key(|trial| (trial.tool, trial.number));
        for tool in Driver::ALL {
            let count = ran.iter().filter(|trial| trial.tool == tool).count();
            if u32::try_from(count).ok() != Some(*trials) {
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
            if trial.actions != planned.actions() || trial.sessions != planned.sessions {
                findings.push(Finding::new(
                    "different_actions",
                    subject(trial),
                    format!(
                        "{} actions and {} Sessions, where its cell plans {} and {}",
                        trial.actions,
                        trial.sessions,
                        planned.actions(),
                        planned.sessions
                    ),
                ));
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

/// Every trial of the Collie scenario, `trials` of each tool in each cell,
/// in one random order drawn from `state` by `next`.
pub(crate) fn order(
    trials: u32,
    state: &mut u64,
    next: fn(&mut u64) -> u64,
) -> Vec<(Exercise, Driver, u32)> {
    let mut order: Vec<_> = EXERCISES
        .into_iter()
        .flat_map(|exercise| {
            Driver::ALL
                .into_iter()
                .flat_map(move |tool| (1..=trials).map(move |number| (exercise, tool, number)))
        })
        .collect();
    for last in (1..order.len()).rev() {
        let pick = usize::try_from(next(state) % (last as u64 + 1))
            .expect("invariant: an index below the trial count fits a usize");
        order.swap(last, pick);
    }
    order
}

#[cfg(test)]
mod tests;
