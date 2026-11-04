//! The Kennel's executable (ADR 0141): the Tray, over the modules of
//! `goodfetch-kennel`; run with `window::ARGUMENT`, the details window, a
//! second process the Tray starts (`details`).
//!
//! A plain `fn main` builds a one-worker multi-thread Tokio runtime and
//! holds it, then runs the UI loop, `winit`'s, on the main thread (ADR
//! 0060). Every task starts through the `Tasks` tracker, and a task reaches
//! the UI only through the event loop's proxy, so the UI thread makes no
//! blocking call. On macOS the Kennel has no Dock icon (`.accessory`).
//!
//! The State paces the redraws: after each input the Tray keeps the latest
//! view, which it draws as the menu opens, and draws what the State says is
//! due, the loop waking at the instant a paced redraw falls due.
//!
//! On Linux the Tray runs GTK's main loop on a thread of its own, through
//! the `Tasks` tracker, and draws the frames this loop sends it (`tray`).
//!
//! At launch, once the settings are read, the State asks for the
//! hidden-icon check, which the Tray makes once the status item has had
//! time to settle, and makes the login agent as start at login says.
//!
//! The State also decides the desktop notices, and the Kennel shows them
//! through the platform's adapter (`notices`), which it builds only while
//! the settings turn them on, waking the loop when a held summary falls
//! due.

mod ask;
mod details;
mod fonts;
mod notices;
mod tray;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Context;
use goodfetch_kennel::client::{self, CONNECTION_LOST, Call, CallFailure, Report, Wanted};
use goodfetch_kennel::daemon_start;
use goodfetch_kennel::home::Home;
use goodfetch_kennel::login::{self, LoginAgent, SystemLaunchctl};
use goodfetch_kennel::settings::{self, Settings, SettingsError};
#[cfg(target_os = "linux")]
use goodfetch_kennel::state::{Choice, Confirmation};
use goodfetch_kennel::state::{Effect, Input, MenuChange, Redraw, State};
use goodfetch_kennel::tasks::Tasks;
use goodfetch_kennel::window::{self, Opening};
use tokio::sync::{mpsc, watch};
#[cfg(not(target_os = "linux"))]
use tray_icon::menu::MenuId;
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

/// How many calls a person makes may wait for the Client to send them.
const CALLS_WAITING: usize = 16;

/// How long the Kennel waits for its tasks once it quits.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// How long after the hidden-icon check is asked for the Tray reads the
/// status item's occlusion state: the time #1915 read it at, once the menu
/// bar had placed the item.
const ICON_SETTLES: Duration = Duration::from_secs(3);

/// What reaches the UI loop from elsewhere.
#[derive(Debug)]
pub(crate) enum UserEvent {
    /// An input for the State, from a task.
    Input(Input),
    /// What the Client reports.
    Client(Report),
    /// The person chose a menu item.
    #[cfg(not(target_os = "linux"))]
    Menu(MenuId),
    /// The person chose this, as the Tray's GTK thread found it.
    #[cfg(target_os = "linux")]
    Chose(Choice),
    /// The person confirmed this, in the Tray's GTK dialog.
    #[cfg(target_os = "linux")]
    Confirmed(Confirmation),
    /// The menu opened, drawing the latest view.
    #[cfg(not(target_os = "linux"))]
    MenuOpened,
    /// The menu closed.
    #[cfg(target_os = "macos")]
    MenuClosed,
    /// The details window's process started, with this process ID.
    DetailsOpened(u32),
    /// The hidden-icon check falls due.
    CheckIcon,
    /// The icon is in the panel, or could not be put there: the Linux
    /// Tray's GTK thread says so once it started.
    #[cfg(target_os = "linux")]
    Installed(Result<(), tray::InstallError>),
}

