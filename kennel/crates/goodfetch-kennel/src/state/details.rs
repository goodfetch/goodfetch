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
//!   code and its Run report; and, for a live crawl on a Foxhound Daemon
//!   while `controls` is on, Add Seeds, which asks a confirmation naming
//!   the Run and the consequence before it adds anything. Each of the four
//!   sections is a setting of its own (`[views]`): one turned off is not
//!   shown, and a read only it needs is not made.
//! - **The views** ([`View`]) beside the Runs: the chosen Run's Records,
//!   through `dataset.query`, on a Foxhound Daemon only; the Daemon's
//!   settings in force, through `config.explain`; and the Resource
//!   governor's readings, `engine.system_info`. Each, the Runs table's
//!   included, is a setting of its own (`[views]`), and one turned off is
//!   neither shown nor read; with the Runs table off, no Run is chosen.
//! - **The reads the view needs** ([`Wanted`]): on the Runs view,
//!   `run.progress` only of the running Runs on screen and of the chosen
//!   Run while it runs, the chosen Run's report, which a live Run answers
//!   at its end, and its Usage, read again at each new progress while it
//!   runs; on another view, only what that view shows, the governor's
//!   readings among them. Nothing while the window is closed or no Daemon
//!   is reached.
//! - **The Kennel settings view** (`kennel_settings`): the Kennel's own
//!   settings as `kennel.toml` holds them, which the window edits, with why
//!   the file cannot be read or why an edit failed; a change in the file,
//!   the window's own edits included, comes back through the settings'
//!   watch. It reads nothing from the Daemon.
//! - **Repaints**: only when the view changed, at most once each
//!   [`REDRAW_PERIOD`], and none once the window is closed.
//!
//! Its parts: `fields`, how the Daemon's values are worded; `records`,
//! the Records view; `seeds`, Add Seeds; and `kennel_settings`, the Kennel
//! settings view.
//!
//! Every string shown passes through the Text function.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_protocol::foxhound::report::{CountOf, RequestCounts};
use goodfetch_protocol::foxhound::sql_tab::DatasetQueryResult;
use goodfetch_protocol::governor::SystemInfo;
use goodfetch_protocol::run::{
    Lifecycle, ProgressMessage, RECIPE_TAG, RunId, RunIndexRow, RunListParams, RunProgress,
    RunReport, RunUsage,
};
use goodfetch_protocol::settings::SettingsExplanation;

use super::{Daemon, REDRAW_PERIOD, health_text, product_name, refused};
use crate::client::{
    Call, CallFailure, ConnectionChange, DaemonEvent, Generation, Listing, Query, SeedsAdded,
    UsageRead, Wanted,
};
use crate::settings::{Settings, SettingsError, Views};
use crate::text;

mod fields;
mod kennel_settings;
mod records;
mod seeds;

pub use kennel_settings::{KennelSettingsView, PeriodLine, SettingsSection, SwitchLine};
pub use seeds::{SeedsConfirmation, SeedsOffer};

use super::part;
use fields::{
    governor_fields, lifecycle, point, progress_fields, report_fields, unreported, usage_fields,
};
use records::records_query;
use seeds::Seeding;

#[cfg(test)]
mod tests;

/// The most characters of a value the window shows.
const VALUE_MOST: usize = 300;

/// The most characters of a Run's name in the table.
const NAME_MOST: usize = 60;

/// How many Records the Records view reads at a time.
pub const RECORDS_PAGE: u64 = 50;

/// The window's views, in the order it offers them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// The Runs table and the chosen Run: the window's own view, which
    /// `window` switches.
    #[default]
    Runs,
    /// The chosen Run's Records, on a Foxhound Daemon.
    Records,
    /// The Daemon's settings in force and where each comes from.
    DaemonSettings,
    /// The Resource governor's readings.
    Governor,
    /// The Kennel's own settings, which the window edits.
    KennelSettings,
}

impl View {
    /// What the window calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Runs => "Runs",
            Self::Records => "Records",
            Self::DaemonSettings => "Daemon settings",
            Self::Governor => "Resource governor",
            Self::KennelSettings => "Kennel settings",
        }
    }
}

