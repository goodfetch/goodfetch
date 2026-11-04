//! The details window (ADR 0141): the Kennel's executable run with
//! `window::ARGUMENT`, as a second process the tray starts. It exits when
//! its window closes, so a closed window costs nothing.
//!
//! - **Its own Connection.** It reads the Daemon's runtime file itself,
//!   through the Client, and connects as its own `kennel` Connection,
//!   which only observes: the tray passes it no token.
//! - **Threads (ADR 0060).** `eframe`, on glow, runs on the main thread.
//!   The window's model, `state::details`, lives in one task of the
//!   one-worker runtime: the Client's reports and the person's inputs reach
//!   it through one bounded channel, it keeps the Client's reads in step
//!   with what the view needs, and it publishes each view it paces. The UI
//!   thread only reads the latest view and offers the person's inputs, so
//!   it never waits.
//! - **Backpressure.** While the channel is full, the Client and the
//!   settings' follower wait to report, and the Client reads nothing more
//!   from its Connection, so the Daemon's queue for it falls back to
//!   `events.resync`. The UI thread never waits: an input the full channel
//!   refuses is offered again at a later frame when it is a state the
//!   window keeps (the filters, the rows on screen, the chosen Run, the
//!   view), and otherwise the window says it was not taken.
//! - **Fonts.** The system's fonts are read on the tracker's blocking
//!   threads while the window opens, then handed to `egui`, which lays
//!   them out from the next frame; until then the window draws no text.
//!   Without a usable font the window logs why and closes.
//! - **Repaints.** The model asks for a repaint only when its view
//!   changed, at most once a second; the person's own input is drawn by
//!   `egui` as it comes.
//! - **Its views.** The Runs, then each view `kennel.toml` leaves on: the
//!   chosen Run's Records, the Daemon's settings and the Resource
//!   governor's readings. The model offers them, and reads only what the
//!   shown one needs.
//! - **Its settings.** It follows `kennel.toml` itself, as the tray does:
//!   `controls` off, or settings not read yet, offer no Add Seeds.
//! - **The Kennel settings view** edits `kennel.toml`: the model task makes
//!   each edit on the tracker's blocking threads, then reads the file
//!   again; a change made in the file comes through the settings' watch.
//! - **A hidden icon.** Opened by the hidden-icon check, the window shows a
//!   note asking the person to free space in the menu bar.
//! - **Add Seeds** asks the platform's own confirmation, naming the Run and
//!   the consequence, on the UI thread, then hands the person's yes to the
//!   model, which makes the call only while the Run still offers it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use goodfetch_kennel::client::{self, CONNECTION_LOST, Call, CallFailure, Report, Wanted};
use goodfetch_kennel::home::Home;
use goodfetch_kennel::reports::Reports;
use goodfetch_kennel::settings::{self, Edit, Settings, SettingsError};
use goodfetch_kennel::state::details::{
    Details, DetailsView, Field, Filter, GovernorPanel, Input, KennelSettingsView, Page,
    PeriodLine, RecordsPanel, Repaint, RunLine, RunPanel, SeedsConfirmation, SettingsPanel, View,
};
use goodfetch_kennel::tasks::Tasks;
use goodfetch_kennel::window::Opening;
use goodfetch_protocol::run::{Lifecycle, RunId};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::{ask, fonts};

/// The window's size when it opens, in points.
const SIZE: [f32; 2] = [1040.0, 640.0];

/// How many calls the person makes may wait for the Client to send them.
const CALLS_WAITING: usize = 4;

/// How many inputs may wait for the window's model: the Client's reports,
/// the settings' and the person's.
const INPUTS_WAITING: usize = 64;

/// How soon the UI thread offers again an input the full channel refused.
const OFFER_AGAIN: Duration = Duration::from_millis(100);

/// Why the details window ended other than by its closing.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DetailsError {
    /// `eframe` could not run the window.
    #[error("the details window failed: {0}")]
    Window(#[from] eframe::Error),
}

