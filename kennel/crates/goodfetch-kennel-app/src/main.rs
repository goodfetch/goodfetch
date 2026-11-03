//! The Kennel's executable (ADR 0141): the Tray, over the modules of
//! `goodfetch-kennel`.
//!
//! A plain `fn main` builds a one-worker multi-thread Tokio runtime and
//! holds it, then runs the UI loop, `winit`'s, on the main thread (ADR
//! 0060). Every task starts through the `Tasks` tracker, and a task reaches
//! the UI only through the event loop's proxy, so the UI thread makes no
//! blocking call. On macOS the Kennel has no Dock icon (`.accessory`).

mod tray;

use std::time::Duration;

use anyhow::Context;
use goodfetch_kennel::client::{self, ConnectionChange};
use goodfetch_kennel::daemon_start;
use goodfetch_kennel::home::Home;
use goodfetch_kennel::settings::{self, Settings, SettingsError};
use goodfetch_kennel::state::{Effect, Input, State};
use goodfetch_kennel::tasks::Tasks;
use tokio::sync::{mpsc, watch};
use tray_icon::menu::MenuId;
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

/// How long the Kennel waits for its tasks once it quits.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// What reaches the UI loop from elsewhere.
#[derive(Debug)]
pub(crate) enum UserEvent {
    /// An input for the State, from a task.
    Input(Input),
    /// The person chose a menu item.
    Menu(MenuId),
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
    start_tasks(&tasks, &home, looks, &proxy);
    let mut kennel = Kennel {
        state: State::new(),
        tasks: tasks.clone(),
        proxy,
        look_again,
        installed: false,
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
    proxy: &EventLoopProxy<UserEvent>,
) {
    let to_ui = |proxy: &EventLoopProxy<UserEvent>| {
        let proxy = proxy.clone();
        move |input: Input| {
            // The loop has stopped: the Kennel is quitting.
            let _ = proxy.send_event(UserEvent::Input(input));
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
        },
        move |change: ConnectionChange| report(Input::Connection(change)),
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
            report(Input::Settings(settings));
        },
    ));
    let report = to_ui(proxy);
    let lookup = home.clone();
    let blocking = tasks.clone();
    tasks.spawn(async move {
        let installed = blocking.blocking(move || daemon_start::find(&lookup)).await;
        report(Input::Installed(installed));
    });
}

/// The UI loop's application.
struct Kennel {
    state: State,
    tasks: Tasks,
    proxy: EventLoopProxy<UserEvent>,
    /// Asks the Client to look for the Daemon again.
    look_again: mpsc::Sender<()>,
    /// Whether the icon is in the menu bar.
    installed: bool,
}

impl Kennel {
    /// Carries out the State's `effects`, then draws its view.
    fn carry_out(&mut self, event_loop: &ActiveEventLoop, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Quit => event_loop.exit(),
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
        if self.installed {
            tray::show(self.state.view());
        }
    }
}

impl ApplicationHandler<UserEvent> for Kennel {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        // On macOS the status item is made once the loop runs, not before.
        if matches!(cause, StartCause::Init) {
            match tray::install(self.state.view()) {
                Ok(()) => self.installed = true,
                Err(error) => {
                    tracing::error!(%error, "the icon could not be put in the menu bar");
                    event_loop.exit();
                }
            }
        }
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
            UserEvent::Menu(id) => match tray::choice(&id) {
                Some(choice) => self.state.choose(choice),
                None => Vec::new(),
            },
        };
        self.carry_out(event_loop, effects);
    }
}
