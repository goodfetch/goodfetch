//! The Tray (ADR 0141): it draws the State's view with `tray-icon` and
//! `muda` on the main thread. A left and a right click both open the menu.
//!
//! - **Fresh when it opens.** The Tray keeps the latest view, and the
//!   click's mouse-down handler rebuilds the menu from it before the menu
//!   opens, then tells the State the menu opened.
//! - **Live while open.** The State paces the redraws, and the Tray draws
//!   the frames it gives: the icon when the verdict changed, the menu's
//!   rows when they changed while it is open.
//! - **Its closing.** On macOS the Tray learns it from `AppKit`
//!   (`menu_closed`). `tray-icon` reports neither the opening nor the
//!   closing elsewhere, so on Linux and Windows the menu counts as open
//!   from launch (`MenuChange::Unreported`): it applies every change,
//!   paced, the State follows the progress of the Runs it lists for as
//!   long as the Kennel runs, and a failed or Limit-stopped Run, or a
//!   failed call, leaves `state::SHOWN_FOR` after it was first drawn.
//!
//! Menu choices reach the event loop through its proxy, as
//! `UserEvent::Menu`. `muda`'s items are not `Send`, and the mouse-down
//! handler is a `'static` callback that cannot reach the application, so
//! the Tray lives in a thread-local of the main thread, where both run.
//!
//! **On Linux** the Tray runs on a GTK thread of its own instead (`linux`):
//! `tray-icon`'s `AppIndicator` backend and its `muda` menu are GTK's, which
//! needs its main loop on the thread that builds them, and `AppIndicator`
//! keeps the first menu it is given, so the rows change in place.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod menu_closed;

#[cfg(not(target_os = "linux"))]
use std::cell::RefCell;

use goodfetch_kennel::state::{Choice, IconCheck, Row, Verdict};
#[cfg(not(target_os = "linux"))]
use goodfetch_kennel::state::{Frame, View};
#[cfg(not(target_os = "linux"))]
use goodfetch_kennel::tasks::Tasks;
use tray_icon::Icon;
#[cfg(not(target_os = "linux"))]
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
#[cfg(not(target_os = "linux"))]
use tray_icon::{MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
#[cfg(not(target_os = "linux"))]
use winit::event_loop::EventLoopProxy;

#[cfg(target_os = "linux")]
pub(crate) use linux::{confirm, draw, install, listen};

/// Why the icon could not be put in the menu bar or the panel.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InstallError {
    /// GTK, which the Linux Tray runs on, could not start.
    #[cfg(target_os = "linux")]
    #[error("GTK could not start: no display? ({0})")]
    Gtk(gtk::glib::BoolError),
    /// The Tray was installed before `listen`, or a second time.
    #[cfg(target_os = "linux")]
    #[error("the Tray is installed once, after `listen`")]
    Twice,
    /// `tray-icon` refused the icon.
    #[error("the icon could not be put in the menu bar: {0}")]
    Icon(#[from] tray_icon::Error),
}

#[cfg(not(target_os = "linux"))]
use crate::UserEvent;

/// What a detail line starts with, under the line it details.
const INDENT: &str = "    ";

/// The icon's colour. On macOS the icon is a template image, whose colour
/// the menu bar replaces. Elsewhere nothing tints it, and a panel may be
/// light or dark, so it is a grey that shows on both.
const INK: [u8; 3] = if cfg!(target_os = "macos") {
    [0, 0, 0]
} else {
    [0x80, 0x80, 0x80]
};

/// Whether the platform tells the Tray when its menu opens and closes.
pub(crate) const FOLLOWS_THE_MENU: bool = cfg!(target_os = "macos");

/// The icon in the menu bar and its menu.
#[cfg(not(target_os = "linux"))]
struct Tray {
    icon: TrayIcon,
    menu: Menu,
    /// The id of each choice the menu shows now, its submenus' included.
    choices: Vec<(MenuId, Choice)>,
    /// The latest view, which the mouse-down handler draws.
    latest: View,
}

#[cfg(not(target_os = "linux"))]
thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// Sends menu choices to the event loop, rebuilds the menu from the
/// latest view on each click's mouse-down, before it opens, and tells the
/// event loop the menu opened and, on macOS, closed.
#[cfg(not(target_os = "linux"))]
pub(crate) fn listen(proxy: EventLoopProxy<UserEvent>) {
    let choices = proxy.clone();
    #[cfg(target_os = "macos")]
    let closed = proxy.clone();
    let opened = proxy;
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        // The loop has stopped: the Kennel is quitting.
        let _ = choices.send_event(UserEvent::Menu(event.id));
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button_state: MouseButtonState::Down,
            ..
        } = event
        {
            with_tray(|tray| {
                let rows = tray.latest.rows.clone();
                tray.draw_rows(&rows);
            });
            if FOLLOWS_THE_MENU {
                let _ = opened.send_event(UserEvent::MenuOpened);
            }
        }
    }));
    #[cfg(target_os = "macos")]
    menu_closed::observe(move || {
        let _ = closed.send_event(UserEvent::MenuClosed);
    });
}

