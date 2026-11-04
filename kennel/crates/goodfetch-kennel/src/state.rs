//! The Kennel's pure model (ADR 0141): the Client's reports, the settings,
//! the Daemon start's outcomes and the menu's opening and closing go in;
//! the view the Tray draws, when to draw it, the effects to carry out and
//! the reads the view needs come out. It knows no toolkit, socket or
//! clock: the instants it paces redraws by are given to it, so its tests
//! script the inputs and read the outputs.
//!
//! - **The verdict**, worst first: the Daemon unreachable, a Needs you
//!   item, a Run failed or stopped by a Limit, a Run running, idle.
//! - **The live Runs**, from the subscription's events, with their Recipe
//!   or Run ID, their Lifecycle, their progress message and, on a Foxhound
//!   Daemon, a crawl's succeeded, failed and pending counts. A Run that
//!   failed or was stopped by a Limit stays listed, and in the verdict,
//!   until the menu that showed it closes.
//! - **The reads the view needs** ([`Wanted`]): the live Runs' rows of the
//!   Run index after each resync, which starts every Connection's
//!   subscription, and when a Run the rows do not hold starts; the
//!   progress of each stopped Run until
//!   its stop reason is known; and, only while the menu is open, the
//!   progress of each Run it shows.
//! - **Each Run's controls** in a submenu, labelled with what they do: Pause,
//!   Stop and Abort for a running Run, and Retry failed for a running crawl
//!   on a Foxhound Daemon; Resume for a paused or Limit-stopped Run that a
//!   Recipe runs, and "Copy resume command" for a script's (ADR 0031). Stop
//!   asks first as the settings say, Abort and Retry failed always, Pause
//!   never. A read-only Kennel offers none. A call that failed is shown
//!   until the menu that showed it closes.
//! - **Redraws**: at most one a second, and none of the menu while it is
//!   hidden, since the Tray draws the latest view as it opens.
//! - **Needs you**: the items, longest waiting first, each a submenu with
//!   the actions the menu offers for it (`needs_you`). Choosing one gives
//!   the call it makes, [`Effect::Call`], and a call that fails says so
//!   in the menu, with the engine's Error message, until a menu that
//!   showed it closes.
//! - **Desktop notices** (`notices`): a transition that needs a person,
//!   each type a setting, a burst coalesced over the period the settings
//!   give; nothing for the state the Kennel finds at its start or for a
//!   resync's replayed state, and nothing before the settings are read or
//!   while `notifications` is off.
//!
//! - **At launch**, once the settings are read: the hidden-icon check,
//!   asked for once with [`Effect::CheckIcon`] while the setting and the
//!   window are on, whose [`IconCheck::Hidden`] opens the details window
//!   with a note asking the person to free space in the menu bar; and start
//!   at login, [`Effect::StartAtLogin`], at the first read and at each
//!   change of its setting, never from a file that cannot be read.
//!
//! Every string shown, a notice's included, passes through the Text
//! function.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::RunResumeParams;
use goodfetch_protocol::daemon::{DaemonHealth, Unready};
use goodfetch_protocol::foxhound::outcomes::RunRetryFailedParams;
use goodfetch_protocol::foxhound::progress::RequestProgress;
use goodfetch_protocol::needs_you::{NeedsYouChange, NeedsYouItem, RUN_STOPPED_BY_LIMIT};
use goodfetch_protocol::run::{
    Lifecycle, RECIPE_TAG, RunAbortParams, RunAxes, RunId, RunIndexRow, RunKind, RunParams,
    RunProgress, RunWaitParams, StopReason,
};

use crate::client::{
    Call, CallFailure, ConnectionChange, DaemonEvent, DaemonIdentity, Refusal, Wanted,
};
use crate::daemon_start::{InstalledProduct, StartFailure};
use crate::settings::{Settings, SettingsError};
use crate::text;
use crate::window::Opening;

pub mod details;
mod needs_you;
mod notices;

pub use needs_you::Action;
pub use notices::{Notice, Notices};

use notices::{Coalescer, Ended, Kind};

#[cfg(test)]
mod tests;

/// The most Runs the menu lists; the others are counted.
const MENU_RUNS: usize = 10;

/// The most characters of a Recipe's name in the menu.
const NAME_MOST: usize = 40;

/// The most characters of a menu line.
const LINE_MOST: usize = 120;

/// The shortest time between two redraws.
pub const REDRAW_PERIOD: Duration = Duration::from_secs(1);

/// What reaches the State.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The Client's Connection to the Daemon changed.
    Connection(ConnectionChange),
    /// Events of the Daemon's subscription, in order.
    Events(Vec<DaemonEvent>),
    /// The live Runs' rows of the Run index, as `run.list` answered.
    LiveRuns(Vec<RunIndexRow>),
    /// A Run's progress, as `run.progress` answered.
    Progress(RunProgress),
    /// The menu opened or closed.
    Menu(MenuChange),
    /// The installed Product found at launch, whose `daemon start` the
    /// Kennel offers; none when no Product is installed.
    Installed(Option<InstalledProduct>),
    /// The Kennel's settings, read at launch and on each change of their
    /// file.
    Settings(Result<Settings, SettingsError>),
    /// The `daemon start` the person chose has ended.
    StartEnded(Result<(), StartFailure>),
    /// The details window's process ended: the person closed it, or it
    /// could not start.
    DetailsClosed,
    /// A call the person made through the Client was answered, or failed.
    Answered(Call, Result<(), CallFailure>),
    /// What the hidden-icon check read, once at launch.
    IconChecked(IconCheck),
}