fn main() -> anyhow::Result<()> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the tracing subscriber's writer is one of STYLE.md's three writers to stderr"
    )]
    let stderr = std::io::stderr;
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(stderr)
        .init();
    #[expect(
        clippy::disallowed_methods,
        reason = "only main reads the environment (ADR 0059)"
    )]
    let home_variable = std::env::var_os("GOODFETCH_HOME");
    let home = Home::resolve(home_variable)
        .context("the OS names no data folder for the Goodfetch home: set GOODFETCH_HOME")?;
    #[expect(
        clippy::disallowed_methods,
        reason = "main builds the runtime (ADRs 0060 and 0141)"
    )]
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .context("building the runtime")?;

    if let Some(opening) = Opening::from_arguments(std::env::args_os().skip(1)) {
        let tasks = Tasks::new(runtime.handle().clone());
        let ran = details::run(home, opening, &tasks);
        runtime.block_on(tasks.stop(STOP_GRACE));
        runtime.shutdown_timeout(STOP_GRACE);
        // `eframe`'s error holds platform handles, which are not `Send`.
        return ran.map_err(|error| anyhow::anyhow!("{error}"));
    }

    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_default_menu(false);
    }
    let event_loop = builder.build().context("building the event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    tray::listen(proxy.clone());

    let tasks = Tasks::new(runtime.handle().clone());
    let (look_again, looks) = mpsc::channel(1);
    let (wanted, wanted_now) = watch::channel(Wanted::default());
    let (calls, calls_made) = mpsc::channel(CALLS_WAITING);
    let (start_at_login, login_wanted) = watch::channel(None);
    start_tasks(&tasks, &home, looks, wanted_now, calls_made, &proxy);
    tasks.spawn(follow_start_at_login(tasks.clone(), login_wanted));
    let mut kennel = Kennel {
        state: State::new(),
        tasks: tasks.clone(),
        proxy,
        look_again,
        wanted,
        calls,
        start_at_login,
        installed: false,
        next_draw: None,
        details: None,
        notifier: Notifier::Off,
        next_notice: None,
    };
    let ran = event_loop.run_app(&mut kennel);
    runtime.block_on(tasks.stop(STOP_GRACE));
    runtime.shutdown_timeout(STOP_GRACE);
    ran.context("running the event loop")
}

/// Starts the Client, the settings' follower and the lookup of the
/// installed Product, each reporting to the UI loop.
fn start_tasks(
    tasks: &Tasks,
    home: &Home,
    looks: mpsc::Receiver<()>,
    wanted: watch::Receiver<Wanted>,
    calls: mpsc::Receiver<Call>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    let to_ui = |proxy: &EventLoopProxy<UserEvent>| {
        let proxy = proxy.clone();
        move |event: UserEvent| {
            // The loop has stopped: the Kennel is quitting.
            let _ = proxy.send_event(event);
        }
    };
    let (keep_alive, keep_daemon_alive) = watch::channel(Settings::default().keep_daemon_alive);
    let report = to_ui(proxy);
    tasks.spawn(client::follow(
        tasks.clone(),
        client::Follow {
            home: home.clone(),
            look_again: looks,
            keep_daemon_alive,
            health_every: client::HEALTH_EVERY,
            wanted,
            calls,
        },
        move |client: Report| report(UserEvent::Client(client)),
    ));
    let report = to_ui(proxy);
    tasks.spawn(settings::follow(
        tasks.clone(),
        home.clone(),
        move |settings: Result<Settings, SettingsError>| {
            // An unreadable file gives the defaults, as the State shows.
            let wanted = settings
                .as_ref()
                .map_or(Settings::default().keep_daemon_alive, |settings| {
                    settings.keep_daemon_alive
                });
            keep_alive.send_if_modified(|kept| std::mem::replace(kept, wanted) != wanted);
            report(UserEvent::Input(Input::Settings(settings)));
        },
    ));
    let report = to_ui(proxy);
    let lookup = home.clone();
    let blocking = tasks.clone();
    tasks.spawn(async move {
        let installed = blocking.blocking(move || daemon_start::find(&lookup)).await;
        report(UserEvent::Input(Input::Installed(installed)));
    });
}

