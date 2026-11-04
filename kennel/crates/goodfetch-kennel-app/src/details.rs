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
//!   it through a channel, it keeps the Client's reads in step with what
//!   the view needs, and it publishes each view it paces. The UI thread
//!   only reads the latest view and sends the person's inputs, so it never
//!   waits.
//! - **Repaints.** The model asks for a repaint only when its view
//!   changed, at most once a second; the person's own input is drawn by
//!   `egui` as it comes.

use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::Context as _;
use eframe::egui;
use goodfetch_kennel::client::{self, Report, Wanted};
use goodfetch_kennel::home::Home;
use goodfetch_kennel::state::details::{
    Details, DetailsView, Field, Filter, Input, Repaint, RunLine, RunPanel,
};
use goodfetch_kennel::tasks::Tasks;
use goodfetch_kennel::window::Opening;
use goodfetch_protocol::run::{Lifecycle, RunId};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::fonts;

/// The window's size when it opens, in points.
const SIZE: [f32; 2] = [1040.0, 640.0];

/// What reaches the window's model.
#[derive(Debug)]
enum ToModel {
    /// What the Client reports.
    Client(Report),
    /// What the person did.
    Person(Input),
}

/// Runs the details window, opened as `opening` says, until the person
/// closes it.
pub(crate) fn run(
    home: Home,
    opening: Opening,
    tasks: &Tasks,
    runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    let fonts = runtime
        .block_on(tasks.blocking(fonts::system))
        .context("no system font was found for the details window")?;
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);

    let (to_model, inputs) = mpsc::unbounded_channel();
    let (wanted, wanted_now) = watch::channel(Wanted::default());
    let (views, latest) = watch::channel(None);
    // The window never asks the Client to look again, only observes, and
    // makes no call: the senders live as long as the window.
    let (_look_again, looks) = mpsc::channel(1);
    let (_keep_alive, keep_daemon_alive) = watch::channel(false);
    let (_calls, calls) = mpsc::channel(1);
    let reports = to_model.clone();
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
        move |report: Report| {
            // The model has stopped: the window is closing.
            let _ = reports.send(ToModel::Client(report));
        },
    ));
    tasks.spawn(model(inputs, wanted, views, ctx.clone(), tasks.stopping()));

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
        to_model,
        latest,
        filter: Filter::default(),
        chosen: None,
        on_screen: BTreeSet::new(),
        closed: false,
    };
    eframe::run_native_ext(
        "Kennel",
        options,
        Some(ctx),
        Box::new(|_| Ok(Box::new(window))),
    )
    .map_err(|error| anyhow::anyhow!("the details window failed: {error}"))
}

/// The window's model: applies each input, keeps the Client's reads in
/// step with the view, and publishes the views it paces, asking `ctx` to
/// repaint them.
async fn model(
    mut inputs: mpsc::UnboundedReceiver<ToModel>,
    wanted: watch::Sender<Wanted>,
    views: watch::Sender<Option<DetailsView>>,
    ctx: egui::Context,
    stop: CancellationToken,
) {
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
        // The window never asks for the tray's live Runs, and makes no
        // call.
        Report::LiveRuns(_) | Report::Answered(..) => {}
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
    to_model: mpsc::UnboundedSender<ToModel>,
    latest: watch::Receiver<Option<DetailsView>>,
    /// The filters as the person set them.
    filter: Filter,
    /// The Run the person chose.
    chosen: Option<RunId>,
    /// The table's rows on screen at the last frame.
    on_screen: BTreeSet<RunId>,
    closed: bool,
}

impl Window {
    fn send(&self, input: Input) {
        // The model has stopped: the window is closing.
        let _ = self.to_model.send(ToModel::Person(input));
    }

    /// The Daemon's line, its health and the table's filters.
    fn header(&mut self, ui: &mut egui::Ui, view: &DetailsView) {
        ui.horizontal(|ui| {
            ui.strong(&view.daemon);
            if let Some(health) = &view.health {
                ui.label(health);
            }
        });
        let before = self.filter.clone();
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
        if self.filter != before {
            self.send(Input::Filter(self.filter.clone()));
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
        if on_screen != self.on_screen {
            self.on_screen.clone_from(&on_screen);
            self.send(Input::OnScreen(on_screen));
        }
        if let Some(run) = chose {
            // A second click on the chosen Run puts it away.
            self.chosen = (self.chosen.as_ref() != Some(&run)).then_some(run);
            self.send(Input::Chose(self.chosen.clone()));
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

/// The chosen Run's progress, Usage, failures by Error code and Run
/// report.
fn panel(ui: &mut egui::Ui, panel: &RunPanel) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(&panel.title);
        section(ui, "Progress", &panel.progress);
        section(ui, "Usage", &panel.usage);
        if !panel.failures.is_empty() {
            section(ui, "Failures by Error code", &panel.failures);
        }
        section(ui, "Run report", &panel.report);
    });
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
            // model reads nothing more.
            self.closed = true;
            self.send(Input::Closed);
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
        egui::Panel::top("daemon").show(ui, |ui| self.header(ui, &view));
        if let Some(chosen) = &view.chosen {
            egui::Panel::right("run")
                .resizable(true)
                .default_size(400.0)
                .show(ui, |ui| panel(ui, chosen));
        }
        egui::CentralPanel::default().show(ui, |ui| self.table(ui, &view));
    }
}