/// A sink of a task's reports into the model's channel, each wrapped as
/// `wrap` says: the task waits while the channel is full.
struct ToModelSink<F> {
    to_model: mpsc::Sender<ToModel>,
    wrap: F,
}

impl<T, F> Reports<T> for ToModelSink<F>
where
    T: Send + 'static,
    F: Fn(T) -> ToModel + Send + Sync + 'static,
{
    async fn report(&self, value: T) {
        // The model has stopped: the window is closing.
        let _ = self.to_model.send((self.wrap)(value)).await;
    }
}

/// What reaches the window's model.
#[derive(Debug)]
enum ToModel {
    /// What the Client reports.
    Client(Report),
    /// What the person did.
    Person(Input),
    /// The person said yes to Add Seeds.
    Confirmed(SeedsConfirmation),
    /// The person changed one of the Kennel's settings.
    Edit(Edit),
}

/// Runs the details window, opened as `opening` says, until the person
/// closes it.
pub(crate) fn run(home: Home, opening: Opening, tasks: &Tasks) -> Result<(), DetailsError> {
    let ctx = egui::Context::default();
    let (fonts_read, fonts) = watch::channel(false);
    tasks.spawn(read_fonts(tasks.clone(), ctx.clone(), fonts_read));

    let (to_model, inputs) = mpsc::channel(INPUTS_WAITING);
    let home_for_settings = home.clone();
    let settings_file = home.settings_file();
    let (wanted, wanted_now) = watch::channel(Wanted::default());
    let (views, latest) = watch::channel(None);
    // The window never asks the Client to look again and only observes:
    // the senders live as long as the window.
    let (_look_again, looks) = mpsc::channel(1);
    let (_keep_alive, keep_daemon_alive) = watch::channel(false);
    let (calls_made, calls) = mpsc::channel(CALLS_WAITING);
    let reports = ToModelSink {
        to_model: to_model.clone(),
        wrap: ToModel::Client,
    };
    tasks.spawn(client::follow(
        tasks.clone(),
        client::Follow {
            home,
            look_again: looks,
            keep_daemon_alive,
            health_every: client::HEALTH_EVERY,
            wanted: wanted_now,
            calls,
        },
        reports,
    ));
    tasks.spawn(settings::follow(
        tasks.clone(),
        home_for_settings,
        ToModelSink {
            to_model: to_model.clone(),
            // An unreadable file gives the defaults, and the Kennel
            // settings view says why.
            wrap: |settings: Result<Settings, SettingsError>| {
                ToModel::Person(Input::Settings(settings))
            },
        },
    ));
    tasks.spawn(model(
        Model {
            wanted,
            views,
            calls: calls_made,
            ctx: ctx.clone(),
            tasks: tasks.clone(),
            settings_file,
        },
        inputs,
    ));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kennel")
            .with_inner_size(SIZE)
            .with_min_inner_size([640.0, 360.0])
            .with_active(true),
        // The system's fonts are set above; no fallback provider is built.
        system_font_fallback: false,
        ..eframe::NativeOptions::default()
    };
    let window = Window {
        note: opening.note(),
        period: None,
        to_model,
        latest,
        fonts,
        refused: false,
        filter: Filter::default(),
        offered_filter: Filter::default(),
        chosen: None,
        on_screen: BTreeSet::new(),
        view: View::Runs,
        seeds_label: String::new(),
        seeds_urls: String::new(),
        seeds_note: None,
        settings_filter: String::new(),
        closed: false,
    };
    eframe::run_native_ext(
        "Kennel",
        options,
        Some(ctx),
        Box::new(|_| Ok(Box::new(window))),
    )?;
    Ok(())
}

