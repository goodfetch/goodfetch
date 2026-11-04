//! The details window's model (ADR 0141), the window process's own State:
//! the Client's reports of the window's own Connection and the person's
//! inputs go in; the view the window draws, when to repaint it and the
//! reads the view needs come out. Like the Tray's State, it knows no
//! toolkit, socket or clock.
//!
//! - **The Runs table**: the Run index's rows of the person's filters, as
//!   `run.list` answers them, listed again after each resync and each Run
//!   that starts or ends.
//! - **The chosen Run**: its progress, its Usage, its failures by Error
//!   code and its Run report.
//! - **The reads the view needs** ([`Wanted`]): `run.progress` only of the
//!   running Runs on screen and of the chosen Run while it runs; the
//!   chosen Run's report, which a live Run answers at its end; its Usage,
//!   read again at each new progress while it runs. Nothing while the
//!   window is closed or no Daemon is reached.
//! - **Repaints**: only when the view changed, at most once each
//!   [`REDRAW_PERIOD`], and none once the window is closed.
//!
//! Every string shown passes through the Text function.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_protocol::foxhound::report::{CountOf, RequestCounts};
use goodfetch_protocol::run::{
    Lifecycle, ProgressMessage, RECIPE_TAG, RunId, RunIndexRow, RunListParams, RunProgress,
    RunReport, RunUsage, StopReason,
};
use serde_json::{Map, Value};

use super::{Daemon, REDRAW_PERIOD, health_text, product_name, refused};
use crate::client::{ConnectionChange, DaemonEvent, Listing, UsageRead, Wanted};
use crate::text;

#[cfg(test)]
mod tests;

/// The most characters of a value the window shows.
const VALUE_MOST: usize = 300;

/// The most characters of a Run's name in the table.
const NAME_MOST: usize = 60;

/// What reaches the window's model.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The window's Connection to the Daemon changed.
    Connection(ConnectionChange),
    /// Events of the window's subscription, in order.
    Events(Vec<DaemonEvent>),
    /// The rows of the listing of this generation, as `run.list` answered.
    Listed {
        /// The listing's generation.
        generation: u64,
        /// The rows.
        rows: Vec<RunIndexRow>,
    },
    /// A Run's progress, as `run.progress` answered.
    Progress(RunProgress),
    /// A Run's report, or the Error code its read failed with.
    Report {
        /// The Run.
        run_id: RunId,
        /// Its report, or why there is none.
        report: Result<Box<RunReport>, String>,
    },
    /// A Run's Usage, as `run.usage` answered.
    Usage {
        /// The Run.
        run_id: RunId,
        /// What it used.
        usage: RunUsage,
    },
    /// The person changed the table's filters.
    Filter(Filter),
    /// The person chose a Run in the table, or none.
    Chose(Option<RunId>),
    /// The table's rows on screen now, by Run ID.
    OnScreen(BTreeSet<RunId>),
    /// The person closed the window.
    Closed,
}

/// The Runs table's filters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Text found, ignoring case, in a Run's ID or tags, its Recipe's
    /// among them; blank lists every Run.
    pub text: String,
    /// The Lifecycle of the Runs listed; none lists every one.
    pub lifecycle: Option<Lifecycle>,
}

impl Filter {
    /// The `run.list` parameters of the filters, newest first.
    fn params(&self) -> RunListParams {
        let text = self.text.trim();
        RunListParams {
            text: (!text.is_empty()).then(|| text.to_owned()),
            lifecycle: self.lifecycle,
            ..RunListParams::default()
        }
    }
}

/// What the window draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailsView {
    /// The Daemon the window reached, or why none.
    pub daemon: String,
    /// The reached Daemon's health, once read.
    pub health: Option<String>,
    /// What the table says instead of rows, when it has none.
    pub note: Option<String>,
    /// The Runs table, in the listing's order.
    pub runs: Vec<RunLine>,
    /// The chosen Run.
    pub chosen: Option<RunPanel>,
}

/// A row of the Runs table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunLine {
    /// The Run's ID, by which the person chooses it.
    pub run_id: RunId,
    /// Its Recipe, or its Run ID when it ran none.
    pub name: String,
    /// Its Lifecycle.
    pub lifecycle: String,
    /// Its Verdict, once it ended.
    pub verdict: String,
    /// Its Completeness, once it ended.
    pub completeness: String,
    /// When it started.
    pub started: String,
    /// When it ended.
    pub ended: String,
    /// Its progress message and, on a Foxhound Daemon, a crawl's counts,
    /// while it runs on screen.
    pub progress: String,
}