/// Whether the Kennel's icon shows in the menu bar, as the status item
/// window's occlusion state says (macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconCheck {
    /// It shows.
    Shown,
    /// It is hidden, behind the notch or in the menu bar's overflow.
    Hidden,
}

/// The menu opened or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChange {
    /// It opened at this instant, drawing the latest view.
    Opened(Instant),
    /// It closed.
    Closed,
}

/// What a person can choose in the menu.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    /// Run the installed Product's `daemon start`.
    StartDaemon,
    /// Open the details window, or bring it forward when it is open.
    OpenDetails,
    /// One of a Run's controls.
    Control {
        /// The Run.
        run: RunId,
        /// The control.
        control: Control,
    },
    /// Resolve a Needs you item through one of its actions.
    NeedsYou {
        /// The item's ID.
        item: String,
        /// The action.
        action: Action,
    },
    /// Quit the Kennel, which stops none of the Runs.
    Quit,
}

/// A Run's control in its submenu, each labelled with what it does (ADR
/// 0141).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    /// A Checkpoint and a stop, resumable: `run.pause`.
    Pause,
    /// A Checkpoint, and the Run can resume: `run.stop`.
    Stop,
    /// No attempt in flight commits; not immediate: `run.abort`.
    Abort,
    /// The crawl's failed requests run again: `run.retry_failed`.
    RetryFailed,
    /// The Daemon runs the stopped Run again: `run.resume`, for a Run the
    /// engine can run itself, a Recipe's.
    Resume,
    /// Copies the command that resumes a script's Run, which only its
    /// program can run (ADR 0031).
    CopyResumeCommand,
}

/// What the Kennel carries out for the State.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Run this Product's `daemon start`, then look for the Daemon again.
    StartDaemon(InstalledProduct),
    /// Start the details window's process, which reads the runtime file
    /// and connects itself, opened as this says.
    OpenDetails(Opening),
    /// Bring the open details window forward.
    RaiseDetails,
    /// Ask the person this, then give the State their yes with
    /// [`State::confirmed`].
    Confirm(Confirmation),
    /// Copy this text to the clipboard.
    Copy(String),
    /// Quit.
    Quit,
    /// Make this call through the Client, and give its answer back as
    /// [`Input::Answered`].
    Call(Call),
    /// Check once whether the icon shows, and give what it read back as
    /// [`Input::IconChecked`].
    CheckIcon,
    /// Make the login agent as start at login now says: on, an agent that
    /// opens the installed `.app`; off, none.
    StartAtLogin(bool),
}

/// A question asked before a control acts: it names the Run and the
/// consequence (ADR 0141).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    /// The question, naming the Run.
    pub title: String,
    /// What the control does to the Run.
    pub consequence: String,
    /// The label of the button that confirms.
    pub confirm: String,
    run: RunId,
    control: Control,
}

/// The one verdict the icon shows, worst first (ADR 0141).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No Daemon this Kennel can talk to.
    Unreachable,
    /// A Needs you item waits.
    NeedsYou,
    /// A Run failed or was stopped by a Limit.
    Failed,
    /// A Run runs.
    Running,
    /// A Daemon is reached and nothing needs the person.
    Idle,
}

/// One row of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A line the person reads.
    Text(String),
    /// A line the person reads under the one above, drawn indented.
    Detail(String),
    /// A line the person chooses.
    Choice {
        /// What the line says.
        label: String,
        /// What choosing it does.
        choice: Choice,
    },
    /// A separator.
    Separator,
    /// A line that opens a submenu of its own rows.
    Submenu {
        /// What the line says.
        label: String,
        /// The submenu's rows, top to bottom.
        rows: Vec<Row>,
    },
}

impl Row {
    /// The row with its text, and its submenu's, made safe to show by the
    /// Text function.
    fn shown(self) -> Self {
        match self {
            Self::Text(line) => Self::Text(text::shown(&line, LINE_MOST)),
            Self::Detail(line) => Self::Detail(text::shown(&line, LINE_MOST)),
            Self::Choice { label, choice } => Self::Choice {
                label: text::shown(&label, LINE_MOST),
                choice,
            },
            Self::Separator => Self::Separator,
            Self::Submenu { label, rows } => Self::Submenu {
                label: text::shown(&label, LINE_MOST),
                rows: rows.into_iter().map(Self::shown).collect(),
            },
        }
    }
}

/// What the Tray draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// The icon's verdict.
    pub verdict: Verdict,
    /// The menu, top to bottom.
    pub rows: Vec<Row>,
}

impl View {
    /// The view of `verdict` and `rows`, each row's text made safe to
    /// show by the Text function.
    fn new(verdict: Verdict, rows: Vec<Row>) -> Self {
        let rows = rows.into_iter().map(Row::shown).collect();
        Self { verdict, rows }
    }
}

/// Whether the Tray redraws now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Redraw {
    /// Draw this now.
    Now(Frame),
    /// A redraw is due at this instant, not before.
    At(Instant),
    /// Nothing to draw.
    Nothing,
}

/// What a redraw draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The icon's new verdict, when it changed.
    pub verdict: Option<Verdict>,
    /// The menu's new rows, when they changed while it is open, or at the
    /// first draw.
    pub rows: Option<Vec<Row>>,
}

/// The Daemon as the Client last reported it.
#[derive(Clone, Debug, PartialEq)]
enum Daemon {
    /// No report yet.
    Looking,
    /// None runs.
    None,
    /// Reached, with its last health reading on this Connection.
    Reached(DaemonIdentity, Option<DaemonHealth>),
    /// One runs, and refused this Kennel.
    Refused(Refusal),
}