/// Puts the icon in the menu bar, drawing `view`, and gives whether it
/// did: always a result here, where the main thread builds the icon at
/// once. Call it once the event loop runs: on macOS the status item needs
/// the running application. The Tray starts no task here: `_tasks` is for
/// Linux's GTK thread.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as the Linux Tray's, whose GTK thread reports later"
)]
pub(crate) fn install(view: View, _tasks: &Tasks) -> Option<Result<(), InstallError>> {
    Some(build(view))
}

/// Builds the icon and its menu, drawing `view`.
#[cfg(not(target_os = "linux"))]
fn build(view: View) -> Result<(), InstallError> {
    let menu = Menu::new();
    let builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(true)
        .with_menu_on_right_click(true)
        .with_tooltip("Kennel");
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_templated(icon(view.verdict));
    #[cfg(not(target_os = "macos"))]
    let builder = builder.with_icon(icon(view.verdict));
    let icon = builder.build()?;
    let mut tray = Tray {
        icon,
        menu,
        choices: Vec::new(),
        latest: view,
    };
    let rows = tray.latest.rows.clone();
    tray.draw_rows(&rows);
    TRAY.with(|cell| *cell.borrow_mut() = Some(tray));
    Ok(())
}

/// Keeps `view` as the latest, which the menu draws as it opens; draws
/// nothing.
#[cfg(not(target_os = "linux"))]
pub(crate) fn keep(view: View) {
    with_tray(|tray| tray.latest = view);
}

/// Draws `frame`: the icon of its verdict, its rows into the menu.
#[cfg(not(target_os = "linux"))]
pub(crate) fn draw(frame: Frame) {
    with_tray(|tray| {
        if let Some(verdict) = frame.verdict {
            #[cfg(target_os = "macos")]
            let drawn = tray.icon.set_icon_templated(Some(icon(verdict)));
            #[cfg(not(target_os = "macos"))]
            let drawn = tray.icon.set_icon(Some(icon(verdict)));
            if let Err(error) = drawn {
                tracing::warn!(%error, "the icon was not redrawn");
            }
        }
        if let Some(rows) = frame.rows {
            tray.draw_rows(&rows);
        }
    });
}

/// The choice behind the menu item `id`, if the menu still shows it.
#[cfg(not(target_os = "linux"))]
pub(crate) fn choice(id: &MenuId) -> Option<Choice> {
    TRAY.with(|cell| {
        cell.borrow().as_ref().and_then(|tray| {
            tray.choices
                .iter()
                .find(|(shown, _)| shown == id)
                .map(|(_, choice)| choice.clone())
        })
    })
}

#[cfg(not(target_os = "linux"))]
fn with_tray(change: impl FnOnce(&mut Tray)) {
    TRAY.with(|cell| {
        if let Some(tray) = cell.borrow_mut().as_mut() {
            change(tray);
        }
    });
}

#[cfg(not(target_os = "linux"))]
impl Tray {
    /// Replaces the menu's items with `rows`.
    fn draw_rows(&mut self, rows: &[Row]) {
        while self.menu.remove_at(0).is_some() {}
        self.choices.clear();
        for row in rows {
            let item = item(row, &mut self.choices);
            if let Err(error) = self.menu.append(item.as_ref()) {
                tracing::warn!(%error, "a menu item was not added");
            }
        }
    }
}