/// The chosen Run's details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunPanel {
    /// The Run's ID.
    pub run_id: RunId,
    /// Its name and Run ID.
    pub title: String,
    /// Its progress.
    pub progress: Vec<Field>,
    /// Its Usage, with the Limits that apply to it.
    pub usage: Vec<Field>,
    /// A crawl's failed requests, by Error code.
    pub failures: Vec<Field>,
    /// Its Run report.
    pub report: Vec<Field>,
}

/// A labelled value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// What the value is.
    pub label: String,
    /// The value.
    pub value: String,
}

impl Field {
    /// The field `label` of `value`, both made safe to show: a label may
    /// be the Daemon's, such as an Error code.
    fn new(label: &str, value: &str) -> Self {
        Self {
            label: text::shown(label, NAME_MOST),
            value: text::shown(value, VALUE_MOST),
        }
    }
}

/// Whether the window repaints now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repaint {
    /// Repaint this view now.
    Now(Box<DetailsView>),
    /// A repaint is due at this instant, not before.
    At(Instant),
    /// Nothing to repaint.
    Nothing,
}

/// The details window's model.
#[derive(Debug)]
pub struct Details {
    open: bool,
    daemon: Daemon,
    /// Whether this Connection's subscription has resynced, after which
    /// the Runs are listed.
    resynced: bool,
    filter: Filter,
    /// Bumped each time the table's rows are to be listed again.
    generation: u64,
    /// The table's rows, once the listing of `generation` answered.
    rows: Option<Vec<RunIndexRow>>,
    on_screen: BTreeSet<RunId>,
    chosen: Option<RunId>,
    /// The last progress read of each Run followed.
    progress: BTreeMap<RunId, RunProgress>,
    /// Bumped each time the chosen Run's Usage is to be read again.
    usage_generation: u64,
    /// The chosen Run's Usage, as `run.usage` last answered.
    usage: Option<(RunId, RunUsage)>,
    /// The chosen Run's report, or why there is none.
    report: Option<(RunId, Result<Box<RunReport>, String>)>,
    /// When the window last repainted, and what.
    drawn: Option<(Instant, DetailsView)>,
}

impl Default for Details {
    fn default() -> Self {
        Self::new()
    }
}

impl Details {
    /// The model of a window that just opened, looking for the Daemon.
    pub fn new() -> Self {
        Self {
            open: true,
            daemon: Daemon::Looking,
            resynced: false,
            filter: Filter::default(),
            generation: 0,
            rows: None,
            on_screen: BTreeSet::new(),
            chosen: None,
            progress: BTreeMap::new(),
            usage_generation: 0,
            usage: None,
            report: None,
            drawn: None,
        }
    }

    /// Takes `input` in.
    pub fn apply(&mut self, input: Input) {
        match input {
            Input::Connection(change) => self.connection(change),
            Input::Events(events) => {
                // A resync, and a Run that starts or ends, change the rows.
                if events
                    .iter()
                    .any(|event| matches!(event, DaemonEvent::Resync(_) | DaemonEvent::RunAxes(_)))
                {
                    self.generation += 1;
                }
                if events
                    .iter()
                    .any(|event| matches!(event, DaemonEvent::Resync(_)))
                {
                    self.resynced = true;
                }
            }
            Input::Listed { generation, rows } => {
                if generation == self.generation {
                    let listed: BTreeSet<&RunId> = rows.iter().map(|row| &row.run_id).collect();
                    self.progress
                        .retain(|id, _| listed.contains(id) || self.chosen.as_ref() == Some(id));
                    self.rows = Some(rows);
                }
            }
            Input::Progress(progress) => {
                if self.chosen.as_ref() == Some(&progress.run_id) {
                    self.usage_generation += 1;
                }
                self.progress.insert(progress.run_id.clone(), progress);
            }
            Input::Report { run_id, report } => {
                if self.chosen.as_ref() == Some(&run_id) {
                    self.report = Some((run_id, report));
                }
            }
            Input::Usage { run_id, usage } => {
                if self.chosen.as_ref() == Some(&run_id) {
                    self.usage = Some((run_id, usage));
                }
            }
            Input::Filter(filter) => {
                if filter != self.filter {
                    self.filter = filter;
                    self.generation += 1;
                }
            }
            Input::Chose(chosen) => {
                if chosen != self.chosen {
                    self.chosen = chosen;
                    self.report = None;
                    self.usage = None;
                    self.usage_generation += 1;
                }
            }
            Input::OnScreen(runs) => self.on_screen = runs,
            Input::Closed => self.open = false,
        }
    }

