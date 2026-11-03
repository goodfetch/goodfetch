//! The Kennel's executable (ADR 0141): the Tray, over the modules of
//! `goodfetch-kennel`.
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

mod ask;
mod tray;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Context;
use goodfetch_kennel::client::{self, CONNECTION_LOST, Call, CallFailure, Report, Wanted};
use goodfetch_kennel::daemon_start;
use goodfetch_kennel::home::Home;
use goodfetch_kennel::settings::{self, Settings, SettingsError};
use goodfetch_kennel::state::{Effect, Input, MenuChange, Redraw, State};
use goodfetch_kennel::tasks::Tasks;
use tokio::sync::{mpsc, watch};
use tray_icon::menu::MenuId;
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

/// How many calls a person makes may wait for the Client to send them.
const CALLS_WAITING: usize = 16;

/// How long the Kennel waits for its tasks once it quits.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// What reaches the UI loop from elsewhere.
#[derive(Debug)]
pub(crate) enum UserEvent {
    /// An input for the State, from a task.
    Input(Input),
    /// What the Client reports.
    Client(Report),
    /// The person chose a menu item.
    Menu(MenuId),
    /// The menu opened, drawing the latest view.
    MenuOpened,
    /// The menu closed.
    MenuClosed,
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
    start_tasks(&tasks, &home, looks, wanted_now, calls_made, &proxy);
    let mut kennel = Kennel {
        state: State::new(),
        tasks: tasks.clone(),
        proxy,
        look_again,
        wanted,
        calls,
        installed: false,
        next_draw: None,
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
    /// Whether the icon is in the menu bar.
    installed: bool,
    /// When a paced redraw falls due.
    next_draw: Option<Instant>,
}

impl Kennel {
    /// Carries out the State's `effects`, then keeps the Client's reads
    /// and the Tray in step with the State.
    fn carry_out(&mut self, event_loop: &ActiveEventLoop, effects: Vec<Effect>) {
        let mut effects = VecDeque::from(effects);
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::Quit => event_loop.exit(),
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
            tray::keep(self.state.view());
            self.redraw();
        }
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

/// The instant the State paces its redraws by.
fn now() -> Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Tray's one clock read: the State takes the instant it paces redraws by, and reads no clock itself (ADR 0059)"
    )]
    Instant::now()
}

impl ApplicationHandler<UserEvent> for Kennel {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            // On macOS the status item is made once the loop runs, not
            // before.
            StartCause::Init => match tray::install(self.state.view()) {
                Ok(()) => {
                    self.installed = true;
                    if !tray::FOLLOWS_THE_MENU {
                        // Nothing says when the menu opens or closes here,
                        // so it counts as open from launch.
                        let effects = self.state.apply(Input::Menu(MenuChange::Opened(now())));
                        self.carry_out(event_loop, effects);
                    }
                    self.redraw();
                }
                Err(error) => {
                    tracing::error!(%error, "the icon could not be put in the menu bar");
                    event_loop.exit();
                }
            },
            StartCause::ResumeTimeReached { .. } => self.redraw(),
            StartCause::WaitCancelled { .. } | StartCause::Poll => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(match self.next_draw {
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
            UserEvent::Input(input) => self.state.apply(input),
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
            UserEvent::Client(Report::Answered(call, answer)) => {
                self.state.apply(Input::Answered(call, answer))
            }
            UserEvent::MenuOpened => self.state.apply(Input::Menu(MenuChange::Opened(now()))),
            UserEvent::MenuClosed => self.state.apply(Input::Menu(MenuChange::Closed)),
            UserEvent::Menu(id) => match tray::choice(&id) {
                Some(choice) => self.state.choose(choice),
                None => Vec::new(),
            },
        };
        self.carry_out(event_loop, effects);
    }
}