/// The Daemon start the person chose.
#[derive(Clone, Debug, PartialEq)]
enum Start {
    /// None chosen, or the last one is over.
    Idle,
    /// `daemon start` runs.
    Running,
    /// `daemon start` ended well, and the Client looks for the Daemon
    /// again.
    Started,
    /// The last one failed.
    Failed(StartFailure),
}

/// Where a Run the State follows is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// It runs.
    Running,
    /// It stopped, and its stop reason is read.
    Stopping,
    /// It was paused, to be resumed: it stays listed until it runs again
    /// or ends.
    Paused,
    /// It failed.
    Failed,
    /// It was stopped by a Limit.
    StoppedByLimit,
}

impl Phase {
    /// Whether it ended in a state that needs the person.
    fn needs_a_look(self) -> bool {
        matches!(self, Self::Failed | Self::StoppedByLimit)
    }
}

/// A Run the State follows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    phase: Phase,
    /// Its progress message, from its last progress read.
    message: Option<String>,
    /// A crawl's counts, from its last progress read.
    counts: Option<Counts>,
    /// Whether an open menu showed it since it ended.
    seen: bool,
    /// Whether its phase came from a resync's replayed state, whose end
    /// sends no desktop notice.
    replayed: bool,
}

impl Run {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            message: None,
            counts: None,
            seen: false,
            replayed: false,
        }
    }
}

/// A crawl's requests the menu counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counts {
    succeeded: u64,
    failed: u64,
    pending: u64,
}

/// What the Run index's row says of a live Run.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Facts {
    kind: RunKind,
    recipe: Option<String>,
    message: Option<String>,
}

/// A call that failed, which the menu says until a menu that showed it
/// closes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Failed {
    /// What the menu says.
    line: String,
    /// Whether an open menu showed it.
    seen: bool,
}

/// Where the hidden-icon check stands: it is made once, at launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// The settings are not read yet.
    Waiting,
    /// Asked for, and its reading not in yet.
    Asked,
    /// Made, or not asked for by the settings at launch.
    Done,
}

/// What the Tray last drew.
#[derive(Clone, Debug)]
struct Drawn {
    at: Instant,
    verdict: Verdict,
    rows: Vec<Row>,
}

/// The Kennel's model.
#[derive(Debug)]
pub struct State {
    daemon: Daemon,
    start: Start,
    installed: Option<InstalledProduct>,
    settings: Settings,
    settings_error: Option<SettingsError>,
    /// Where the hidden-icon check stands.
    icon_check: Check,
    /// What start at login was last made, from a readable settings file.
    start_at_login: Option<bool>,
    /// The Runs followed: the live ones, those that stopped for a reason
    /// not yet read, and those that ended in a state that needs a look.
    runs: BTreeMap<RunId, Run>,
    /// The live Runs' rows, as `run.list` last answered.
    facts: BTreeMap<RunId, Facts>,
    /// The Needs you items listed, by ID.
    needs_you: BTreeMap<String, NeedsYouItem>,
    /// The last call that failed, until a menu that showed it closes.
    failed: Option<Failed>,
    /// Bumped each time the live Runs' rows are to be read again.
    live_runs: u64,
    menu_open: bool,
    /// Whether the details window's process runs.
    details_open: bool,
    drawn: Option<Drawn>,
    /// The desktop notices decided and not yet sent.
    notices: Coalescer,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// The State at launch: no report from the Client yet, no installed
    /// Product found and the default settings.
    pub fn new() -> Self {
        Self {
            daemon: Daemon::Looking,
            start: Start::Idle,
            installed: None,
            settings: Settings::default(),
            settings_error: None,
            icon_check: Check::Waiting,
            start_at_login: None,
            runs: BTreeMap::new(),
            facts: BTreeMap::new(),
            needs_you: BTreeMap::new(),
            failed: None,
            live_runs: 0,
            menu_open: false,
            details_open: false,
            drawn: None,
            notices: Coalescer::default(),
        }
    }

    /// Takes `input` in, and gives the effects it calls for: the launch's
    /// hidden-icon check and start at login, never a Daemon's start.
    pub fn apply(&mut self, input: Input) -> Vec<Effect> {
        let mut effects = Vec::new();
        match input {
            Input::Connection(change) => self.connection(change),
            Input::Events(events) => {
                for event in events {
                    self.event(event);
                }
            }
            Input::LiveRuns(rows) => {
                let mut facts: BTreeMap<RunId, Facts> = rows
                    .into_iter()
                    .map(|row| {
                        let facts = Facts {
                            kind: row.kind,
                            recipe: row.tags.reserved(RECIPE_TAG).map(str::to_owned),
                            message: row.progress_message,
                        };
                        (row.run_id, facts)
                    })
                    .collect();
                // A Run still listed that no longer runs keeps what its
                // last row said: its name and whether it can resume.
                for (id, kept) in std::mem::take(&mut self.facts) {
                    if self.runs.contains_key(&id) {
                        facts.entry(id).or_insert(kept);
                    }
                }
                self.facts = facts;
            }
            Input::Progress(progress) => self.progress(&progress),
            Input::Menu(MenuChange::Opened(at)) => {
                self.menu_open = true;
                let view = self.view();
                let verdict = self
                    .drawn
                    .as_ref()
                    .map_or(view.verdict, |drawn| drawn.verdict);
                self.drawn = Some(Drawn {
                    at,
                    verdict,
                    rows: view.rows,
                });
                self.mark_seen();
            }
            Input::Menu(MenuChange::Closed) => {
                self.menu_open = false;
                self.runs
                    .retain(|_, run| !(run.phase.needs_a_look() && run.seen));
                if self.failed.as_ref().is_some_and(|failed| failed.seen) {
                    self.failed = None;
                }
            }
            Input::Installed(installed) => self.installed = installed,
            Input::Settings(Ok(settings)) => {
                self.notices.settings(settings.notifications);
                if self.start_at_login != Some(settings.start_at_login) {
                    self.start_at_login = Some(settings.start_at_login);
                    effects.push(Effect::StartAtLogin(settings.start_at_login));
                }
                self.settings = settings;
                self.settings_error = None;
                effects.extend(self.ask_icon_check());
            }
            Input::Settings(Err(error)) => {
                self.settings = Settings::default();
                self.notices.settings(self.settings.notifications);
                self.settings_error = Some(error);
                effects.extend(self.ask_icon_check());
            }
            Input::IconChecked(check) => {
                let opens = check == IconCheck::Hidden
                    && self.icon_check == Check::Asked
                    && self.checks_the_icon()
                    && !self.details_open;
                self.icon_check = Check::Done;
                if opens {
                    self.details_open = true;
                    effects.push(Effect::OpenDetails(Opening::HiddenIcon));
                }
            }
            Input::StartEnded(Ok(())) => self.start = Start::Started,
            Input::StartEnded(Err(failure)) => self.start = Start::Failed(failure),
            Input::DetailsClosed => self.details_open = false,
            // An answered call shows in what it changes, which the events
            // bring.
            Input::Answered(_, Ok(())) => {}
            Input::Answered(call, Err(failure)) => {
                self.failed = Some(Failed {
                    line: format!(
                        "{} failed: {}",
                        attempt(&call, |run| self.name(run)),
                        failure.message
                    ),
                    seen: false,
                });
            }
        }
        effects
    }

