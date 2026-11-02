//! THROWAWAY SPIKE for goodfetch/goodfetch#1915, not Kennel code.
//!
//! On the owner's Mac it answers what the Linux server cannot: the icon and
//! the activation policy, a menu refreshed on click and while open, desktop
//! notices from an ad hoc `.app`, whether a notch-hidden icon is detectable,
//! and the memory and CPU of the tray process and of a details window run as
//! a second process. Every observation is printed with a timestamp.

#[cfg(target_os = "macos")]
mod mac;
mod window;

use std::cell::RefCell;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

static START: OnceLock<Instant> = OnceLock::new();

/// Ticks sent by the Tokio task, standing in for the Daemon's events.
pub(crate) static TICKS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn log(message: impl std::fmt::Display) {
    let elapsed = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    println!("[{elapsed:>9.3}s] {message}");
}

#[derive(Debug)]
enum UserEvent {
    Tick(u64),
    Menu(MenuEvent),
}

/// The menu items the click handler and the ticks rewrite. muda's items are
/// not `Send`, and both writers run on the main thread, so they live here.
struct Items {
    fresh: MenuItem,
    live: MenuItem,
}

thread_local! {
    static ITEMS: RefCell<Option<Items>> = const { RefCell::new(None) };
}

struct Ids {
    window: MenuId,
    notice: MenuId,
    notch: MenuId,
    quit: MenuId,
}

enum Action {
    Window,
    Notice,
    Notch,
    Quit,
}

#[derive(Default)]
struct App {
    tray: Option<TrayIcon>,
    ids: Option<Ids>,
    windows: Vec<Child>,
    #[cfg(target_os = "macos")]
    notices: Option<mac::Notices>,
}

impl App {
    fn build_tray(&mut self) {
        let fresh = MenuItem::new("Refreshed on click: not yet", false, None);
        let live = MenuItem::new("Live: tick 0", false, None);
        let window = MenuItem::new("Open the details window (second process)", true, None);
        let notice = MenuItem::new("Send a test desktop notice", true, None);
        let notch = MenuItem::new("Check whether the icon is visible", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &fresh,
            &live,
            &PredefinedMenuItem::separator(),
            &window,
            &notice,
            &notch,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .expect("the menu takes its items");
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_menu_on_right_click(true)
            .with_icon_templated(icon())
            .with_tooltip("Kennel spike")
            .build()
            .expect("the tray icon builds");
        log("tray icon built: look at the menu bar, and at the Dock for any flash");
        self.ids = Some(Ids {
            window: window.id().clone(),
            notice: notice.id().clone(),
            notch: notch.id().clone(),
            quit: quit.id().clone(),
        });
        ITEMS.with(|cell| *cell.borrow_mut() = Some(Items { fresh, live }));
        self.tray = Some(tray);
        #[cfg(target_os = "macos")]
        {
            self.notices = mac::Notices::new();
        }
    }

    fn on_tick(&mut self, tick: u64) {
        ITEMS.with(|cell| {
            if let Some(items) = cell.borrow().as_ref() {
                items.live.set_text(format!("Live: tick {tick}"));
            }
        });
        if tick.is_multiple_of(10) {
            log(format!("tick {tick} applied to the menu's live line"));
        }
        if tick == 3 {
            self.report_visibility("3 s after launch");
        }
        self.windows.retain_mut(|child| match child.try_wait() {
            Ok(Some(status)) => {
                log(format!("window process {} exited ({status})", child.id()));
                false
            }
            _ => true,
        });
    }

    fn on_menu(&mut self, event_loop: &ActiveEventLoop, id: &MenuId) {
        let action = match &self.ids {
            Some(ids) if *id == ids.window => Action::Window,
            Some(ids) if *id == ids.notice => Action::Notice,
            Some(ids) if *id == ids.notch => Action::Notch,
            Some(ids) if *id == ids.quit => Action::Quit,
            _ => return,
        };
        match action {
            Action::Window => {
                let exe = std::env::current_exe().expect("the executable's path");
                match Command::new(exe).arg("window").spawn() {
                    Ok(child) => {
                        log(format!("window process started, pid {}", child.id()));
                        self.windows.push(child);
                    }
                    Err(error) => log(format!("window process failed to start: {error}")),
                }
            }
            Action::Notice => self.send_notice(),
            Action::Notch => self.report_visibility("on request"),
            Action::Quit => {
                log("quit");
                event_loop.exit();
            }
        }
    }

    fn report_visibility(&self, when: &str) {
        let Some(tray) = &self.tray else { return };
        #[cfg(target_os = "macos")]
        log(format!("icon check ({when}): {}", mac::visibility(tray)));
        #[cfg(not(target_os = "macos"))]
        log(format!("icon check ({when}): rect {:?}", tray.rect()));
    }

    fn send_notice(&self) {
        #[cfg(target_os = "macos")]
        match &self.notices {
            Some(notices) => notices.send(),
            None => log("desktop notices are off outside a .app: run ./run.sh app"),
        }
        #[cfg(not(target_os = "macos"))]
        log("desktop notices: macOS only in this spike");
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // tray-icon: on macOS, build the icon once the loop runs, not before.
        if matches!(cause, StartCause::Init) {
            self.build_tray();
        }
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, _event: WindowEvent) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Tick(tick) => self.on_tick(tick),
            UserEvent::Menu(event) => {
                log(format!("menu item chosen: {:?}", event.id));
                self.on_menu(event_loop, &event.id);
            }
        }
    }
}