/// The UI loop's application.
struct Kennel {
    state: State,
    tasks: Tasks,
    proxy: EventLoopProxy<UserEvent>,
    /// Asks the Client to look for the Daemon again.
    look_again: mpsc::Sender<()>,
    /// The reads the State wants the Client to keep in step with.
    wanted: watch::Sender<Wanted>,
    /// The calls a person makes, which the Client sends.
    calls: mpsc::Sender<Call>,
    /// Start at login as the State last asked it made.
    start_at_login: watch::Sender<Option<bool>>,
    /// Whether the icon is in the menu bar.
    installed: bool,
    /// When a paced redraw falls due.
    next_draw: Option<Instant>,
    /// The details window's process ID, while it runs.
    details: Option<u32>,
    /// The desktop notices' adapter, while the settings turn them on.
    notifier: Notifier,
    /// When the summary of the held desktop notices falls due.
    next_notice: Option<Instant>,
}

/// The desktop notices' adapter, as the settings and the platform allow.
enum Notifier {
    /// The settings turn desktop notices off, or are not read yet: no
    /// notice code runs.
    Off,
    /// Built, and showing them.
    On(Box<dyn notices::Adapter>),
    /// On, but the platform cannot show them here, as the log said once.
    Unavailable,
}

impl Kennel {
    /// Carries out the State's `effects`, then keeps the Client's reads
    /// and the Tray in step with the State.
    fn carry_out(&mut self, event_loop: &ActiveEventLoop, effects: Vec<Effect>) {
        let mut effects = VecDeque::from(effects);
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::Quit => event_loop.exit(),
                Effect::OpenDetails(opening) => self.open_details(opening),
                Effect::CheckIcon => {
                    let proxy = self.proxy.clone();
                    self.tasks.spawn(async move {
                        tokio::time::sleep(ICON_SETTLES).await;
                        let _ = proxy.send_event(UserEvent::CheckIcon);
                    });
                }
                Effect::StartAtLogin(on) => {
                    self.start_at_login.send_replace(Some(on));
                }
                Effect::RaiseDetails => {
                    if let Some(pid) = self.details {
                        tray::raise(pid);
                    } else {
                        tracing::info!("the details window is starting");
                    }
                }
                Effect::Call(call) => {
                    if let Err(unsent) = self.calls.try_send(call) {
                        // Full or closed: the Client is stuck or gone, and
                        // the State hears that the call was not sent.
                        let failure = CallFailure {
                            code: CONNECTION_LOST.to_owned(),
                            message: "The Kennel could not send it to the Daemon".to_owned(),
                        };
                        let call = unsent.into_inner();
                        effects.extend(self.state.apply(Input::Answered(call, Err(failure))));
                    }
                }
                // On Linux the GTK thread asks, and the answer comes back
                // as `UserEvent::Confirmed`.
                #[cfg(target_os = "linux")]
                Effect::Confirm(confirmation) => tray::confirm(confirmation),
                #[cfg(not(target_os = "linux"))]
                Effect::Confirm(confirmation) => {
                    if ask::confirmed(&confirmation) {
                        effects.extend(self.state.confirmed(confirmation));
                    }
                }
                Effect::Copy(text) => ask::copy(&text),
                Effect::StartDaemon(product) => {
                    tracing::info!(path = %product.path.display(), "starting the Daemon");
                    let proxy = self.proxy.clone();
                    let look_again = self.look_again.clone();
                    self.tasks.spawn(async move {
                        let ended = daemon_start::start(&product).await;
                        if let Err(failure) = &ended {
                            tracing::warn!(%failure, "the Daemon did not start");
                        }
                        let _ = proxy.send_event(UserEvent::Input(Input::StartEnded(ended)));
                        // Full: the Client already looks again.
                        let _ = look_again.try_send(());
                    });
                }
            }
        }
        let wanted = self.state.wanted();
        self.wanted.send_if_modified(|now| {
            let changed = *now != wanted;
            *now = wanted;
            changed
        });
        if self.installed {
            // Linux draws no menu on a click: nothing to keep.
            #[cfg(not(target_os = "linux"))]
            tray::keep(self.state.view());
            self.redraw();
        }
        self.notify();
    }

    /// Shows the desktop notices the State says are due now, building the
    /// adapter when the settings turn them on and dropping it when they
    /// turn them off, or wakes the loop when held ones fall due.
    fn notify(&mut self) {
        match (self.state.notifying(), &self.notifier) {
            (false, _) => self.notifier = Notifier::Off,
            (true, Notifier::Off) => {
                self.notifier = match notices::adapter(&self.tasks) {
                    Some(adapter) => Notifier::On(adapter),
                    None => Notifier::Unavailable,
                };
            }
            (true, Notifier::On(_) | Notifier::Unavailable) => {}
        }
        let due = self.state.notices(now());
        if let Notifier::On(adapter) = &self.notifier {
            for notice in due.send {
                adapter.show(notice);
            }
        }
        self.next_notice = due.due;
    }

    /// Starts the details window's process from this executable, with no
    /// more than its arguments, `opening`'s: it reads the runtime file and
    /// connects itself. The State learns when it ends.
    fn open_details(&self, opening: Opening) {
        let proxy = self.proxy.clone();
        let executable = std::env::current_exe();
        self.tasks.spawn(async move {
            let opened = proxy.clone();
            let ran = match executable {
                Ok(executable) => {
                    window::run(&executable, opening, move |pid| {
                        let _ = opened.send_event(UserEvent::DetailsOpened(pid));
                    })
                    .await
                }
                Err(error) => Err(error),
            };
            if let Err(error) = ran {
                tracing::warn!(%error, "the details window did not start");
            }
            let _ = proxy.send_event(UserEvent::Input(Input::DetailsClosed));
        });
    }

    /// The icon is in the menu bar, or could not be put there, which ends
    /// the Kennel.
    fn installed(
        &mut self,
        event_loop: &ActiveEventLoop,
        installed: Result<(), tray::InstallError>,
    ) {
        if let Err(error) = installed {
            tracing::error!(%error, "the icon could not be put in the menu bar");
            event_loop.exit();
            return;
        }
        self.installed = true;
        if !tray::FOLLOWS_THE_MENU {
            // Nothing says when the menu opens or closes here, so it
            // counts as open from launch.
            let effects = self.state.apply(Input::Menu(MenuChange::Opened(now())));
            self.carry_out(event_loop, effects);
        }
        self.redraw();
    }

    /// Draws what the State says is due now, or wakes the loop when it is.
    fn redraw(&mut self) {
        self.next_draw = match self.state.redraw(now()) {
            Redraw::Now(frame) => {
                tray::draw(frame);
                None
            }
            Redraw::At(due) => Some(due),
            Redraw::Nothing => None,
        };
    }
}