/// Reads the system's fonts on the tracker's blocking threads and hands
/// them to `ctx`, which lays them out from its next frame; says so through
/// `read`. Without a usable font, logs why and closes the window.
async fn read_fonts(tasks: Tasks, ctx: egui::Context, read: watch::Sender<bool>) {
    if let Some(fonts) = tasks.blocking(fonts::system).await {
        ctx.set_fonts(fonts);
        read.send_replace(true);
    } else {
        tracing::error!(
            "no usable system font was found for the details window, so it closes: install DejaVu, Noto or Liberation fonts"
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    ctx.request_repaint();
}

/// What the window's model task keeps in step.
struct Model {
    /// The reads the view needs, which the Client follows.
    wanted: watch::Sender<Wanted>,
    /// The latest view, which the UI thread draws.
    views: watch::Sender<Option<DetailsView>>,
    /// The calls the person confirmed, which the Client makes.
    calls: mpsc::Sender<Call>,
    ctx: egui::Context,
    tasks: Tasks,
    /// The settings file, which the person's edits change.
    settings_file: PathBuf,
}

/// The window's model: applies each input, makes the person's edits of
/// the Kennel's settings, keeps the Client's reads in
/// step with the view, makes the calls the person confirmed, and
/// publishes the views it paces, asking `ctx` to repaint them.
async fn model(model: Model, mut inputs: mpsc::Receiver<ToModel>) {
    let Model {
        wanted,
        views,
        calls,
        ctx,
        tasks,
        settings_file,
    } = model;
    let stop: CancellationToken = tasks.stopping();
    let mut details = Details::new();
    loop {
        let now_wanted = details.wanted();
        wanted.send_if_modified(|kept| {
            let changed = *kept != now_wanted;
            *kept = now_wanted;
            changed
        });
        let due = match details.repaint(now()) {
            Repaint::Now(view) => {
                views.send_replace(Some(*view));
                ctx.request_repaint();
                None
            }
            Repaint::At(due) => Some(tokio::time::Instant::from_std(due)),
            Repaint::Nothing => None,
        };
        tokio::select! {
            () = stop.cancelled() => return,
            input = inputs.recv() => match input {
                None => return,
                Some(ToModel::Person(input)) => details.apply(input),
                Some(ToModel::Client(report)) => apply(&mut details, report),
                Some(ToModel::Confirmed(confirmation)) => {
                    if let Some(call) = details.confirmed(confirmation)
                        && let Err(unsent) = calls.try_send(call)
                    {
                        // Full or closed: the Client is stuck or gone.
                        let failure = CallFailure {
                            code: CONNECTION_LOST.to_owned(),
                            message: "The Kennel could not send it to the Daemon".to_owned(),
                        };
                        details.apply(Input::Answered(unsent.into_inner(), Err(failure)));
                    }
                }
                Some(ToModel::Edit(edit)) => {
                    let file = settings_file.clone();
                    let edited = tasks
                        .blocking(move || {
                            let edited = settings::edit(&file, edit);
                            // The watch reports the change too; reading it
                            // here shows it without waiting for the watch.
                            edited.map(|()| settings::read(&file))
                        })
                        .await;
                    match edited {
                        Ok(read) => details.apply(Input::Settings(read)),
                        Err(error) => details.apply(Input::EditFailed(error)),
                    }
                }
            },
            () = sleep_until(due), if due.is_some() => {}
        }
    }
}

/// Sleeps until `due`; never wakes without one.
async fn sleep_until(due: Option<tokio::time::Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

/// Applies what the Client reports.
fn apply(details: &mut Details, report: Report) {
    match report {
        Report::Connection(change) => details.apply(Input::Connection(change)),
        Report::Events(batch) => {
            // Dropping `applied` once the events are applied gives their
            // credits back to the subscription.
            let (events, applied) = batch.into_parts();
            details.apply(Input::Events(events));
            drop(applied);
        }
        Report::Progress(progress) => details.apply(Input::Progress(progress)),
        Report::Listed { generation, rows } => details.apply(Input::Listed { generation, rows }),
        Report::RunReport { run_id, report } => details.apply(Input::Report { run_id, report }),
        Report::Usage { run_id, usage } => details.apply(Input::Usage { run_id, usage }),
        Report::Queried { generation, result } => {
            details.apply(Input::Queried { generation, result });
        }
        Report::Explained { generation, result } => {
            details.apply(Input::Explained { generation, result });
        }
        Report::SystemInfo(reading) => details.apply(Input::SystemInfo(reading)),
        Report::SeedsAdded { run_id, added } => details.apply(Input::SeedsAdded { run_id, added }),
        Report::Answered(call, answer) => details.apply(Input::Answered(call, answer)),
        // The window never asks for the tray's live Runs.
        Report::LiveRuns(_) => {}
    }
}

/// The instant the model paces its repaints by.
fn now() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the window's one clock read: its model takes the instant it paces repaints by, and reads no clock itself (ADR 0059)"
    )]
    Instant::now()
}