/// A filled disc, drawn as a template image so macOS tints it.
fn icon() -> Icon {
    const SIZE: u32 = 32;
    let centre = (SIZE as f32 - 1.0) / 2.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
            let alpha = if distance <= centre - 3.0 { 255 } else { 0 };
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("a valid icon")
}

fn main() {
    START.get_or_init(Instant::now);
    if std::env::args().nth(1).as_deref() == Some("window") {
        window::run();
        return;
    }
    log(format!("tray process, pid {}", std::process::id()));

    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_default_menu(false);
    }
    let event_loop = builder.build().expect("the event loop builds");
    event_loop.set_control_flow(ControlFlow::Wait);

    // Runs inside the click, before the menu opens: the refresh the design
    // relies on. muda's items stay on the main thread, which this is.
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button,
            button_state,
            rect,
            ..
        } = &event
        {
            if *button_state == MouseButtonState::Down {
                let tick = TICKS.load(Ordering::Relaxed);
                ITEMS.with(|cell| {
                    if let Some(items) = cell.borrow().as_ref() {
                        items
                            .fresh
                            .set_text(format!("Refreshed on click: tick {tick}"));
                    }
                });
                log(format!(
                    "click down ({button:?}) at icon rect {rect:?}: menu refreshed at tick {tick}, before it opens"
                ));
            } else {
                log(format!("click up ({button:?})"));
            }
        }
    }));

    let menu_proxy = Mutex::new(event_loop.create_proxy());
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Ok(proxy) = menu_proxy.lock() {
            let _ = proxy.send_event(UserEvent::Menu(event));
        }
    }));

    // One worker, as the Kennel's design holds it in `main`. The task stands
    // in for the Daemon's events reaching the UI through the loop's proxy.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("the runtime builds");
    let period = std::env::var("SPIKE_TICK_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(Duration::from_secs(1), Duration::from_millis);
    let tick_proxy = event_loop.create_proxy();
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(period);
        interval.tick().await;
        loop {
            interval.tick().await;
            let tick = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
            if tick_proxy.send_event(UserEvent::Tick(tick)).is_err() {
                break;
            }
        }
    });
    log(format!(
        "ticks every {period:?}; open the menu and keep it open to watch the live line"
    ));

    let mut app = App::default();
    event_loop.run_app(&mut app).expect("the event loop runs");
    runtime.shutdown_timeout(Duration::from_secs(1));
}