    /// Whether the settings ask for the hidden-icon check: it is on, and
    /// the window it would open is too.
    fn checks_the_icon(&self) -> bool {
        self.settings.hidden_icon_check && self.settings.window
    }

    /// The hidden-icon check, asked for once, at the first read of the
    /// settings that asks for it.
    fn ask_icon_check(&mut self) -> Option<Effect> {
        if self.icon_check != Check::Waiting {
            return None;
        }
        if !self.checks_the_icon() {
            self.icon_check = Check::Done;
            return None;
        }
        self.icon_check = Check::Asked;
        Some(Effect::CheckIcon)
    }

    /// The person chose `choice` in the menu; a choice the menu no longer
    /// offers does nothing.
    pub fn choose(&mut self, choice: Choice) -> Vec<Effect> {
        match choice {
            Choice::Quit => vec![Effect::Quit],
            Choice::OpenDetails if !self.settings.window => Vec::new(),
            Choice::OpenDetails if self.details_open => vec![Effect::RaiseDetails],
            Choice::OpenDetails => {
                self.details_open = true;
                vec![Effect::OpenDetails(Opening::Asked)]
            }
            Choice::Control { run, control } => {
                if !self.offered(&run).contains(&control) {
                    return Vec::new();
                }
                match self.confirmation(&run, control) {
                    Some(confirmation) => vec![Effect::Confirm(confirmation)],
                    None => self.act(run, control).into_iter().collect(),
                }
            }
            Choice::NeedsYou { item, action } => self
                .needs_you
                .get(&item)
                .filter(|_| self.settings.controls && matches!(self.daemon, Daemon::Reached(..)))
                .and_then(|item| needs_you::call(item, action))
                .map(Effect::Call)
                .into_iter()
                .collect(),
            Choice::StartDaemon => match self.startable() {
                Some(installed) => {
                    let effect = Effect::StartDaemon(installed.clone());
                    self.start = Start::Running;
                    vec![effect]
                }
                None => Vec::new(),
            },
        }
    }

    /// The person said yes to `confirmation`: its control acts, unless
    /// the menu no longer offers it.
    pub fn confirmed(&mut self, confirmation: Confirmation) -> Vec<Effect> {
        let Confirmation { run, control, .. } = confirmation;
        if !self.offered(&run).contains(&control) {
            return Vec::new();
        }
        self.act(run, control).into_iter().collect()
    }

    /// The reads the view needs now.
    pub fn wanted(&self) -> Wanted {
        let mut progress: BTreeSet<RunId> = self
            .runs
            .iter()
            .filter(|(_, run)| run.phase == Phase::Stopping)
            .map(|(id, _)| id.clone())
            .collect();
        if self.menu_open && matches!(self.daemon, Daemon::Reached(..)) {
            progress.extend(
                self.listed()
                    .0
                    .into_iter()
                    .filter(|(_, run)| run.phase == Phase::Running)
                    .map(|(id, _)| id.clone()),
            );
        }
        Wanted {
            live_runs: self.live_runs,
            progress,
            ..Wanted::default()
        }
    }

    /// Whether desktop notices are on: the settings are read and say so.
    /// While they are off the Kennel builds no notice adapter.
    pub fn notifying(&self) -> bool {
        self.notices.notifying()
    }

    /// The desktop notices to send at `now`: the first of a burst at
    /// once, then one summary per period, which falls due at
    /// [`Notices::due`].
    pub fn notices(&mut self, now: Instant) -> Notices {
        self.notices.take(now)
    }