/// The window on the UI thread: it draws the latest view and sends what
/// the person does to the model.
struct Window {
    /// Why the window opened, when it says so, as for a hidden icon.
    note: Option<String>,
    /// The coalescing period as the person types it, until they leave the
    /// field.
    period: Option<u32>,
    to_model: mpsc::Sender<ToModel>,
    latest: watch::Receiver<Option<DetailsView>>,
    /// Whether the system's fonts are in `egui`: until then the window
    /// draws no text.
    fonts: watch::Receiver<bool>,
    /// Whether the full channel refused the person's last click, which the
    /// window then says.
    refused: bool,
    /// The filters as the person set them.
    filter: Filter,
    /// The filters the model took.
    offered_filter: Filter,
    /// The Run the person chose.
    chosen: Option<RunId>,
    /// The table's rows on screen at the last frame.
    on_screen: BTreeSet<RunId>,
    /// The view the person chose.
    view: View,
    /// Add Seeds' Label, as the person typed it.
    seeds_label: String,
    /// Add Seeds' URLs, one a line, as the person typed them.
    seeds_urls: String,
    /// What Add Seeds still needs from the person.
    seeds_note: Option<String>,
    /// The Daemon settings view's filter on keys.
    settings_filter: String,
    closed: bool,
}

impl Window {
    /// Offers `input` to the model; whether it took it. A full channel
    /// refuses it, and the UI thread never waits: the caller offers it
    /// again at a later frame, or says it was not taken.
    fn offer(&mut self, ctx: &egui::Context, input: ToModel) -> bool {
        match self.to_model.try_send(input) {
            Ok(()) => {
                self.refused = false;
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                ctx.request_repaint_after(OFFER_AGAIN);
                false
            }
            // The model has stopped: the window is closing.
            Err(mpsc::error::TrySendError::Closed(_)) => true,
        }
    }

    /// Offers the person's `input`; whether the model took it.
    fn send(&mut self, ctx: &egui::Context, input: Input) -> bool {
        self.offer(ctx, ToModel::Person(input))
    }

    /// Offers the person's click `input`, saying so in the window when the
    /// full channel refuses it.
    fn click(&mut self, ctx: &egui::Context, input: ToModel) {
        if !self.offer(ctx, input) {
            self.refused = true;
        }
    }

    /// The Daemon's line, its health and the views the window offers.
    fn header(&mut self, ui: &mut egui::Ui, view: &DetailsView) {
        ui.horizontal(|ui| {
            ui.strong(&view.daemon);
            if let Some(health) = &view.health {
                ui.label(health);
            }
        });
        if view.views.len() > 1 {
            ui.horizontal(|ui| {
                for offered in &view.views {
                    if ui
                        .selectable_label(self.view == *offered, offered.label())
                        .clicked()
                        && self.view != *offered
                    {
                        if self.send(ui.ctx(), Input::Show(*offered)) {
                            self.view = *offered;
                        } else {
                            self.refused = true;
                        }
                    }
                }
            });
        }
    }