/// The menu item that draws `row`, each choice it holds, a submenu's
/// included, added to `choices`.
#[cfg(not(target_os = "linux"))]
fn item(row: &Row, choices: &mut Vec<(MenuId, Choice)>) -> Box<dyn IsMenuItem> {
    match row {
        Row::Text(_) | Row::Detail(_) | Row::Choice { .. } => {
            let (text, choice) =
                line(row).expect("invariant: a text, a detail or a choice is a line");
            let item = MenuItem::new(text, choice.is_some(), None);
            if let Some(choice) = choice {
                choices.push((item.id().clone(), choice));
            }
            Box::new(item)
        }
        Row::Separator => Box::new(PredefinedMenuItem::separator()),
        Row::Submenu { label: text, rows } => {
            let submenu = Submenu::new(label(text), true);
            for row in rows {
                if let Err(error) = submenu.append(item(row, choices).as_ref()) {
                    tracing::warn!(%error, "a submenu item was not added");
                }
            }
            Box::new(submenu)
        }
    }
}

/// The label and the choice of the `muda` item that draws `row`, for both
/// Trays: a text, a detail indented under the line it details, or a
/// choice, the only one enabled; `None` for a separator or a submenu.
fn line(row: &Row) -> Option<(String, Option<Choice>)> {
    match row {
        Row::Text(text) => Some((label(text), None)),
        Row::Detail(text) => Some((label(&format!("{INDENT}{text}")), None)),
        Row::Choice {
            label: text,
            choice,
        } => Some((label(text), Some(choice.clone()))),
        Row::Separator | Row::Submenu { .. } => None,
    }
}

/// `text` as a `muda` label, which reads `&` as the mark of a mnemonic
/// and `&&` as a plain `&`.
fn label(text: &str) -> String {
    text.replace('&', "&&")
}

/// The icon of `verdict`, with no numbers (ADR 0141), drawn in `INK`: on
/// macOS a template image, which the menu bar tints:
/// - unreachable: a ring;
/// - a Needs you item: a disc with an exclamation mark cut out;
/// - a Run failed or stopped by a Limit: a disc with a bar cut across it;
/// - running: a ring with its left half filled;
/// - idle: a filled disc.
fn icon(verdict: Verdict) -> Icon {
    const SIZE: u32 = 32;
    let centre = (f64::from(SIZE) - 1.0) / 2.0;
    let outer = centre - 3.0;
    let ring = outer - 4.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = f64::from(x) - centre;
            let dy = f64::from(y) - centre;
            let distance = dx.hypot(dy);
            let disc = distance <= outer;
            let filled = match verdict {
                Verdict::Idle => disc,
                Verdict::Unreachable => disc && distance >= ring,
                Verdict::Running => disc && (distance >= ring || dx <= 0.0),
                Verdict::Failed => disc && dy.abs() > 2.5,
                Verdict::NeedsYou => {
                    let stem = dx.abs() <= 2.0 && (-9.0..=3.0).contains(&dy);
                    let dot = dx.abs() <= 2.0 && (6.0..=9.0).contains(&dy);
                    disc && !stem && !dot
                }
            };
            let alpha = if filled { 255 } else { 0 };
            let [red, green, blue] = INK;
            rgba.extend_from_slice(&[red, green, blue, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE)
        .expect("invariant: the icon's buffer holds SIZE × SIZE pixels")
}

/// Whether the icon shows in the menu bar, from the occlusion state of the
/// status item's window, never from its frame against the notch: macOS 27
/// reports items hidden in the overflow as on screen, and only the
/// occlusion state tells them apart (#1915). None when it cannot be read.
#[cfg(target_os = "macos")]
pub(crate) fn icon_check() -> Option<IconCheck> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindowOcclusionState;
    let main_thread = MainThreadMarker::new()?;
    TRAY.with(|cell| {
        let tray = cell.borrow();
        let item = tray.as_ref()?.icon.ns_status_item()?;
        let window = item.button(main_thread)?.window()?;
        let shown = window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible);
        Some(if shown {
            IconCheck::Shown
        } else {
            IconCheck::Hidden
        })
    })
}

/// Whether the icon shows: Linux and Windows have no notch, nor a state
/// that tells it, so the check is made on macOS only.
#[cfg(not(target_os = "macos"))]
pub(crate) fn icon_check() -> Option<IconCheck> {
    None
}

/// Brings the details window's process `pid` forward, with its windows.
#[cfg(target_os = "macos")]
pub(crate) fn raise(pid: u32) {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    let raised =
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid).is_some_and(|window| {
            window.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows)
        });
    if !raised {
        tracing::info!(pid, "the details window was not brought forward");
    }
}

/// Brings the details window forward: on Linux and Windows the Tray has no
/// way to, so the person finds it among their windows.
#[cfg(not(target_os = "macos"))]
pub(crate) fn raise(pid: u32) {
    tracing::info!(pid, "the details window is already open");
}