    /// The reads the view needs now: none while the window is closed or
    /// no Daemon is reached.
    pub fn wanted(&self) -> Wanted {
        if !self.open || !matches!(self.daemon, Daemon::Reached(..)) {
            return Wanted::default();
        }
        let mut progress: BTreeSet<RunId> = self
            .on_screen
            .iter()
            .filter(|id| self.runs(id))
            .cloned()
            .collect();
        let chosen_runs = self.chosen.as_ref().filter(|id| self.runs(id));
        progress.extend(chosen_runs.cloned());
        Wanted {
            live_runs: 0,
            progress,
            listing: self.resynced.then(|| Listing {
                generation: self.generation,
                params: self.filter.params(),
            }),
            report: self.chosen.clone(),
            usage: chosen_runs.map(|id| UsageRead {
                run_id: id.clone(),
                generation: self.usage_generation,
            }),
        }
    }

    /// What the window draws now.
    pub fn view(&self) -> DetailsView {
        let (daemon, health) = match &self.daemon {
            Daemon::Looking => ("Looking for the Daemon…".to_owned(), None),
            Daemon::None => ("No Daemon".to_owned(), None),
            Daemon::Reached(identity, health) => (
                format!(
                    "{} Daemon {}",
                    product_name(identity.product),
                    identity.product_version
                ),
                health.as_ref().map(health_text),
            ),
            Daemon::Refused(refusal) => (refused(refusal), None),
        };
        let product = match &self.daemon {
            Daemon::Reached(identity, _) => Some(identity.product),
            Daemon::Looking | Daemon::None | Daemon::Refused(_) => None,
        };
        let crawls = product == Some(Product::Foxhound);
        let rows = self.rows.as_deref().unwrap_or_default();
        let note = match (&self.daemon, &self.rows) {
            (Daemon::Reached(..), None) => Some("Reading the Runs…"),
            (Daemon::Reached(..), Some(rows)) if rows.is_empty() => {
                Some("No Run matches the filters")
            }
            _ => None,
        };
        DetailsView {
            daemon: text::shown(&daemon, VALUE_MOST),
            health: health.map(|health| text::shown(&health, VALUE_MOST)),
            note: note.map(str::to_owned),
            runs: rows.iter().map(|row| self.line(row, crawls)).collect(),
            chosen: self.chosen.as_ref().map(|id| self.panel(id, crawls)),
        }
    }

    /// Whether the window repaints at `now`: when the view changed since
    /// its last repaint, at most once each [`REDRAW_PERIOD`], and never
    /// once it is closed.
    pub fn repaint(&mut self, now: Instant) -> Repaint {
        if !self.open {
            return Repaint::Nothing;
        }
        let view = self.view();
        if let Some((at, drawn)) = &self.drawn {
            if *drawn == view {
                return Repaint::Nothing;
            }
            let due = *at + REDRAW_PERIOD;
            if now < due {
                return Repaint::At(due);
            }
        }
        self.drawn = Some((now, view.clone()));
        Repaint::Now(Box::new(view))
    }

    fn connection(&mut self, change: ConnectionChange) {
        self.daemon = match change {
            ConnectionChange::NoDaemon => Daemon::None,
            ConnectionChange::Connected(identity) => Daemon::Reached(identity, None),
            ConnectionChange::Refused(refusal) => Daemon::Refused(refusal),
            ConnectionChange::Health(reading) => {
                if let Daemon::Reached(_, health) = &mut self.daemon {
                    *health = Some(reading);
                }
                return;
            }
        };
        // Each Connection reads again what the view shows, starting from
        // the resync that starts its subscription.
        self.resynced = false;
        self.rows = None;
        self.progress.clear();
        self.usage = None;
        self.report = None;
    }

    /// Whether the Run `id` runs, by its last progress or else its row.
    fn runs(&self, id: &RunId) -> bool {
        match self.progress.get(id) {
            Some(progress) => progress.lifecycle == Lifecycle::Running && !progress.last,
            None => self
                .row(id)
                .is_some_and(|row| row.lifecycle == Lifecycle::Running),
        }
    }