/// A page of Records to go to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// The next page.
    Next,
    /// The previous page.
    Previous,
}

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
        generation: Generation,
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
    /// The Kennel's settings, as `kennel.toml` gives them, or why it
    /// cannot be read, when the defaults apply.
    Settings(Result<Settings, SettingsError>),
    /// The person's edit of the settings file was not made, and why.
    EditFailed(SettingsError),
    /// The person chose a view.
    Show(View),
    /// The person went to another page of Records.
    RecordsPage(Page),
    /// The person asked to read the shown view again.
    Refresh,
    /// The answer to the Records query of this generation.
    Queried {
        /// The query's generation.
        generation: Generation,
        /// Its result, or why there is none.
        result: Result<DatasetQueryResult, CallFailure>,
    },
    /// The answer to the read of the settings in force of this generation.
    Explained {
        /// The read's generation.
        generation: Generation,
        /// The settings, or why there are none.
        result: Result<Box<SettingsExplanation>, CallFailure>,
    },
    /// The Resource governor's latest reading.
    SystemInfo(Box<SystemInfo>),
    /// What Add Seeds did to a crawl.
    SeedsAdded {
        /// The crawl's Run.
        run_id: RunId,
        /// The counts.
        added: SeedsAdded,
    },
    /// A call the window made was answered, or failed.
    Answered(Call, Result<(), CallFailure>),
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
    /// The views the window offers, in order.
    pub views: Vec<View>,
    /// The view shown; none when the settings turn every view off.
    pub shown: Option<View>,
    /// The Records view, while it shows.
    pub records: Option<RecordsPanel>,
    /// The Daemon settings view, while it shows.
    pub daemon_settings: Option<SettingsPanel>,
    /// The Resource governor's view, while it shows.
    pub governor: Option<GovernorPanel>,
    /// The Kennel settings view, while it shows.
    pub kennel_settings: Option<KennelSettingsView>,
}

/// The Records view: a page of the chosen Run's Records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordsPanel {
    /// Whose Records they are.
    pub title: String,
    /// What the view says instead of Records, or beside them.
    pub note: Option<String>,
    /// The columns' names.
    pub columns: Vec<String>,
    /// The page's Records, a value per column.
    pub rows: Vec<Vec<String>>,
    /// Which Records the page shows, of how many.
    pub range: String,
    /// Whether a previous page exists.
    pub previous: bool,
    /// Whether a next page exists.
    pub next: bool,
}

/// The Daemon settings view: each setting in force and where it comes
/// from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsPanel {
    /// What the view says instead of settings.
    pub note: Option<String>,
    /// The Operator policy's version and the configuration files read.
    pub sources: Vec<Field>,
    /// Each setting, by key.
    pub settings: Vec<SettingLine>,
}

/// One setting in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingLine {
    /// Its key.
    pub key: String,
    /// Its value, with what limits it.
    pub value: String,
    /// The layer that set it.
    pub set_by: String,
    /// The layer that locked it, if any.
    pub locked_by: String,
}