    /// What the Tray draws now.
    pub fn view(&self) -> View {
        let mut rows = Vec::new();
        match (&self.start, &self.daemon) {
            (Start::Running | Start::Started, _) => {
                rows.push(Row::Text("Starting the Daemon…".to_owned()));
            }
            (_, Daemon::Looking) => rows.push(Row::Text("Looking for the Daemon…".to_owned())),
            (_, Daemon::Reached(identity, health)) => {
                rows.push(Row::Text(format!(
                    "{} Daemon {}",
                    product_name(identity.product),
                    identity.product_version
                )));
                if let Some(health) = health {
                    rows.push(Row::Text(health_text(health)));
                }
            }
            (_, Daemon::Refused(refusal)) => rows.push(Row::Text(refused(refusal))),
            (start, Daemon::None) => {
                rows.push(Row::Text("No Daemon".to_owned()));
                if let Start::Failed(failure) = start {
                    rows.push(Row::Text(format!("The Daemon did not start: {failure}")));
                }
                if self.settings.controls {
                    rows.push(match self.installed {
                        Some(_) => Row::Choice {
                            label: "Start Daemon".to_owned(),
                            choice: Choice::StartDaemon,
                        },
                        None => Row::Text(
                            "Start Daemon: no installed Collie or Foxhound was found".to_owned(),
                        ),
                    });
                }
            }
        }
        if let Some(error) = &self.settings_error {
            rows.push(Row::Text(format!(
                "kennel.toml cannot be read, so the defaults apply: {error}"
            )));
        }
        if let Some(failed) = &self.failed {
            rows.push(Row::Text(failed.line.clone()));
        }
        if let Daemon::Reached(identity, _) = &self.daemon {
            self.run_rows(identity.product, &mut rows);
            needs_you::rows(&self.needs_you, self.settings.controls, &mut rows);
        }
        rows.push(Row::Separator);
        if self.settings.window {
            rows.push(Row::Choice {
                label: "Open details".to_owned(),
                choice: Choice::OpenDetails,
            });
        }
        rows.push(Row::Choice {
            label: "Quit".to_owned(),
            choice: Choice::Quit,
        });
        View::new(self.verdict(), rows)
    }

    /// Whether the Tray redraws at `now`: when the view changed since its
    /// last draw, at most once each [`REDRAW_PERIOD`], and the menu's rows
    /// only while the menu is open.
    pub fn redraw(&mut self, now: Instant) -> Redraw {
        let view = self.view();
        let Some(drawn) = &self.drawn else {
            self.drawn = Some(Drawn {
                at: now,
                verdict: view.verdict,
                rows: view.rows.clone(),
            });
            return Redraw::Now(Frame {
                verdict: Some(view.verdict),
                rows: Some(view.rows),
            });
        };
        let verdict = (drawn.verdict != view.verdict).then_some(view.verdict);
        let rows = (self.menu_open && drawn.rows != view.rows).then_some(view.rows);
        if verdict.is_none() && rows.is_none() {
            return Redraw::Nothing;
        }
        let due = drawn.at + REDRAW_PERIOD;
        if now < due {
            return Redraw::At(due);
        }
        let drawn_rows = rows.clone().unwrap_or_else(|| drawn.rows.clone());
        self.drawn = Some(Drawn {
            at: now,
            verdict: verdict.unwrap_or(drawn.verdict),
            rows: drawn_rows,
        });
        if rows.is_some() {
            self.mark_seen();
        }
        Redraw::Now(Frame { verdict, rows })
    }

    fn connection(&mut self, change: ConnectionChange) {
        // The Daemon lost while a Run was live: its Connection ends with
        // no Daemon reached in its place.
        if let Daemon::Reached(identity, _) = &self.daemon
            && matches!(
                change,
                ConnectionChange::NoDaemon | ConnectionChange::Refused(_)
            )
        {
            let product = identity.product;
            let live = self
                .runs
                .values()
                .filter(|run| run.phase == Phase::Running)
                .count();
            if live > 0 {
                self.notices
                    .push(Kind::DaemonLost, || notices::daemon_lost(product, live));
            }
        }
        self.daemon = match change {
            ConnectionChange::NoDaemon => Daemon::None,
            // Each Connection starts with no reading: nothing the last one
            // showed carries over.
            ConnectionChange::Connected(identity) => Daemon::Reached(identity, None),
            ConnectionChange::Refused(refusal) => Daemon::Refused(refusal),
            ConnectionChange::Health(reading) => {
                // A reading the Client sent before it lost the Daemon is
                // of no Daemon now.
                if let Daemon::Reached(_, health) = &mut self.daemon {
                    *health = Some(reading);
                }
                return;
            }
        };
        // What the Daemon holds comes again with the resync that starts
        // each Connection's subscription, after which the live Runs' rows
        // are read; the Runs that need a look are the Kennel's own record.
        self.runs.retain(|_, run| run.phase.needs_a_look());
        self.needs_you.clear();
        let reached = matches!(self.daemon, Daemon::Reached(..));
        // A start that ended well waits for the Client's next report; a
        // reached Daemon makes a failed one moot.
        if self.start == Start::Started || (reached && matches!(self.start, Start::Failed(_))) {
            self.start = Start::Idle;
        }
    }