    /// The Runs table's filters.
    fn filters(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter.text)
                    .hint_text("Run ID, Recipe or tag")
                    .desired_width(240.0),
            );
            egui::ComboBox::from_label("Lifecycle")
                .selected_text(lifecycle_choice(self.filter.lifecycle))
                .show_ui(ui, |ui| {
                    for choice in [
                        None,
                        Some(Lifecycle::Running),
                        Some(Lifecycle::Stopped),
                        Some(Lifecycle::Finished),
                        Some(Lifecycle::Failed),
                    ] {
                        ui.selectable_value(
                            &mut self.filter.lifecycle,
                            choice,
                            lifecycle_choice(choice),
                        );
                    }
                });
        });
        // A change the full channel refused is offered again at the next
        // frame.
        if self.filter != self.offered_filter
            && self.send(ui.ctx(), Input::Filter(self.filter.clone()))
        {
            self.offered_filter = self.filter.clone();
        }
    }

    /// The Runs table, which draws only the rows on screen and tells the
    /// model which they are.
    fn table(&mut self, ui: &mut egui::Ui, view: &DetailsView) {
        if let Some(note) = &view.note {
            ui.label(note);
        }
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        let mut chose = None;
        let mut on_screen = BTreeSet::new();
        egui::ScrollArea::both().auto_shrink(false).show_rows(
            ui,
            row_height,
            view.runs.len() + 1,
            |ui, rows| {
                egui::Grid::new("runs")
                    .striped(true)
                    .num_columns(COLUMNS.len())
                    .show(ui, |ui| {
                        for row in rows {
                            let Some(line) = row.checked_sub(1).and_then(|at| view.runs.get(at))
                            else {
                                for column in COLUMNS {
                                    ui.strong(column);
                                }
                                ui.end_row();
                                continue;
                            };
                            on_screen.insert(line.run_id.clone());
                            if self.line(ui, line) {
                                chose = Some(line.run_id.clone());
                            }
                            ui.end_row();
                        }
                    });
            },
        );
        // Rows the full channel refused are offered again at the next
        // frame.
        if on_screen != self.on_screen && self.send(ui.ctx(), Input::OnScreen(on_screen.clone())) {
            self.on_screen = on_screen;
        }
        if let Some(run) = chose {
            // A second click on the chosen Run puts it away.
            let chosen = (self.chosen.as_ref() != Some(&run)).then_some(run);
            if self.send(ui.ctx(), Input::Chose(chosen.clone())) {
                self.chosen = chosen;
            } else {
                self.refused = true;
            }
        }
    }

    /// One row of the table; whether the person clicked it.
    fn line(&self, ui: &mut egui::Ui, line: &RunLine) -> bool {
        let chosen = self.chosen.as_ref() == Some(&line.run_id);
        let clicked = ui.selectable_label(chosen, &line.name).clicked();
        for value in [
            &line.lifecycle,
            &line.verdict,
            &line.completeness,
            &line.started,
            &line.ended,
            &line.progress,
        ] {
            ui.label(value);
        }
        clicked
    }
}

/// The table's columns.
const COLUMNS: [&str; 7] = [
    "Run",
    "Lifecycle",
    "Verdict",
    "Completeness",
    "Started",
    "Ended",
    "Progress",
];