/// Makes the login agent as each change of `wanted` says, one change at a
/// time, so the latest wins.
async fn follow_start_at_login(tasks: Tasks, mut wanted: watch::Receiver<Option<bool>>) {
    while wanted.changed().await.is_ok() {
        let on = *wanted.borrow_and_update();
        if let Some(on) = on {
            tasks.blocking(move || start_at_login(on)).await;
        }
    }
}

/// Makes the login agent as start at login says (`on`): an agent that
/// opens the `.app` the Kennel runs from, or none. Blocks.
fn start_at_login(on: bool) {
    if !cfg!(target_os = "macos") {
        if on {
            tracing::warn!("start at login is made on macOS only for now");
        }
        return;
    }
    let Some(agent) = LoginAgent::of_the_person() else {
        tracing::warn!("the OS names no home folder, so start at login is not made");
        return;
    };
    let bundle = if on {
        let bundle = std::env::current_exe()
            .ok()
            .and_then(|executable| login::bundle_of(&executable));
        if bundle.is_none() {
            tracing::warn!(
                "start at login opens the Kennel's installed .app, and this Kennel runs outside one: install it with `just kennel`"
            );
            return;
        }
        bundle
    } else {
        None
    };
    match agent.apply(bundle.as_deref(), &SystemLaunchctl) {
        Ok(change) => tracing::info!(?change, "start at login made"),
        Err(error) => tracing::warn!(%error, "start at login could not be made"),
    }
}