    fn event(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::Resync(resync) => {
                let live: BTreeSet<&RunId> = resync.runs.iter().map(|axes| &axes.run_id).collect();
                // A Run held as running that the resync leaves out ended.
                for (id, run) in &mut self.runs {
                    if run.phase == Phase::Running && !live.contains(id) {
                        run.phase = Phase::Stopping;
                    }
                }
                for axes in resync.runs {
                    self.axes(axes, true);
                }
                self.needs_you = resync
                    .needs_you
                    .into_iter()
                    .map(|item| (item.id.clone(), item))
                    .collect();
                self.live_runs += 1;
            }
            DaemonEvent::RunAxes(axes) => {
                let known = self.facts.contains_key(&axes.run_id);
                if axes.lifecycle == Lifecycle::Running && !known {
                    self.live_runs += 1;
                }
                self.axes(axes, false);
            }
            DaemonEvent::NeedsYou(change) => match change {
                NeedsYouChange::Listed { item } => {
                    if !self.needs_you.contains_key(&item.id) && !self.limit_noticed(&item) {
                        let summary = item.summary.clone();
                        self.notices
                            .push(Kind::NeedsYou, || notices::needs_you(&summary));
                    }
                    self.needs_you.insert(item.id.clone(), item);
                }
                NeedsYouChange::Removed { id, .. } => {
                    self.needs_you.remove(&id);
                }
            },
        }
    }

    /// A Run's axes as they are now, `replayed` by a resync or changed.
    fn axes(&mut self, axes: RunAxes, replayed: bool) {
        let phase = match axes.lifecycle {
            Lifecycle::Running => Phase::Running,
            Lifecycle::Failed => Phase::Failed,
            // Its stop reason, which says whether a Limit stopped it, is
            // read from its progress.
            Lifecycle::Stopped => Phase::Stopping,
            Lifecycle::Finished => {
                // A Run the State does not follow ended before it learned
                // of it, or its progress already said so.
                if let Some(run) = self.runs.remove(&axes.run_id)
                    && !replayed
                    && !run.replayed
                {
                    self.ended(&axes.run_id, Some(run.phase), Ended::Finished);
                }
                return;
            }
        };
        let id = axes.run_id;
        let was = self.runs.get(&id).map(|run| run.phase);
        let run = self
            .runs
            .entry(id.clone())
            .or_insert_with(|| Run::new(phase));
        if was != Some(phase) {
            run.phase = phase;
            run.seen = false;
            // A running Run's end is a change, whatever listed it.
            run.replayed = replayed && phase != Phase::Running;
            if phase == Phase::Failed && !replayed {
                self.ended(&id, was, Ended::Failed);
            }
        }
    }

    /// Sends the notice of the Run `id` ending as `ended` says, from the
    /// phase `was` it was followed in, if any; none when it had already
    /// ended in a state that needs a look, or when the notice of its Needs
    /// you item tells the same.
    fn ended(&mut self, id: &RunId, was: Option<Phase>, ended: Ended) {
        if was.is_some_and(Phase::needs_a_look) {
            return;
        }
        if ended == Ended::StoppedByLimit && self.notices.wants(Kind::NeedsYou) {
            let told = self
                .needs_you
                .values()
                .any(|item| item.kind == RUN_STOPPED_BY_LIMIT && item.run.as_ref() == Some(id));
            if told {
                return;
            }
        }
        let name = self.run_name(id);
        self.notices
            .push(ended.kind(), || notices::run_ended(ended, &name));
    }

    /// Whether `item` says a Run stopped by a Limit whose own notice was
    /// already sent.
    fn limit_noticed(&self, item: &NeedsYouItem) -> bool {
        item.kind == RUN_STOPPED_BY_LIMIT
            && self.notices.wants(Kind::RunStoppedByLimit)
            && item
                .run
                .as_ref()
                .and_then(|id| self.runs.get(id))
                .is_some_and(|run| run.phase == Phase::StoppedByLimit && !run.replayed)
    }

    /// What names the Run `id`: its Recipe, or its Run ID when it ran
    /// none.
    fn run_name(&self, id: &RunId) -> String {
        self.facts
            .get(id)
            .and_then(|facts| facts.recipe.as_deref())
            .map_or_else(
                || id.as_str().to_owned(),
                |recipe| text::shown(recipe, NAME_MOST),
            )
    }

    fn progress(&mut self, progress: &RunProgress) {
        let Some(run) = self.runs.get_mut(&progress.run_id) else {
            return;
        };
        if let Some(message) = &progress.message {
            run.message = Some(message.as_str().to_owned());
        }
        if let Some(requests) = progress
            .kind_parts
            .get("requests")
            .and_then(|requests| serde_json::from_value::<RequestProgress>(requests.clone()).ok())
        {
            run.counts = Some(Counts {
                succeeded: requests.succeeded,
                failed: requests.failed,
                pending: requests.pending,
            });
        }
        let phase = match (progress.lifecycle, &progress.stop_reason, progress.last) {
            (Lifecycle::Running, _, _) => Phase::Running,
            (Lifecycle::Failed, _, _) => Phase::Failed,
            (Lifecycle::Stopped, Some(StopReason::Limit(_)), _) => Phase::StoppedByLimit,
            (Lifecycle::Stopped, Some(StopReason::Pause), _) => Phase::Paused,
            (Lifecycle::Stopped, _, false) => Phase::Stopping,
            (Lifecycle::Stopped, _, true) => {
                self.runs.remove(&progress.run_id);
                return;
            }
            (Lifecycle::Finished, _, _) => {
                let (was, replayed) = (run.phase, run.replayed);
                self.runs.remove(&progress.run_id);
                if !replayed {
                    self.ended(&progress.run_id, Some(was), Ended::Finished);
                }
                return;
            }
        };
        let (was, replayed) = (run.phase, run.replayed);
        if was != phase {
            run.phase = phase;
            run.seen = false;
            run.replayed = replayed && phase != Phase::Running;
            let ended = match phase {
                Phase::Failed => Some(Ended::Failed),
                Phase::StoppedByLimit => Some(Ended::StoppedByLimit),
                Phase::Running | Phase::Stopping | Phase::Paused => None,
            };
            if let Some(ended) = ended
                && !replayed
            {
                self.ended(&progress.run_id, Some(was), ended);
            }
        }
    }

    /// The verdict, worst first.
    fn verdict(&self) -> Verdict {
        if !matches!(self.daemon, Daemon::Reached(..)) {
            Verdict::Unreachable
        } else if !self.needs_you.is_empty() {
            Verdict::NeedsYou
        } else if self.runs.values().any(|run| run.phase.needs_a_look()) {
            Verdict::Failed
        } else if self.runs.values().any(|run| run.phase == Phase::Running) {
            Verdict::Running
        } else {
            Verdict::Idle
        }
    }

    /// The Runs the menu lists, those that need a look first, then by Run
    /// ID, and how many more it leaves out.
    fn listed(&self) -> (Vec<(&RunId, &Run)>, usize) {
        let mut listed: Vec<(&RunId, &Run)> = self
            .runs
            .iter()
            .filter(|(_, run)| run.phase != Phase::Stopping)
            .collect();
        listed.sort_by_key(|(id, run)| (!run.phase.needs_a_look(), *id));
        let more = listed.len().saturating_sub(MENU_RUNS);
        listed.truncate(MENU_RUNS);
        (listed, more)
    }

    /// The menu's rows of the Runs, under a Daemon of `product`.
    fn run_rows(&self, product: Product, rows: &mut Vec<Row>) {
        let (listed, more) = self.listed();
        if listed.is_empty() {
            return;
        }
        rows.push(Row::Separator);
        for (id, run) in listed {
            let facts = self.facts.get(id);
            let label = format!("{} — {}", self.name(id), lifecycle(run.phase));
            let controls = self.offered(id);
            rows.push(if controls.is_empty() {
                Row::Text(label)
            } else {
                Row::Submenu {
                    label,
                    rows: controls
                        .into_iter()
                        .map(|control| Row::Choice {
                            label: control_label(control).to_owned(),
                            choice: Choice::Control {
                                run: id.clone(),
                                control,
                            },
                        })
                        .collect(),
                }
            });
            let message = run
                .message
                .as_deref()
                .or_else(|| facts.and_then(|facts| facts.message.as_deref()));
            if let Some(message) = message {
                rows.push(Row::Detail(message.to_owned()));
            }
            // A crawl's counts are Foxhound's.
            if let (Product::Foxhound, Some(counts)) = (product, run.counts) {
                rows.push(Row::Detail(format!(
                    "{} succeeded, {} failed, {} pending",
                    counts.succeeded, counts.failed, counts.pending
                )));
            }
        }
        if more > 0 {
            rows.push(Row::Text(format!(
                "{more} more {}",
                if more == 1 { "Run" } else { "Runs" }
            )));
        }
    }

    /// What the menu calls `id`: its Recipe, or its Run ID when it ran
    /// none.
    fn name(&self, id: &RunId) -> String {
        self.facts
            .get(id)
            .and_then(|facts| facts.recipe.as_deref())
            .map_or_else(
                || id.as_str().to_owned(),
                |recipe| text::shown(recipe, NAME_MOST),
            )
    }

    /// The controls the menu offers for `id`, in their order: none in a
    /// read-only Kennel, for a Run it does not list, or with no Daemon.
    fn offered(&self, id: &RunId) -> Vec<Control> {
        let Daemon::Reached(identity, _) = &self.daemon else {
            return Vec::new();
        };
        let listed = self
            .listed()
            .0
            .into_iter()
            .find(|(listed, _)| *listed == id);
        let (Some((_, run)), true) = (listed, self.settings.controls) else {
            return Vec::new();
        };
        let facts = self.facts.get(id);
        match run.phase {
            Phase::Running => {
                let mut controls = vec![Control::Pause, Control::Stop, Control::Abort];
                // A crawl's retry is Foxhound's.
                let crawl = facts.is_some_and(|facts| facts.kind == RunKind::Crawl);
                if crawl && identity.product == Product::Foxhound {
                    controls.push(Control::RetryFailed);
                }
                controls
            }
            // The engine runs a Recipe's Run itself; only its program runs
            // a script's (ADR 0031).
            Phase::Paused | Phase::StoppedByLimit => {
                if facts.is_some_and(|facts| facts.recipe.is_some()) {
                    vec![Control::Resume]
                } else {
                    vec![Control::CopyResumeCommand]
                }
            }
            Phase::Stopping | Phase::Failed => Vec::new(),
        }
    }

    /// The question `control` asks before it acts on `run`: Abort and
    /// Retry failed always ask, Stop as the settings say, the others never.
    fn confirmation(&self, run: &RunId, control: Control) -> Option<Confirmation> {
        let name = self.name(run);
        let (title, consequence, confirm) = match control {
            Control::Stop if self.settings.confirm_stop => (
                format!("Stop {name}?"),
                "It stops at its next safe point with a Checkpoint, and it can resume.",
                "Stop",
            ),
            Control::Abort => (
                format!("Abort {name}?"),
                "It stops at its next safe point, not at once. No attempt in flight commits, and its work items return to pending.",
                "Abort",
            ),
            Control::RetryFailed => (
                format!("Retry the failed requests of {name}?"),
                "Each request whose latest outcome is failed runs again in this crawl, with its retries counted afresh.",
                "Retry failed",
            ),
            Control::Stop | Control::Pause | Control::Resume | Control::CopyResumeCommand => {
                return None;
            }
        };
        Some(Confirmation {
            title: text::shown(&title, LINE_MOST),
            consequence: text::shown(consequence, LINE_MOST * 2),
            confirm: text::shown(confirm, LINE_MOST),
            run: run.clone(),
            control,
        })
    }

    /// What `control` does to `run` once it may act.
    fn act(&self, run: RunId, control: Control) -> Option<Effect> {
        let call = match control {
            Control::Pause => Call::Pause(RunParams { run_id: run }),
            Control::Stop => Call::Stop(RunWaitParams {
                run_id: run,
                timeout: None,
            }),
            Control::Abort => Call::Abort(RunAbortParams {
                run_id: run,
                forwarded: false,
            }),
            Control::RetryFailed => Call::RetryFailed(RunRetryFailedParams {
                run_id: run,
                blocked: false,
                settings: serde_json::Map::new(),
                project: None,
            }),
            Control::Resume => Call::Resume(RunResumeParams {
                run_id: run,
                checkpoint: None,
                settings: serde_json::Map::new(),
                project: None,
            }),
            Control::CopyResumeCommand => {
                let Daemon::Reached(identity, _) = &self.daemon else {
                    return None;
                };
                // A Run ID holds only lowercase letters, digits, `.`, `_`
                // and `-`: the command needs no quoting.
                return Some(Effect::Copy(format!(
                    "goodfetch-{} run resume {}",
                    identity.product.as_str(),
                    run.as_str()
                )));
            }
        };
        Some(Effect::Call(call))
    }

    /// Marks each Run that needs a look and that the open menu lists as
    /// seen, and the failed call it shows.
    fn mark_seen(&mut self) {
        let listed: Vec<RunId> = self
            .listed()
            .0
            .into_iter()
            .filter(|(_, run)| run.phase.needs_a_look())
            .map(|(id, _)| id.clone())
            .collect();
        for id in listed {
            if let Some(run) = self.runs.get_mut(&id) {
                run.seen = true;
            }
        }
        if let Some(failed) = &mut self.failed {
            failed.seen = true;
        }
    }

    /// The Product whose `daemon start` the menu offers now.
    fn startable(&self) -> Option<&InstalledProduct> {
        let idle = matches!(self.start, Start::Idle | Start::Failed(_));
        let offered = idle && self.daemon == Daemon::None && self.settings.controls;
        self.installed.as_ref().filter(|_| offered)
    }
}