impl Window {
    /// The chosen Run's progress, Usage, failures by Error code and Run
    /// report, and Add Seeds when it offers it.
    fn panel(&mut self, ui: &mut egui::Ui, panel: &RunPanel) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(&panel.title);
            section(ui, "Progress", &panel.progress);
            section(ui, "Usage", &panel.usage);
            if !panel.failures.is_empty() {
                section(ui, "Failures by Error code", &panel.failures);
            }
            section(ui, "Run report", &panel.report);
            if panel.add_seeds.is_some() || panel.seeds.is_some() {
                ui.add_space(8.0);
                ui.strong("Add seeds");
                self.add_seeds(ui, panel);
            }
        });
    }

    /// Add Seeds: a Label and URLs, one a line, added once the person
    /// confirms; then what the crawl did with them.
    fn add_seeds(&mut self, ui: &mut egui::Ui, panel: &RunPanel) {
        if let Some(offer) = &panel.add_seeds {
            ui.horizontal(|ui| {
                ui.label("Label");
                ui.add(
                    egui::TextEdit::singleline(&mut self.seeds_label)
                        .hint_text("the handler's Label")
                        .desired_width(200.0),
                );
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.seeds_urls)
                    .hint_text("URLs, one a line")
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            if ui.button("Add seeds…").clicked() {
                match offer.confirmation(&self.seeds_label, &self.seeds_urls) {
                    Err(missing) => self.seeds_note = Some(missing),
                    Ok(confirmation) => {
                        self.seeds_note = None;
                        if ask::asked(
                            &confirmation.title,
                            &confirmation.consequence,
                            &confirmation.confirm,
                        ) {
                            if self.offer(ui.ctx(), ToModel::Confirmed(confirmation)) {
                                self.seeds_urls.clear();
                            } else {
                                self.seeds_note = Some(
                                    "The seeds were not added: the window is catching up. Try again."
                                        .to_owned(),
                                );
                            }
                        }
                    }
                }
            }
            if let Some(missing) = &self.seeds_note {
                ui.weak(missing.as_str());
            }
        }
        if let Some(seeds) = &panel.seeds {
            ui.add(egui::Label::new(seeds.as_str()).wrap());
        }
    }

    /// The Records view: a page of the chosen Run's Records.
    fn records(&mut self, ui: &mut egui::Ui, panel: &RecordsPanel) {
        ui.horizontal(|ui| {
            ui.heading(&panel.title);
            if ui.button("Refresh").clicked() {
                self.click(ui.ctx(), ToModel::Person(Input::Refresh));
            }
        });
        if let Some(note) = &panel.note {
            ui.label(note);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(panel.previous, egui::Button::new("Previous"))
                .clicked()
            {
                self.click(
                    ui.ctx(),
                    ToModel::Person(Input::RecordsPage(Page::Previous)),
                );
            }
            ui.label(&panel.range);
            if ui
                .add_enabled(panel.next, egui::Button::new("Next"))
                .clicked()
            {
                self.click(ui.ctx(), ToModel::Person(Input::RecordsPage(Page::Next)));
            }
        });
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("records")
                .striped(true)
                .num_columns(panel.columns.len())
                .show(ui, |ui| {
                    for column in &panel.columns {
                        ui.strong(column);
                    }
                    ui.end_row();
                    for row in &panel.rows {
                        for value in row {
                            ui.add(egui::Label::new(value.as_str()).truncate());
                        }
                        ui.end_row();
                    }
                });
        });
    }

    /// The Daemon settings view, filtered on keys by the person's text.
    fn daemon_settings(&mut self, ui: &mut egui::Ui, panel: &SettingsPanel) {
        ui.horizontal(|ui| {
            ui.heading("Daemon settings");
            if ui.button("Refresh").clicked() {
                self.click(ui.ctx(), ToModel::Person(Input::Refresh));
            }
        });
        if let Some(note) = &panel.note {
            ui.label(note);
        }
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            if !panel.sources.is_empty() {
                section(ui, "Sources", &panel.sources);
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_filter)
                        .hint_text("Key")
                        .desired_width(240.0),
                );
            });
            let filter = self.settings_filter.trim().to_lowercase();
            egui::Grid::new("daemon settings")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    for column in ["Key", "Value", "Set by", "Locked by"] {
                        ui.strong(column);
                    }
                    ui.end_row();
                    for line in panel
                        .settings
                        .iter()
                        .filter(|line| line.key.to_lowercase().contains(&filter))
                    {
                        for value in [&line.key, &line.value, &line.set_by, &line.locked_by] {
                            ui.label(value.as_str());
                        }
                        ui.end_row();
                    }
                });
        });
    }
}

/// The Resource governor's view.
fn governor(ui: &mut egui::Ui, panel: &GovernorPanel) {
    ui.heading("Resource governor");
    if let Some(note) = &panel.note {
        ui.label(note);
    }
    if !panel.readings.is_empty() {
        egui::Grid::new("governor")
            .striped(true)
            .num_columns(2)
            .show(ui, |ui| {
                for field in &panel.readings {
                    ui.label(&field.label);
                    ui.label(&field.value);
                    ui.end_row();
                }
            });
    }
}

/// A titled list of fields.
fn section(ui: &mut egui::Ui, title: &str, fields: &[Field]) {
    ui.add_space(8.0);
    ui.strong(title);
    if fields.is_empty() {
        ui.weak("Nothing yet");
        return;
    }
    egui::Grid::new(title).num_columns(2).show(ui, |ui| {
        for field in fields {
            ui.label(&field.label);
            ui.add(egui::Label::new(&field.value).wrap());
            ui.end_row();
        }
    });
}