/// The instant the State paces its redraws and coalesces its desktop
/// notices by.
fn now() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Tray's one clock read: the State takes the instant it paces redraws and notices by, and reads no clock itself (ADR 0059)"
    )]
    Instant::now()
}

impl ApplicationHandler<UserEvent> for Kennel {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            // On macOS the status item is made once the loop runs, not
            // before.
            // On Linux the GTK thread reports it later, as
            // `UserEvent::Installed`.
            StartCause::Init => {
                if let Some(installed) = tray::install(self.state.view(), &self.tasks) {
                    self.installed(event_loop, installed);
                }
            }
            StartCause::ResumeTimeReached { .. } => {
                self.redraw();
                self.notify();
            }
            StartCause::WaitCancelled { .. } | StartCause::Poll => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let next = match (self.next_draw, self.next_notice) {
            (Some(draw), Some(notice)) => Some(draw.min(notice)),
            (draw, notice) => draw.or(notice),
        };
        event_loop.set_control_flow(match next {
            Some(due) => ControlFlow::WaitUntil(due),
            None => ControlFlow::Wait,
        });
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window: WindowId,
        _event: WindowEvent,
    ) {
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let effects = match event {
            UserEvent::Input(input) => {
                if matches!(input, Input::DetailsClosed) {
                    self.details = None;
                }
                self.state.apply(input)
            }
            UserEvent::Client(Report::Connection(change)) => {
                self.state.apply(Input::Connection(change))
            }
            UserEvent::Client(Report::Events(batch)) => {
                // Dropping `applied` once the events are applied gives
                // their credits back to the subscription.
                let (events, applied) = batch.into_parts();
                let effects = self.state.apply(Input::Events(events));
                drop(applied);
                effects
            }
            UserEvent::Client(Report::LiveRuns(rows)) => self.state.apply(Input::LiveRuns(rows)),
            UserEvent::Client(Report::Progress(progress)) => {
                self.state.apply(Input::Progress(progress))
            }
            // The details window's reads, which the Tray never asks for.
            UserEvent::Client(
                Report::Listed { .. }
                | Report::RunReport { .. }
                | Report::Usage { .. }
                | Report::Queried { .. }
                | Report::Explained { .. }
                | Report::SystemInfo(_)
                | Report::SeedsAdded { .. },
            ) => Vec::new(),
            UserEvent::DetailsOpened(pid) => {
                self.details = Some(pid);
                Vec::new()
            }
            UserEvent::CheckIcon => match tray::icon_check() {
                Some(check) => self.state.apply(Input::IconChecked(check)),
                None => Vec::new(),
            },
            UserEvent::Client(Report::Answered(call, answer)) => {
                self.state.apply(Input::Answered(call, answer))
            }
            #[cfg(not(target_os = "linux"))]
            UserEvent::MenuOpened => self.state.apply(Input::Menu(MenuChange::Opened(now()))),
            #[cfg(target_os = "macos")]
            UserEvent::MenuClosed => self.state.apply(Input::Menu(MenuChange::Closed)),
            #[cfg(not(target_os = "linux"))]
            UserEvent::Menu(id) => match tray::choice(&id) {
                Some(choice) => self.state.choose(choice),
                None => Vec::new(),
            },
            #[cfg(target_os = "linux")]
            UserEvent::Chose(choice) => self.state.choose(choice),
            #[cfg(target_os = "linux")]
            UserEvent::Confirmed(confirmation) => self.state.confirmed(confirmation),
            #[cfg(target_os = "linux")]
            UserEvent::Installed(installed) => {
                self.installed(event_loop, installed);
                return;
            }
        };
        self.carry_out(event_loop, effects);
    }
}
