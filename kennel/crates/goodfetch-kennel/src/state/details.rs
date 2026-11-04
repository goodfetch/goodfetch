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
//!   the Run and the consequence before it adds anything.
//! - **The views** ([`View`]) beside the Runs: the chosen Run's Records,
//!   through `dataset.query`, on a Foxhound Daemon only; the Daemon's
//!   settings in force, through `config.explain`; and the Resource
//!   governor's readings, `engine.system_info`. Each is a setting of its
//!   own (`[views]`), and one turned off is neither shown nor read.
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
//! Every string shown passes through the Text function.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{RequestSeed, Seed};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_protocol::foxhound::report::{CountOf, RequestCounts};
use goodfetch_protocol::foxhound::sql_tab::{DatasetQueryParams, DatasetQueryResult};
use goodfetch_protocol::governor::{
    AllowanceSource, GovernorLevel, KindConcurrency, PressureSource, SystemInfo,
};
use goodfetch_protocol::run::{
    Lifecycle, ProgressMessage, RECIPE_TAG, RunId, RunIndexRow, RunKind, RunListParams,
    RunProgress, RunReport, RunUsage, StopReason,
};
use goodfetch_protocol::settings::{
    ExplainedSetting, RunOrigin, SettingsExplanation, SettingsLayer,
};
use serde_json::{Map, Value};

use super::{Daemon, REDRAW_PERIOD, attempt, health_text, product_name, refused};
use crate::client::{
    Call, CallFailure, ConnectionChange, DaemonEvent, Generation, Listing, Query, SeedsAdded,
    SeedsToAdd, UsageRead, Wanted,
};
use crate::settings::{Settings, SettingsError};
use crate::text;

mod kennel_settings;

pub use kennel_settings::{KennelSettingsView, PeriodLine, SettingsSection, SwitchLine};

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
    /// The view shown.
    pub shown: View,
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
    /// Its progress.
    pub progress: Vec<Field>,
    /// Its Usage, with the Limits that apply to it.
    pub usage: Vec<Field>,
    /// A crawl's failed requests, by Error code.
    pub failures: Vec<Field>,
    /// Its Run report.
    pub report: Vec<Field>,
    /// Add Seeds, when the Run offers it.
    pub add_seeds: Option<SeedsOffer>,
    /// What the last Add Seeds of the Run did.
    pub seeds: Option<String>,
}

/// Add Seeds, offered for a live crawl on a Foxhound Daemon while
/// `controls` is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedsOffer {
    run_id: RunId,
    /// The crawl's name, shown.
    name: String,
}

impl SeedsOffer {
    /// The question Add Seeds asks before it adds the seeds `urls`, one a
    /// line, with the Label `label`: naming the Run and the consequence;
    /// or what the person has yet to give.
    ///
    /// # Errors
    ///
    /// The Label is blank, or no line holds a URL.
    pub fn confirmation(&self, label: &str, urls: &str) -> Result<SeedsConfirmation, String> {
        let label = label.trim();
        if label.is_empty() {
            return Err("Give the Label of the handler the seeds go to".to_owned());
        }
        let seeds: Vec<Seed> = urls
            .lines()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| {
                Seed::Request(RequestSeed {
                    url: url.to_owned(),
                    label: label.to_owned(),
                    payload: None,
                    priority: 0,
                    forefront: false,
                    method: HttpMethod::default(),
                    body: None,
                    key: None,
                })
            })
            .collect();
        if seeds.is_empty() {
            return Err("Give at least one URL, one a line".to_owned());
        }
        let described = if self.name == self.run_id.as_str() {
            self.name.clone()
        } else {
            format!("{} ({})", self.name, self.run_id)
        };
        Ok(SeedsConfirmation {
            title: text::shown(
                &format!("Add {} to {}?", counted(seeds.len(), "seed"), self.name),
                VALUE_MOST,
            ),
            consequence: text::shown(
                &format!(
                    "The crawl {described} fetches them with the Label {label}, beside its other requests. A seed it already holds is not added again, and adding cannot be undone."
                ),
                VALUE_MOST * 2,
            ),
            confirm: "Add seeds".to_owned(),
            run_id: self.run_id.clone(),
            seeds,
        })
    }
}