    fn row(&self, id: &RunId) -> Option<&RunIndexRow> {
        self.rows.as_deref()?.iter().find(|row| row.run_id == *id)
    }

    /// The table's row of `row`, with the progress followed while it runs
    /// on screen.
    fn line(&self, row: &RunIndexRow, crawls: bool) -> RunLine {
        let progress = self.progress.get(&row.run_id).filter(|_| {
            self.on_screen.contains(&row.run_id) || self.chosen.as_ref() == Some(&row.run_id)
        });
        let message = progress
            .and_then(|progress| progress.message.as_ref().map(ProgressMessage::as_str))
            .or(row.progress_message.as_deref())
            .filter(|_| row.lifecycle == Lifecycle::Running);
        let counts = progress
            .filter(|_| crawls)
            .and_then(|progress| part::<RequestProgress>(&progress.kind_parts))
            .map(|requests| {
                format!(
                    "{} succeeded, {} failed, {} pending",
                    requests.succeeded, requests.failed, requests.pending
                )
            });
        let shown = match (message, counts) {
            (Some(message), Some(counts)) => format!("{message} · {counts}"),
            (Some(message), None) => message.to_owned(),
            (None, Some(counts)) => counts,
            (None, None) => String::new(),
        };
        RunLine {
            run_id: row.run_id.clone(),
            name: text::shown(&name(row), NAME_MOST),
            lifecycle: lifecycle(row.lifecycle).to_owned(),
            verdict: row
                .verdict
                .map_or_else(String::new, |verdict| verdict.as_str().to_owned()),
            completeness: row
                .completeness
                .map_or_else(String::new, |completeness| completeness.as_str().to_owned()),
            started: text::shown(row.started_at.as_str(), VALUE_MOST),
            ended: row
                .ended_at
                .as_ref()
                .map_or_else(String::new, |ended| text::shown(ended.as_str(), VALUE_MOST)),
            progress: text::shown(&shown, VALUE_MOST),
        }
    }