/// What the menu says a failed `call` was trying to do, naming a Run by
/// `name`.
fn attempt(call: &Call, name: impl Fn(&RunId) -> String) -> String {
    match call {
        Call::RaiseLimit(params) => format!("Raising the Limit on {}", params.usage_kind),
        Call::AllowConsentDomain(params) => {
            format!("Accepting the consent walls of {}", params.domain)
        }
        Call::DenyWallDomain(params) => format!("Denying {}", params.domain),
        Call::RemoveRefetchDomain(params) => {
            format!("Removing {} from the refetch domains", params.domain)
        }
        Call::Dismiss(_) => "Dismissing a Needs you item".to_owned(),
        Call::Pause(params) => format!("Pausing {}", name(&params.run_id)),
        Call::Stop(params) => format!("Stopping {}", name(&params.run_id)),
        Call::Abort(params) => format!("Aborting {}", name(&params.run_id)),
        Call::Resume(params) => format!("Resuming {}", name(&params.run_id)),
        Call::RetryFailed(params) => {
            format!("Retrying the failed requests of {}", name(&params.run_id))
        }
    }
}

/// What the menu says of a Run's Lifecycle.
fn lifecycle(phase: Phase) -> &'static str {
    match phase {
        Phase::Running | Phase::Stopping => "Running",
        Phase::Paused => "Paused",
        Phase::Failed => "Failed",
        Phase::StoppedByLimit => "Stopped by a Limit",
    }
}

