//! The Tray (ADR 0141): it draws the State's view with `tray-icon` and
//! `muda` on the main thread. A left and a right click both open the menu,
//! which is rebuilt in the click's mouse-down handler, before it opens, and
//! again on each view change while it stays open. Menu choices reach the
//! event loop through its proxy, as `UserEvent::Menu`.
//!
//! `muda`'s items are not `Send`, and the mouse-down handler is a `'static`
//! callback that cannot reach the application, so the Tray lives in a
//! thread-local of the main thread, where both run.

use std::cell::RefCell;

use goodfetch_kennel::state::{Choice, Row, Verdict, View};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// The icon in the menu bar and its menu.
struct Tray {
    icon: TrayIcon,
    menu: Menu,
    /// The id of each choice the menu shows now.
    choices: Vec<(MenuId, Choice)>,
    /// The view drawn, which the mouse-down handler draws again.
    view: View,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// Sends menu choices to the event loop, and rebuilds the menu on each
/// click's mouse-down, before it opens.
pub(crate) fn listen(proxy: EventLoopProxy<UserEvent>) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        // The loop has stopped: the Kennel is quitting.
        let _ = proxy.send_event(UserEvent::Menu(event.id));
    }));
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button_state: MouseButtonState::Down,
            ..
        } = event
        {
            with_tray(Tray::draw_menu);
        }
    }));
}

/// Puts the icon in the menu bar, drawing `view`. Call it once the event
/// loop runs: on macOS the status item needs the running application.
pub(crate) fn install(view: View) -> tray_icon::Result<()> {
    let menu = Menu::new();
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(true)
        .with_menu_on_right_click(true)
        .with_icon_templated(icon(view.verdict))
        .with_tooltip("Kennel")
        .build()?;
    let mut tray = Tray {
        icon,
        menu,
        choices: Vec::new(),
        view,
    };
    tray.draw_menu();
    TRAY.with(|cell| *cell.borrow_mut() = Some(tray));
    Ok(())
}

/// Draws `view`, the icon only when its verdict changed.
pub(crate) fn show(view: View) {
    with_tray(|tray| {
        if tray.view.verdict != view.verdict
            && let Err(error) = tray.icon.set_icon_templated(Some(icon(view.verdict)))
        {
            tracing::warn!(%error, "the icon was not redrawn");
        }
        let changed_rows = tray.view.rows != view.rows;
        tray.view = view;
        if changed_rows {
            tray.draw_menu();
        }
    });
}

/// The choice behind the menu item `id`, if the menu still shows it.
pub(crate) fn choice(id: &MenuId) -> Option<Choice> {
    TRAY.with(|cell| {
        cell.borrow().as_ref().and_then(|tray| {
            tray.choices
                .iter()
                .find(|(shown, _)| shown == id)
                .map(|(_, choice)| *choice)
        })
    })
}

fn with_tray(change: impl FnOnce(&mut Tray)) {
    TRAY.with(|cell| {
        if let Some(tray) = cell.borrow_mut().as_mut() {
            change(tray);
        }
    });
}

impl Tray {
    /// Replaces the menu's items with the view's rows.
    fn draw_menu(&mut self) {
        while self.menu.remove_at(0).is_some() {}
        self.choices.clear();
        for row in &self.view.rows {
            let appended = match row {
                Row::Text(text) => self.menu.append(&MenuItem::new(text, false, None)),
                Row::Choice { label, choice } => {
                    let item = MenuItem::new(label, true, None);
                    self.choices.push((item.id().clone(), *choice));
                    self.menu.append(&item)
                }
                Row::Separator => self.menu.append(&PredefinedMenuItem::separator()),
            };
            if let Err(error) = appended {
                tracing::warn!(%error, "a menu item was not added");
            }
        }
    }
}

/// The icon of `verdict`, drawn as a template image so the menu bar tints
/// it, with no numbers: a filled disc for an idle Daemon, a ring for an
/// unreachable one.
fn icon(verdict: Verdict) -> Icon {
    const SIZE: u32 = 32;
    let centre = (f64::from(SIZE) - 1.0) / 2.0;
    let outer = centre - 3.0;
    let inner = match verdict {
        Verdict::Idle => 0.0,
        Verdict::Unreachable => outer - 4.0,
    };
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = (f64::from(x) - centre).hypot(f64::from(y) - centre);
            let alpha = if distance <= outer && distance >= inner {
                255
            } else {
                0
            };
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE)
        .expect("invariant: the icon's buffer holds SIZE × SIZE pixels")
}