/// The Resource governor's view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GovernorPanel {
    /// What the view says instead of readings.
    pub note: Option<String>,
    /// The latest reading.
    pub readings: Vec<Field>,
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
    /// Its progress, while `views.run_progress` is on.
    pub progress: Option<Vec<Field>>,
    /// Its Usage, with the Limits that apply to it, while `views.run_usage`
    /// is on.
    pub usage: Option<Vec<Field>>,
    /// A crawl's failed requests, by Error code, on a Foxhound Daemon while
    /// `views.run_failures` is on.
    pub failures: Option<Vec<Field>>,
    /// Its Run report, while `views.run_report` is on.
    pub report: Option<Vec<Field>>,
    /// Add Seeds, when the Run offers it.
    pub add_seeds: Option<SeedsOffer>,
    /// What the last Add Seeds of the Run did.
    pub seeds: Option<String>,
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
    generation: Generation,
    /// The table's rows, once the listing of `generation` answered.
    rows: Option<Vec<RunIndexRow>>,
    on_screen: BTreeSet<RunId>,
    chosen: Option<RunId>,
    /// The last progress read of each Run followed.
    progress: BTreeMap<RunId, RunProgress>,
    /// Bumped each time the chosen Run's Usage is to be read again.
    usage_generation: Generation,
    /// The chosen Run's Usage, as `run.usage` last answered.
    usage: Option<(RunId, RunUsage)>,
    /// The chosen Run's report, or why there is none.
    report: Option<(RunId, Result<Box<RunReport>, String>)>,
    /// The Kennel's settings, once read.
    settings: Option<Settings>,
    /// Why the settings file cannot be read, or why the last edit failed.
    settings_note: Option<String>,
    /// The view the person chose.
    view: View,
    /// Bumped each time the Records are to be read again.
    records_generation: Generation,
    /// The first Record of the Records page, from 0.
    records_from: u64,
    /// The Records page's answer, of `records_generation`.
    records: Option<Result<DatasetQueryResult, CallFailure>>,
    /// Bumped each time the settings in force are to be read again.
    explain_generation: Generation,
    /// The settings in force, of `explain_generation`.
    explained: Option<Result<Box<SettingsExplanation>, CallFailure>>,
    /// The Resource governor's latest reading while its view shows.
    reading: Option<Box<SystemInfo>>,
    /// The last Add Seeds, of a Run.
    seeds: Option<(RunId, Seeding)>,
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
            generation: Generation::default(),
            rows: None,
            on_screen: BTreeSet::new(),
            chosen: None,
            progress: BTreeMap::new(),
            usage_generation: Generation::default(),
            usage: None,
            report: None,
            settings: None,
            settings_note: None,
            view: View::Runs,
            records_generation: Generation::default(),
            records_from: 0,
            records: None,
            explain_generation: Generation::default(),
            explained: None,
            reading: None,
            seeds: None,
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
                    self.generation.bump();
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
                    self.usage_generation.bump();
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
                    self.generation.bump();
                }
            }
            Input::Chose(chosen) => {
                if chosen != self.chosen {
                    self.chosen = chosen;
                    self.report = None;
                    self.usage = None;
                    self.usage_generation.bump();
                    self.records_from = 0;
                    self.read_records_again();
                }
            }
            Input::OnScreen(runs) => self.on_screen = runs,
            Input::Settings(read) => self.read_settings(read),
            Input::EditFailed(error) => {
                self.settings_note = Some(format!("The change was not made: {error}"));
            }
            Input::Show(view) => self.show(view),
            Input::RecordsPage(page) => self.go_to(page),
            Input::Refresh => match self.shown() {
                Some(View::Records) => self.read_records_again(),
                Some(View::DaemonSettings) => self.explain_again(),
                Some(View::Runs | View::Governor | View::KennelSettings) | None => {}
            },
            Input::Queried { generation, result } => {
                if generation == self.records_generation {
                    self.records = Some(result);
                }
            }
            Input::Explained { generation, result } => {
                if generation == self.explain_generation {
                    self.explained = Some(result);
                }
            }
            Input::SystemInfo(reading) => {
                if self.shown() == Some(View::Governor) {
                    self.reading = Some(reading);
                }
            }
            Input::SeedsAdded { run_id, added } => {
                self.seeds = Some((run_id, Seeding::Added(added)));
            }
            Input::Answered(call, answer) => {
                if let (Call::AddSeeds(seeds), Err(failure)) = (call, answer) {
                    self.seeds = Some((seeds.run_id, Seeding::Failed(failure)));
                }
            }
            Input::Closed => self.open = false,
        }
    }

    /// Takes the settings `read` in: an unreadable file gives the
    /// defaults, and the Kennel settings view says why.
    fn read_settings(&mut self, read: Result<Settings, SettingsError>) {
        match read {
            Ok(settings) => {
                self.settings = Some(settings);
                self.settings_note = None;
            }
            Err(error) => {
                self.settings = Some(Settings::default());
                self.settings_note = Some(format!(
                    "kennel.toml cannot be read, so the defaults apply: {error}"
                ));
            }
        }
    }

    /// Shows `view`: a view shown again reads afresh what it shows.
    fn show(&mut self, view: View) {
        if view != self.view {
            self.view = view;
            self.read_records_again();
            self.explain_again();
            self.reading = None;
        }
    }

    /// Goes to another page of Records, when there is one.
    fn go_to(&mut self, page: Page) {
        let more = matches!(&self.records, Some(Ok(result))
            if u64::try_from(result.rows.len()).unwrap_or(u64::MAX) < result.total);
        let from = match page {
            Page::Next if more => self.records_from + RECORDS_PAGE,
            Page::Next => self.records_from,
            Page::Previous => self.records_from.saturating_sub(RECORDS_PAGE),
        };
        if from != self.records_from {
            self.records_from = from;
            self.read_records_again();
        }
    }

    /// Reads the settings in force again, dropping the answer shown.
    fn explain_again(&mut self) {
        self.explain_generation.bump();
        self.explained = None;
    }

    /// Reads the Records page again, dropping the answer shown.
    fn read_records_again(&mut self) {
        self.records_generation.bump();
        self.records = None;
    }

    /// The reached Daemon's Product.
    fn product(&self) -> Option<Product> {
        match &self.daemon {
            Daemon::Reached(identity, _) => Some(identity.product),
            Daemon::Looking | Daemon::None | Daemon::Refused(_) => None,
        }
    }

    /// The views and the chosen Run's sections the settings turn on: all
    /// of them before the settings are read.
    fn switched_on(&self) -> Views {
        self.settings
            .as_ref()
            .map(|settings| settings.views.clone())
            .unwrap_or_default()
    }

    /// The views the window offers, each whose setting is on: the Runs,
    /// then the Records, on a Foxhound Daemon only and of a Run chosen in
    /// the Runs, then the others.
    fn views(&self) -> Vec<View> {
        let views = self.switched_on();
        let mut offered = Vec::new();
        if views.runs {
            offered.push(View::Runs);
        }
        if views.runs && views.records && self.product() == Some(Product::Foxhound) {
            offered.push(View::Records);
        }
        if views.daemon_settings {
            offered.push(View::DaemonSettings);
        }
        if views.governor {
            offered.push(View::Governor);
        }
        if views.kennel_settings && self.settings.is_some() {
            offered.push(View::KennelSettings);
        }
        offered
    }

    /// The view shown: the person's, while the window offers it, or else
    /// the first it offers; none when it offers none.
    fn shown(&self) -> Option<View> {
        let offered = self.views();
        if offered.contains(&self.view) {
            Some(self.view)
        } else {
            offered.first().copied()
        }
    }

    /// Whether a chosen crawl's failures by Error code show: their switch
    /// is on, on a Foxhound Daemon.
    fn shows_failures(&self) -> bool {
        self.switched_on().run_failures && self.product() == Some(Product::Foxhound)
    }

    /// The reads the view needs now: none while the window is closed or
    /// no Daemon is reached.
    pub fn wanted(&self) -> Wanted {
        if !self.open || !matches!(self.daemon, Daemon::Reached(..)) {
            return Wanted::default();
        }
        let shown = self.shown();
        let views = self.switched_on();
        let failures = self.shows_failures();
        // The Runs table and the chosen Run show on the Runs view only.
        let runs_shown = shown == Some(View::Runs);
        let mut progress: BTreeSet<RunId> = self
            .on_screen
            .iter()
            .filter(|id| runs_shown && self.runs(id))
            .cloned()
            .collect();
        let chosen = self.chosen.as_ref().filter(|_| runs_shown);
        let chosen_runs = chosen.filter(|id| self.runs(id));
        // A live Run's progress shows its progress and its failures, and
        // paces the reads of its Usage.
        if views.run_progress || views.run_usage || failures {
            progress.extend(chosen_runs.cloned());
        }
        // An ended Run's report holds its Usage and its failures too.
        let report = chosen.filter(|_| views.run_report || views.run_usage || failures);
        Wanted {
            live_runs: Generation::default(),
            progress,
            listing: (self.resynced && views.runs).then(|| Listing {
                generation: self.generation,
                params: self.filter.params(),
            }),
            report: report.cloned(),
            usage: chosen_runs.filter(|_| views.run_usage).map(|id| UsageRead {
                run_id: id.clone(),
                generation: self.usage_generation,
            }),
            records: self
                .chosen
                .as_ref()
                .filter(|_| shown == Some(View::Records))
                .map(|id| Query {
                    generation: self.records_generation,
                    params: records_query(id, self.records_from),
                }),
            explain: (shown == Some(View::DaemonSettings)).then_some(self.explain_generation),
            system_info: shown == Some(View::Governor),
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
        let crawls = self.product() == Some(Product::Foxhound);
        let shown = self.shown();
        let listed = self.switched_on().runs;
        let rows = self.rows.as_deref().filter(|_| listed).unwrap_or_default();
        let note = match (&self.daemon, &self.rows) {
            _ if !listed => None,
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
            chosen: self
                .chosen
                .as_ref()
                .filter(|_| listed)
                .map(|id| self.panel(id, crawls)),
            views: self.views(),
            shown,
            records: (shown == Some(View::Records)).then(|| self.records_panel()),
            daemon_settings: (shown == Some(View::DaemonSettings))
                .then(|| fields::settings_panel(self.explained.as_ref())),
            governor: (shown == Some(View::Governor)).then(|| GovernorPanel {
                note: self
                    .reading
                    .is_none()
                    .then(|| "Waiting for the Resource governor's next reading".to_owned()),
                readings: self
                    .reading
                    .as_deref()
                    .map(governor_fields)
                    .unwrap_or_default(),
            }),
            kennel_settings: self
                .settings
                .as_ref()
                .filter(|_| shown == Some(View::KennelSettings))
                .map(|settings| kennel_settings::view(settings, self.settings_note.as_deref())),
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
        self.records = None;
        self.explained = None;
        self.reading = None;
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
                .as_ref()
                .map_or_else(String::new, |verdict| verdict.as_str().to_owned()),
            completeness: row
                .completeness
                .map_or_else(String::new, |completeness| completeness.as_str().to_owned()),
            started: text::shown(&point(row.started_at), VALUE_MOST),
            ended: row
                .ended_at
                .map_or_else(String::new, |ended| text::shown(&point(ended), VALUE_MOST)),
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
        let views = self.switched_on();
        let failures = self.shows_failures().then(|| {
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
        });
        RunPanel {
            run_id: id.clone(),
            title: text::shown(&title, VALUE_MOST),
            progress: views
                .run_progress
                .then(|| progress_fields(row, progress, crawls)),
            usage: views
                .run_usage
                .then(|| usage.map(usage_fields).unwrap_or_default()),
            failures,
            report: views.run_report.then(|| match report {
                Some(Ok(report)) => report_fields(report),
                Some(Err(code)) => vec![Field::new("Run report", &unreported(code))],
                None if self.runs(id) => {
                    vec![Field::new("Run report", "Comes when the Run ends")]
                }
                None => vec![Field::new("Run report", "Reading…")],
            }),
            add_seeds: self.seeds_offer(id),
            seeds: self
                .seeds
                .as_ref()
                .filter(|(run, _)| run == id)
                .map(|(_, seeding)| self.seeding(id, seeding)),
        }
    }
}

/// `count` of `thing`, plural past one.
fn counted(count: usize, thing: &str) -> String {
    if count == 1 {
        format!("1 {thing}")
    } else {
        format!("{count} {thing}s")
    }
}

/// A Run's Recipe, or its Run ID when it ran none.
fn name(row: &RunIndexRow) -> String {
    row.tags
        .reserved(RECIPE_TAG)
        .map_or_else(|| row.run_id.as_str().to_owned(), str::to_owned)
}