/// What a control's line in a Run's submenu says it does (ADR 0141).
fn control_label(control: Control) -> &'static str {
    match control {
        Control::Pause => "Pause: a Checkpoint and a stop",
        Control::Stop => "Stop: a Checkpoint, and the Run can resume",
        Control::Abort => "Abort: no attempt in flight commits; not immediate",
        Control::RetryFailed => "Retry failed: its failed requests run again",
        Control::Resume => "Resume: the Daemon runs it again from where it stopped",
        Control::CopyResumeCommand => "Copy resume command",
    }
}

/// What the menu says of a Daemon that refused the Kennel.
fn refused(refusal: &Refusal) -> String {
    match refusal {
        Refusal::ProtocolVersion { engine, needs } => format!(
            "The Daemon speaks protocol {engine}, and this Kennel needs a Daemon of protocol {needs}"
        ),
        Refusal::HandOver => "The Daemon is handing over to its major's newer version".to_owned(),
        Refusal::Other { code } => format!("The Daemon refused this Kennel: {code}"),
    }
}

/// What the menu says of the reached Daemon's health: serving, draining,
/// or why it takes no new Run.
fn health_text(health: &DaemonHealth) -> String {
    if !health.live {
        return "Not live: its Runs' registry does not respond".to_owned();
    }
    if health.ready {
        return "Serving".to_owned();
    }
    let draining = health.unready.contains(&Unready::Draining);
    let others: Vec<&str> = health
        .unready
        .iter()
        .filter_map(|unready| match unready {
            Unready::Draining => None,
            Unready::RunStateNotWritable => Some("its run state cannot be written"),
            Unready::BrowsersMissing => Some("a browser it uses is not installed"),
            Unready::GovernorLastResort => Some("its Resource governor is at its last resort"),
            Unready::StorageCritical => Some("its storage is at the critical level"),
        })
        .collect();
    match (draining, others.is_empty()) {
        (true, true) => "Draining: it takes no new Run".to_owned(),
        (true, false) => format!("Draining: it takes no new Run; {}", others.join("; ")),
        (false, true) => "Takes no new Run".to_owned(),
        (false, false) => format!("Takes no new Run: {}", others.join("; ")),
    }
}

fn product_name(product: Product) -> &'static str {
    match product {
        Product::Collie => "Collie",
        Product::Foxhound => "Foxhound",
    }
}