impl Window {
    /// The Kennel settings view: a switch for each setting and the
    /// coalescing period, each change an edit of the file.
    fn kennel_settings(&mut self, ui: &mut egui::Ui, settings: &KennelSettingsView) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(note) = &settings.note {
                ui.add(egui::Label::new(egui::RichText::new(note).strong()).wrap());
            }
            ui.weak(&settings.about);
            for section in &settings.sections {
                ui.add_space(8.0);
                ui.strong(&section.title);
                for line in &section.switches {
                    let mut on = line.on;
                    if ui.checkbox(&mut on, &line.label).changed() {
                        self.click(ui.ctx(), ToModel::Edit(Edit::Switch(line.switch, on)));
                    }
                }
                if let Some(period) = &section.period {
                    self.period(ui, period);
                }
            }
        });
    }

    /// The coalescing period's field, which edits the file once the person
    /// leaves it or stops dragging.
    fn period(&mut self, ui: &mut egui::Ui, period: &PeriodLine) {
        let saved = period.minutes;
        ui.horizontal(|ui| {
            ui.label(&period.label);
            let mut minutes = self.period.unwrap_or(saved);
            let field = ui.add(egui::DragValue::new(&mut minutes).range(0..=1440));
            if field.changed() {
                self.period = Some(minutes);
            }
            if (field.drag_stopped() || field.lost_focus())
                && let Some(minutes) = self.period.take()
                && minutes != saved
            {
                self.click(ui.ctx(), ToModel::Edit(Edit::Period(minutes)));
            }
        });
    }
}

/// What the Lifecycle filter shows for `lifecycle`.
fn lifecycle_choice(lifecycle: Option<Lifecycle>) -> &'static str {
    match lifecycle {
        None => "Any",
        Some(Lifecycle::Running) => "Running",
        Some(Lifecycle::Stopped) => "Stopped",
        Some(Lifecycle::Finished) => "Finished",
        Some(Lifecycle::Failed) => "Failed",
    }
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.closed && ui.ctx().input(|input| input.viewport().close_requested()) {
            // The process exits once the window closes; until then the
            // model reads nothing more. A full channel refuses it only
            // while the process ends anyway.
            self.closed = true;
            self.send(ui.ctx(), Input::Closed);
        }
        if !*self.fonts.borrow() {
            // No font to lay text out with yet.
            return;
        }
        if self.refused {
            egui::Panel::bottom("refused").show(ui, |ui| {
                ui.weak("The window is catching up: that change was not taken. Try again.");
            });
        }
        if let Some(note) = &self.note {
            egui::Panel::top("note").show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(note).strong()).wrap());
            });
        }
        let view = self.latest.borrow().clone();
        let Some(view) = view else {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("Looking for the Daemon…");
            });
            return;
        };
        // A view the window no longer offers gives way to the Runs, as
        // the model shows them.
        if !view.views.contains(&self.view) {
            self.view = View::Runs;
        }
        egui::Panel::top("daemon").show(ui, |ui| self.header(ui, &view));
        match self.view {
            View::KennelSettings => {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(settings) = &view.kennel_settings {
                        self.kennel_settings(ui, settings);
                    }
                });
            }
            View::Runs => {
                egui::Panel::top("filters").show(ui, |ui| self.filters(ui));
                if let Some(chosen) = &view.chosen {
                    egui::Panel::right("run")
                        .resizable(true)
                        .default_size(400.0)
                        .show(ui, |ui| self.panel(ui, chosen));
                }
                egui::CentralPanel::default().show(ui, |ui| self.table(ui, &view));
            }
            View::Records => {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(records) = &view.records {
                        self.records(ui, records);
                    }
                });
            }
            View::DaemonSettings => {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(settings) = &view.daemon_settings {
                        self.daemon_settings(ui, settings);
                    }
                });
            }
            View::Governor => {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(readings) = &view.governor {
                        governor(ui, readings);
                    }
                });
            }
        }
    }
}