/// The question Add Seeds asks: the person's yes, given to
/// [`Details::confirmed`], adds the seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedsConfirmation {
    /// The question, naming the Run and how many seeds.
    pub title: String,
    /// What adding does to the crawl.
    pub consequence: String,
    /// The label of the button that confirms.
    pub confirm: String,
    run_id: RunId,
    seeds: Vec<Seed>,
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

/// Where an Add Seeds is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seeding {
    /// Its seeds are being added.
    Adding(usize),
    /// The crawl answered.
    Added(SeedsAdded),
    /// It failed.
    Failed(CallFailure),
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
                View::Records => self.read_records_again(),
                View::DaemonSettings => self.explain_again(),
                View::Runs | View::Governor | View::KennelSettings => {}
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
                if self.shown() == View::Governor {
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

    /// The person said yes to `confirmation`: the call that adds its
    /// seeds, unless the Run no longer offers Add Seeds.
    pub fn confirmed(&mut self, confirmation: SeedsConfirmation) -> Option<Call> {
        let SeedsConfirmation { run_id, seeds, .. } = confirmation;
        self.seeds_offer(&run_id)?;
        self.seeds = Some((run_id.clone(), Seeding::Adding(seeds.len())));
        Some(Call::AddSeeds(SeedsToAdd { run_id, seeds }))
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

    /// The views the window offers: the Runs, then each view whose
    /// setting is on, the Records on a Foxhound Daemon only.
    fn views(&self) -> Vec<View> {
        let views = self
            .settings
            .as_ref()
            .map(|settings| settings.views.clone())
            .unwrap_or_default();
        let mut offered = vec![View::Runs];
        if views.records && self.product() == Some(Product::Foxhound) {
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
    /// the Runs.
    fn shown(&self) -> View {
        if self.views().contains(&self.view) {
            self.view
        } else {
            View::Runs
        }
    }

    /// Add Seeds for the Run `id`, when it offers it: a live crawl, on a
    /// Foxhound Daemon, while the settings, once read, leave `controls`
    /// on.
    fn seeds_offer(&self, id: &RunId) -> Option<SeedsOffer> {
        let controls = self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.controls);
        let row = self.row(id)?;
        let offered = controls
            && self.product() == Some(Product::Foxhound)
            && row.kind == RunKind::Crawl
            && self.runs(id);
        offered.then(|| SeedsOffer {
            run_id: id.clone(),
            name: text::shown(&name(row), NAME_MOST),
        })
    }

    /// The reads the view needs now: none while the window is closed or
    /// no Daemon is reached.
    pub fn wanted(&self) -> Wanted {
        if !self.open || !matches!(self.daemon, Daemon::Reached(..)) {
            return Wanted::default();
        }
        let shown = self.shown();
        // The Runs table and the chosen Run show on the Runs view only.
        let runs_shown = shown == View::Runs;
        let mut progress: BTreeSet<RunId> = self
            .on_screen
            .iter()
            .filter(|id| runs_shown && self.runs(id))
            .cloned()
            .collect();
        let chosen = self.chosen.as_ref().filter(|_| runs_shown);
        let chosen_runs = chosen.filter(|id| self.runs(id));
        progress.extend(chosen_runs.cloned());
        Wanted {
            live_runs: Generation::default(),
            progress,
            listing: self.resynced.then(|| Listing {
                generation: self.generation,
                params: self.filter.params(),
            }),
            report: chosen.cloned(),
            usage: chosen_runs.map(|id| UsageRead {
                run_id: id.clone(),
                generation: self.usage_generation,
            }),
            records: self
                .chosen
                .as_ref()
                .filter(|_| shown == View::Records)
                .map(|id| Query {
                    generation: self.records_generation,
                    params: records_query(id, self.records_from),
                }),
            explain: (shown == View::DaemonSettings).then_some(self.explain_generation),
            system_info: shown == View::Governor,
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
            views: self.views(),
            shown,
            records: (shown == View::Records).then(|| self.records_panel()),
            daemon_settings: (shown == View::DaemonSettings).then(|| self.settings_panel()),
            governor: (shown == View::Governor).then(|| GovernorPanel {
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
                .filter(|_| shown == View::KennelSettings)
                .map(|settings| kennel_settings::view(settings, self.settings_note.as_deref())),
        }
    }

    /// The Records view of the chosen Run.
    fn records_panel(&self) -> RecordsPanel {
        let mut panel = RecordsPanel {
            title: "Records".to_owned(),
            note: None,
            columns: Vec::new(),
            rows: Vec::new(),
            range: String::new(),
            previous: false,
            next: false,
        };
        let Some(id) = &self.chosen else {
            panel.note = Some("Choose a Run in the Runs view to read its Records".to_owned());
            return panel;
        };
        let named = match self.row(id).map(name) {
            Some(name) if name != id.as_str() => format!("{name} ({id})"),
            Some(_) | None => id.as_str().to_owned(),
        };
        panel.title = text::shown(&format!("Records of {named}"), VALUE_MOST);
        match &self.records {
            None => panel.note = Some("Reading the Records…".to_owned()),
            Some(Err(failure)) => {
                panel.note = Some(text::shown(
                    &format!(
                        "The Daemon refused the query: {} ({})",
                        failure.message, failure.code
                    ),
                    VALUE_MOST,
                ));
            }
            Some(Ok(result)) => {
                panel.columns = result
                    .columns
                    .iter()
                    .map(|column| text::shown(&column.name, NAME_MOST))
                    .collect();
                panel.rows = result
                    .rows
                    .iter()
                    .map(|row| row.iter().map(cell).collect())
                    .collect();
                let shown = u64::try_from(panel.rows.len()).unwrap_or(u64::MAX);
                let total = self.records_from + result.total;
                if shown == 0 {
                    panel.note = Some(if self.records_from == 0 {
                        "The Run has no Record".to_owned()
                    } else {
                        "No Record on this page".to_owned()
                    });
                } else {
                    panel.range = format!(
                        "Records {} to {} of {total}",
                        self.records_from + 1,
                        self.records_from + shown
                    );
                }
                panel.previous = self.records_from > 0;
                panel.next = shown < result.total;
            }
        }
        panel
    }

    /// The Daemon settings view.
    fn settings_panel(&self) -> SettingsPanel {
        match &self.explained {
            None => SettingsPanel {
                note: Some("Reading the settings…".to_owned()),
                sources: Vec::new(),
                settings: Vec::new(),
            },
            Some(Err(failure)) => SettingsPanel {
                note: Some(text::shown(
                    &format!(
                        "The Daemon did not explain its settings: {} ({})",
                        failure.message, failure.code
                    ),
                    VALUE_MOST,
                )),
                sources: Vec::new(),
                settings: Vec::new(),
            },
            Some(Ok(explanation)) => {
                let mut sources = Vec::new();
                if let Some(version) = &explanation.policy_version {
                    sources.push(Field::new("Operator policy", version));
                }
                for file in &explanation.files {
                    let found = if file.found { "" } else { " (not found)" };
                    sources.push(Field::new(
                        &format!("{} file", file_layer(file.layer)),
                        &format!("{}{found}", file.path),
                    ));
                }
                SettingsPanel {
                    note: None,
                    sources,
                    settings: explanation.settings.iter().map(setting_line).collect(),
                }
            }
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
            add_seeds: self.seeds_offer(id),
            seeds: self
                .seeds
                .as_ref()
                .filter(|(run, _)| run == id)
                .map(|(_, seeding)| self.seeding(id, seeding)),
        }
    }

    /// What the window says of an Add Seeds of the Run `id`.
    fn seeding(&self, id: &RunId, seeding: &Seeding) -> String {
        let said = match seeding {
            Seeding::Adding(count) => format!("Adding {}…", counted(*count, "seed")),
            Seeding::Added(added) => {
                let count = usize::try_from(added.added).unwrap_or(usize::MAX);
                let not_added: u64 = added.not_added.values().sum();
                if not_added == 0 {
                    format!("{} added", counted(count, "seed"))
                } else {
                    let reasons: Vec<String> = added
                        .not_added
                        .iter()
                        .map(|(reason, count)| format!("{reason} {count}"))
                        .collect();
                    format!(
                        "{} added; {not_added} not added: {}",
                        counted(count, "seed"),
                        reasons.join(", ")
                    )
                }
            }
            Seeding::Failed(failure) => {
                let call = Call::AddSeeds(SeedsToAdd {
                    run_id: id.clone(),
                    seeds: Vec::new(),
                });
                let tried = attempt(&call, |run| {
                    self.row(run).map_or_else(|| run.as_str().to_owned(), name)
                });
                format!("{tried} failed: {} ({})", failure.message, failure.code)
            }
        };
        text::shown(&said, VALUE_MOST)
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

/// The `dataset.query` of a page of `run`'s Records, from the Record
/// `from`: each Dataset's in order.
fn records_query(run: &RunId, from: u64) -> DatasetQueryParams {
    // A quote in the Run ID is doubled, as SQL quotes it.
    let run_literal = run.as_str().replace('\'', "''");
    let offset = if from > 0 {
        format!(" OFFSET {from}")
    } else {
        String::new()
    };
    DatasetQueryParams {
        sql: format!(
            "SELECT dataset, number, url, committed_at, value FROM records('{run_literal}') ORDER BY dataset, number{offset}"
        ),
        runs: vec![run.clone()],
        limit: Some(RECORDS_PAGE),
    }
}

/// A Record's value as the Records view shows it: a text as written,
/// anything else as JSON.
fn cell(value: &Value) -> String {
    let written = match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    text::shown(&written, VALUE_MOST)
}

/// One setting's line of the Daemon settings view.
fn setting_line(setting: &ExplainedSetting) -> SettingLine {
    let mut value = match &setting.value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if let Some(cap) = &setting.cap {
        value = format!("{value}, capped at {cap}");
    }
    if !setting.applied {
        let products: Vec<&str> = setting
            .products
            .iter()
            .map(|product| product_name(*product))
            .collect();
        let honour = if products.len() == 1 {
            "honours"
        } else {
            "honour"
        };
        value = format!(
            "{value}, not applied: only {} {honour} it",
            products.join(" and ")
        );
    }
    let set_by = match (setting.layer, setting.origin) {
        (SettingsLayer::Run, Some(origin)) => format!("the Run ({})", run_origin(origin)),
        (layer, _) => layer_name(layer).to_owned(),
    };
    SettingLine {
        key: text::shown(&setting.key, VALUE_MOST),
        value: text::shown(&value, VALUE_MOST),
        set_by: text::shown(&set_by, VALUE_MOST),
        locked_by: setting
            .locked_by
            .map_or_else(String::new, |layer| layer_name(layer).to_owned()),
    }
}

/// What the window calls the layer that set or locked a setting.
fn layer_name(layer: SettingsLayer) -> &'static str {
    match layer {
        SettingsLayer::Default => "the default",
        SettingsLayer::OperatorPolicy => "the Operator policy",
        SettingsLayer::UserConfiguration => "user configuration",
        SettingsLayer::ProjectConfiguration => "project configuration",
        SettingsLayer::Recipe => "the Recipe",
        SettingsLayer::Run => "the Run",
    }
}

/// What the window calls the layer a configuration file is of.
fn file_layer(layer: SettingsLayer) -> &'static str {
    match layer {
        SettingsLayer::Default => "Default",
        SettingsLayer::OperatorPolicy => "Operator policy",
        SettingsLayer::UserConfiguration => "User configuration",
        SettingsLayer::ProjectConfiguration => "Project configuration",
        SettingsLayer::Recipe => "Recipe",
        SettingsLayer::Run => "Run",
    }
}

/// Where a run-layer value came from.
fn run_origin(origin: RunOrigin) -> &'static str {
    match origin {
        RunOrigin::Argument => "a call's argument",
        RunOrigin::Settings => "a scope's settings",
        RunOrigin::Flag => "a CLI flag",
        RunOrigin::Form => "an Integration's form",
    }
}

/// The Resource governor's reading, as its view shows it.
fn governor_fields(reading: &SystemInfo) -> Vec<Field> {
    let level = match reading.pressure_source {
        Some(source) => format!(
            "{}, held by {}",
            governor_level(reading.level),
            pressure_source(source)
        ),
        None => governor_level(reading.level).to_owned(),
    };
    let mut fields = vec![Field::new("Level", &level)];
    if reading.last_resort {
        fields.push(Field::new(
            "Last resort",
            "memory stayed critical at the floor",
        ));
    }
    let memory = &reading.memory;
    fields.push(Field::new(
        "Memory",
        &format!(
            "{} used of {}, {}",
            bytes(memory.used),
            bytes(memory.allowance),
            allowance_source(memory.allowance_source)
        ),
    ));
    fields.push(Field::new("Browsers' memory", &bytes(memory.browsers)));
    fields.push(Field::new("Host memory", governor_level(memory.host)));
    fields.push(Field::new(
        "CPU",
        &format!(
            "{}% busy, {} threads",
            reading.cpu.busy, reading.cpu.parallelism
        ),
    ));
    fields.push(Field::new(
        "Tokio workers",
        &format!("{}% busy", reading.cpu.tokio_busy),
    ));
    let concurrency = &reading.concurrency;
    for (label, kind) in [
        ("HTTP fetches", &concurrency.http),
        ("Browser pages", &concurrency.browser_page),
        ("Model calls", &concurrency.model_call),
    ] {
        fields.push(Field::new(label, &kind_concurrency(kind)));
    }
    fields
}

fn kind_concurrency(kind: &KindConcurrency) -> String {
    format!(
        "{} in use of {} (from {} to {})",
        kind.in_use, kind.target, kind.floor, kind.ceiling
    )
}

fn governor_level(level: GovernorLevel) -> &'static str {
    match level {
        GovernorLevel::Normal => "normal",
        GovernorLevel::Pressure => "pressure",
        GovernorLevel::Critical => "critical",
    }
}

fn pressure_source(source: PressureSource) -> &'static str {
    match source {
        PressureSource::Memory => "memory",
        PressureSource::HostMemory => "the host's memory",
        PressureSource::Cpu => "CPU",
        PressureSource::Tokio => "the Tokio workers",
    }
}

fn allowance_source(source: AllowanceSource) -> &'static str {
    match source {
        AllowanceSource::Cgroup => "its cgroup's limit",
        AllowanceSource::JobObject => "its Job object's limit",
        AllowanceSource::Setting => "governor.memory_allowance",
        AllowanceSource::Physical => "half of physical memory",
    }
}

/// A count of bytes in GiB, or in MiB below one.
#[expect(clippy::cast_precision_loss, reason = "a reading shown to one decimal")]
fn bytes(count: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let count = count as f64;
    if count >= 1024.0 * MIB {
        format!("{:.1} GiB", count / (1024.0 * MIB))
    } else {
        format!("{:.1} MiB", count / MIB)
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