    /// The chosen Run's details.
    fn panel(&self, id: &RunId, crawls: bool) -> RunPanel {
        let row = self.row(id);
        let progress = self.progress.get(id);
        let report = self
            .report
            .as_ref()
            .filter(|(run, _)| run == id)
            .map(|(_, report)| report);
        let ended_report = report.and_then(|report| report.as_ref().ok());
        let title = match row.and_then(|row| row.tags.reserved(RECIPE_TAG)) {
            Some(recipe) => format!("{recipe} ({id})"),
            None => id.as_str().to_owned(),
        };
        let usage = ended_report.map(|report| &report.usage).or_else(|| {
            self.usage
                .as_ref()
                .filter(|(run, _)| run == id)
                .map(|(_, usage)| usage)
        });
        let failures = if crawls {
            let by_code = match (ended_report, progress) {
                (Some(report), _) => {
                    part::<RequestCounts>(&report.kind_parts).map(|requests| requests.by_error_code)
                }
                (None, Some(progress)) => part::<RequestProgress>(&progress.kind_parts)
                    .map(|requests| requests.by_error_code),
                (None, None) => None,
            };
            by_code
                .unwrap_or_default()
                .iter()
                .map(|CountOf { name, count }| Field::new(name, &count.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        RunPanel {
            run_id: id.clone(),
            title: text::shown(&title, VALUE_MOST),
            progress: progress_fields(row, progress, crawls),
            usage: usage.map(usage_fields).unwrap_or_default(),
            failures,
            report: match report {
                Some(Ok(report)) => report_fields(report),
                Some(Err(code)) => vec![Field::new("Run report", &unreported(code))],
                None if self.runs(id) => {
                    vec![Field::new("Run report", "Comes when the Run ends")]
                }
                None => vec![Field::new("Run report", "Reading…")],
            },
        }
    }
}

/// The chosen Run's progress, from its last progress read, or else its
/// row.
fn progress_fields(
    row: Option<&RunIndexRow>,
    progress: Option<&RunProgress>,
    crawls: bool,
) -> Vec<Field> {
    let mut fields = Vec::new();
    let Some(progress) = progress else {
        if let Some(row) = row {
            fields.push(Field::new("Lifecycle", lifecycle(row.lifecycle)));
            if let Some(message) = &row.progress_message {
                fields.push(Field::new("Message", message));
            }
        }
        return fields;
    };
    fields.push(Field::new("Lifecycle", lifecycle(progress.lifecycle)));
    if let Some(message) = &progress.message {
        fields.push(Field::new("Message", message.as_str()));
    }
    if let Some(reason) = &progress.stop_reason {
        fields.push(Field::new("Stop reason", &stop_reason(reason)));
    }
    if !progress.pauses.is_empty() {
        fields.push(Field::new(
            "Human pauses",
            &progress.pauses.len().to_string(),
        ));
    }
    if crawls && let Some(requests) = part::<RequestProgress>(&progress.kind_parts) {
        fields.push(Field::new(
            "Requests",
            &format!(
                "{} succeeded, {} failed, {} skipped, {} blocked, {} pending, {} in flight",
                requests.succeeded,
                requests.failed,
                requests.skipped,
                requests.blocked,
                requests.pending,
                requests.fetching + requests.handling
            ),
        ));
    }
    fields
}

/// A Run's Usage, with the Limits that apply to it.
fn usage_fields(usage: &RunUsage) -> Vec<Field> {
    let mut fields = vec![
        Field::new("Requests", &usage.requests.to_string()),
        Field::new("Model calls", &usage.model.calls.to_string()),
        Field::new("Model tokens", &usage.model.tokens.to_string()),
        Field::new("Browser time", &format!("{} s", usage.browser_time / 1000)),
        Field::new("Proxy bytes", &usage.proxy_bytes.to_string()),
    ];
    for limit in &usage.limits {
        fields.push(Field::new(
            "Limit",
            &format!(
                "{}: {} left of {}",
                limit.usage_kind, limit.left, limit.amount
            ),
        ));
    }
    if usage.uncapped {
        fields.push(Field::new(
            "Uncapped",
            "it spent in a currency no Limit caps",
        ));
    }
    fields
}

/// A Run's report: its axes with their reasons, its times, its error and
/// its warnings.
fn report_fields(report: &RunReport) -> Vec<Field> {
    let mut fields = vec![Field::new("Lifecycle", lifecycle(report.lifecycle))];
    if let Some(reason) = &report.stop_reason {
        fields.push(Field::new("Stop reason", &stop_reason(reason)));
    }
    fields.push(Field::new("Verdict", report.verdict.as_str()));
    for reason in &report.verdict_reasons {
        fields.push(Field::new(
            "Why",
            &format!("{}: {}", reason.code, reason.message),
        ));
    }
    fields.push(Field::new("Completeness", report.completeness.as_str()));
    for reason in &report.completeness_reasons {
        fields.push(Field::new(
            "Why",
            &format!("{}: {}", reason.code, reason.message),
        ));
    }
    fields.push(Field::new("Started", report.started_at.as_str()));
    fields.push(Field::new("Ended", report.ended_at.as_str()));
    if let Some(error) = &report.error {
        fields.push(Field::new(
            "Error",
            &format!("{}: {}", error.code, error.message),
        ));
    }
    for warning in &report.warnings {
        fields.push(Field::new(
            "Warning",
            &format!("{}: {}", warning.code, warning.message),
        ));
    }
    fields
}

/// What the window says of a report read that failed with `code`.
fn unreported(code: &str) -> String {
    if code == goodfetch_protocol::run::RUN_INTERRUPTED.name {
        format!("None: no engine works the Run, and a start with its Run ID resumes it ({code})")
    } else {
        format!("None: the Daemon answered {code}")
    }
}

/// A crawl's part `T` among a progress's or a report's kind parts, under
/// its `requests` key.
fn part<T: serde::de::DeserializeOwned>(parts: &Map<String, Value>) -> Option<T> {
    parts
        .get("requests")
        .and_then(|requests| serde_json::from_value(requests.clone()).ok())
}

/// A stop reason by its kind, as the wire names it.
fn stop_reason(reason: &StopReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.get("kind").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

/// A Run's Recipe, or its Run ID when it ran none.
fn name(row: &RunIndexRow) -> String {
    row.tags
        .reserved(RECIPE_TAG)
        .map_or_else(|| row.run_id.as_str().to_owned(), str::to_owned)
}

/// What the window says of a Lifecycle.
fn lifecycle(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Running => "Running",
        Lifecycle::Stopped => "Stopped",
        Lifecycle::Finished => "Finished",
        Lifecycle::Failed => "Failed",
    }
}
