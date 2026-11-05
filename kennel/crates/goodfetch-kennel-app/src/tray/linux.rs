//! The Tray on Linux (#2077): `tray-icon`'s default backend, `AppIndicator`,
//! and its `muda` menu are GTK 3's, and GTK runs only on the thread that
//! initialized it, under its main loop there. So the Tray runs on a GTK
//! thread of its own, started through the `Tasks` tracker: `gtk::init`,
//! the icon and its menu, then `gtk::main` until the Kennel stops.
//!
//! - **What it draws.** The UI loop keeps the State and its pacing. Each
//!   frame it draws updates the latest drawing, a verdict and the menu's
//!   rows, in a one-slot channel the GTK thread reads: a newer drawing
//!   replaces one not drawn yet, so the channel never grows and the menu
//!   ends on the latest rows.
//! - **The menu, in place.** `AppIndicator` keeps the first menu it is given:
//!   it cannot be replaced, but its content can change. The GTK thread
//!   keeps the menu it set and applies `goodfetch_kennel::menu`'s edits to
//!   it: a row that keeps its kind keeps its item, redrawn by `set_text`
//!   and `set_enabled`, and only a row of another kind, or a change in the
//!   number of rows, inserts or removes items.
//! - **Choices.** `muda` reports a choice on the GTK thread, which finds
//!   the choice behind the item and sends it to the UI loop.
//! - **Confirmations.** Stop, Abort and Retry failed ask in GTK's own
//!   message dialog, on the GTK thread, and the answer reaches the UI loop
//!   only when the person confirms. `rfd` is not used here: it would start
//!   a GTK thread of its own beside this one, and GTK runs on one thread.
//! - **Its start.** `install` starts the GTK thread and returns at once:
//!   the UI thread never waits for GTK. The GTK thread reports whether the
//!   icon is in the panel through the event loop's proxy, as
//!   `UserEvent::Installed`, and the UI loop draws nothing before.
//! - **The menu's opening and closing** are not reported by `AppIndicator`,
//!   so the menu counts as open from launch, as on Windows. The panel
//!   sends dbusmenu's `opened` and `closed` events, but `libdbusmenu-gtk`
//!   turns them into GTK's `show` and `hide` only on a submenu's widget,
//!   never on the root menu `AppIndicator` is given, so no GTK signal says
//!   when the person opens the menu.

use std::cell::{Cell, RefCell};

use goodfetch_kennel::menu::{self, Edit};
use goodfetch_kennel::state::{Choice, Confirmation, Frame, Row, Verdict, View};
use goodfetch_kennel::tasks::Tasks;
use gtk::glib;
use gtk::prelude::*;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};
use winit::event_loop::EventLoopProxy;

use super::{InstallError, icon, label, line};
use crate::UserEvent;

/// How many confirmations may wait for the GTK thread to show them.
const CONFIRMATIONS_WAITING: usize = 4;

/// What the GTK thread draws: the latest verdict and rows.
#[derive(Clone, Debug)]
struct Drawing {
    verdict: Verdict,
    rows: Vec<Row>,
}

/// The UI thread's side of the GTK thread.
struct Link {
    drawing: watch::Sender<Drawing>,
    confirmations: mpsc::Sender<Confirmation>,
}

thread_local! {
    /// On the UI thread: the event loop's proxy, from `listen` to
    /// `install`.
    static PROXY: RefCell<Option<EventLoopProxy<UserEvent>>> = const { RefCell::new(None) };
    /// On the UI thread: the GTK thread's channels, once it runs.
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
    /// On the GTK thread: the icon and its menu.
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// Sends each menu choice to the event loop, as the GTK thread finds it.
pub(crate) fn listen(proxy: EventLoopProxy<UserEvent>) {
    let choices = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        // `muda` reports a choice on the GTK thread, where the Tray is.
        if let Some(choice) = choice(&event.id) {
            // The loop has stopped: the Kennel is quitting.
            let _ = choices.send_event(UserEvent::Chose(choice));
        }
    }));
    PROXY.with(|cell| *cell.borrow_mut() = Some(proxy));
}

/// Starts the GTK thread, which puts the icon in the panel drawing `view`
/// and reports whether it did as `UserEvent::Installed`; returns at once,
/// with no result, unless the Tray cannot start.
pub(crate) fn install(view: View, tasks: &Tasks) -> Option<Result<(), InstallError>> {
    let Some(proxy) = PROXY.with(|cell| cell.borrow_mut().take()) else {
        return Some(Err(InstallError::Twice));
    };
    let (drawing, drawings) = watch::channel(Drawing {
        verdict: view.verdict,
        rows: view.rows,
    });
    let (confirmations, asked) = mpsc::channel(CONFIRMATIONS_WAITING);
    let stop = tasks.stopping();
    let thread = tasks.clone();
    tasks.spawn(async move {
        thread
            .blocking(move || run(drawings, asked, &proxy, &stop))
            .await;
    });
    // A drawing made before the GTK thread runs waits in the channel,
    // which the thread reads as it starts.
    LINK.with(|cell| {
        *cell.borrow_mut() = Some(Link {
            drawing,
            confirmations,
        });
    });
    None
}

/// Draws `frame` on the GTK thread: the icon of its verdict, its rows into
/// the menu.
pub(crate) fn draw(frame: Frame) {
    LINK.with(|cell| {
        if let Some(link) = cell.borrow().as_ref() {
            link.drawing.send_modify(|drawing| {
                if let Some(verdict) = frame.verdict {
                    drawing.verdict = verdict;
                }
                if let Some(rows) = frame.rows {
                    drawing.rows = rows;
                }
            });
        }
    });
}

/// Asks `confirmation`'s question on the GTK thread; the event loop hears
/// `UserEvent::Confirmed` once the person confirms.
pub(crate) fn confirm(confirmation: Confirmation) {
    LINK.with(|cell| {
        if let Some(link) = cell.borrow().as_ref()
            && link.confirmations.try_send(confirmation).is_err()
        {
            // Full: the person already has questions open; this one is
            // dropped, and the control can be chosen again.
            tracing::warn!("a confirmation was not asked: others wait to be answered");
        }
    });
}

/// The GTK thread: GTK's start, the icon, reported to the UI loop, then
/// GTK's main loop, which draws each drawing and asks each confirmation
/// until the Kennel stops.
fn run(
    mut drawings: watch::Receiver<Drawing>,
    mut asked: mpsc::Receiver<Confirmation>,
    proxy: &EventLoopProxy<UserEvent>,
    stop: &CancellationToken,
) {
    let tray = gtk::init()
        .map_err(InstallError::Gtk)
        .and_then(|()| Tray::install(&drawings.borrow_and_update()));
    let tray = match tray {
        Ok(tray) => tray,
        Err(error) => {
            // The loop has stopped: the Kennel is quitting.
            let _ = proxy.send_event(UserEvent::Installed(Err(error)));
            return;
        }
    };
    TRAY.with(|cell| *cell.borrow_mut() = Some(tray));
    let _ = proxy.send_event(UserEvent::Installed(Ok(())));
    let proxy = proxy.clone();
    let stop = stop.clone();
    glib::MainContext::default().spawn_local(async move {
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                changed = drawings.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let drawing = drawings.borrow_and_update().clone();
                    with_tray(|tray| tray.show(&drawing));
                }
                Some(confirmation) = asked.recv() => ask(confirmation, &proxy),
            }
        }
        gtk::main_quit();
    });
    gtk::main();
    // The icon leaves the panel with the thread.
    TRAY.with(|cell| cell.borrow_mut().take());
}

/// The choice behind the menu item `id`, if the menu still shows it; on
/// the GTK thread.
fn choice(id: &MenuId) -> Option<Choice> {
    TRAY.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|tray| find(&tray.items, id))
    })
}

fn find(items: &[Item], id: &MenuId) -> Option<Choice> {
    items.iter().find_map(|item| match item {
        Item::Line { item, choice } if item.id() == id => choice.clone(),
        Item::Submenu { items, .. } => find(items, id),
        Item::Line { .. } | Item::Separator(_) => None,
    })
}

fn with_tray(change: impl FnOnce(&mut Tray)) {
    TRAY.with(|cell| {
        if let Some(tray) = cell.borrow_mut().as_mut() {
            change(tray);
        }
    });
}

/// Shows `confirmation`'s question in GTK's message dialog, without
/// waiting for the answer, which reaches the event loop as
/// `UserEvent::Confirmed` when the person confirms.
fn ask(confirmation: Confirmation, proxy: &EventLoopProxy<UserEvent>) {
    let dialog = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Warning,
        gtk::ButtonsType::None,
        &confirmation.title,
    );
    dialog.set_secondary_text(Some(&confirmation.consequence));
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button(&confirmation.confirm, gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let proxy = proxy.clone();
    let waiting = Cell::new(Some(confirmation));
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept
            && let Some(confirmation) = waiting.take()
        {
            // The loop has stopped: the Kennel is quitting.
            let _ = proxy.send_event(UserEvent::Confirmed(confirmation));
        }
        // A closed window answers `DeleteEvent`, and GTK destroys it then.
        if response != gtk::ResponseType::DeleteEvent {
            dialog.close();
        }
    });
    dialog.show();
}

/// The icon in the panel and the menu `AppIndicator` keeps, with the rows it
/// shows.
struct Tray {
    icon: TrayIcon,
    menu: Menu,
    verdict: Verdict,
    /// The rows the menu shows.
    shown: Vec<Row>,
    /// The menu's items, one per shown row.
    items: Vec<Item>,
}

/// The toolkit item that shows a row.
enum Item {
    /// A text, a detail or a choice; enabled only for a choice.
    Line {
        item: MenuItem,
        choice: Option<Choice>,
    },
    Separator(PredefinedMenuItem),
    Submenu {
        submenu: Submenu,
        items: Vec<Item>,
    },
}

/// The menu or submenu an item is in.
enum Parent<'a> {
    Menu(&'a Menu),
    Submenu(&'a Submenu),
}

impl Tray {
    /// Puts the icon of `drawing` in the panel, with a menu of its rows:
    /// the menu `AppIndicator` keeps.
    fn install(drawing: &Drawing) -> Result<Self, InstallError> {
        let menu = Menu::new();
        let items: Vec<Item> = drawing.rows.iter().map(item).collect();
        for item in &items {
            if let Err(error) = menu.append(item.as_menu_item()) {
                tracing::warn!(%error, "a menu item was not added");
            }
        }
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_icon(icon(drawing.verdict))
            .with_tooltip("Kennel")
            .build()?;
        Ok(Self {
            icon,
            menu,
            verdict: drawing.verdict,
            shown: drawing.rows.clone(),
            items,
        })
    }

    /// Shows `drawing`: the icon of its verdict when it changed, and its
    /// rows through the menu's edits.
    fn show(&mut self, drawing: &Drawing) {
        if drawing.verdict != self.verdict {
            if let Err(error) = self.icon.set_icon(Some(icon(drawing.verdict))) {
                tracing::warn!(%error, "the icon was not redrawn");
            }
            self.verdict = drawing.verdict;
        }
        for edit in menu::edits(&self.shown, &drawing.rows) {
            self.apply(edit);
        }
        self.shown.clone_from(&drawing.rows);
    }

    fn apply(&mut self, edit: Edit<'_>) {
        match edit {
            Edit::Redraw { at, row } => {
                let (index, _, items) = self.place(&at);
                redraw(&mut items[index], row);
            }
            Edit::Insert { at, row } => {
                let (index, parent, items) = self.place(&at);
                let item = item(row);
                let inserted = match parent {
                    Parent::Menu(menu) => menu.insert(item.as_menu_item(), index),
                    Parent::Submenu(submenu) => submenu.insert(item.as_menu_item(), index),
                };
                if let Err(error) = inserted {
                    tracing::warn!(%error, "a menu item was not added");
                }
                items.insert(index, item);
            }
            Edit::Remove { at } => {
                let (index, parent, items) = self.place(&at);
                let removed = match parent {
                    Parent::Menu(menu) => menu.remove_at(index),
                    Parent::Submenu(submenu) => submenu.remove_at(index),
                };
                if removed.is_none() {
                    tracing::warn!("a menu item was not removed");
                }
                items.remove(index);
            }
        }
    }

    /// The index `at` ends with, the menu or submenu it indexes, and that
    /// one's items.
    fn place(&mut self, at: &[usize]) -> (usize, Parent<'_>, &mut Vec<Item>) {
        let (index, above) = at
            .split_last()
            .expect("invariant: every edit names its row's place");
        let mut parent = Parent::Menu(&self.menu);
        let mut items = &mut self.items;
        for step in above {
            let (submenu, rows) = match items.get_mut(*step) {
                Some(Item::Submenu { submenu, items }) => Some((submenu, items)),
                Some(Item::Line { .. } | Item::Separator(_)) | None => None,
            }
            .expect("invariant: the edits name the submenus the menu shows");
            parent = Parent::Submenu(submenu);
            items = rows;
        }
        (*index, parent, items)
    }
}

impl Item {
    fn as_menu_item(&self) -> &dyn IsMenuItem {
        match self {
            Self::Line { item, .. } => item,
            Self::Separator(separator) => separator,
            Self::Submenu { submenu, .. } => submenu,
        }
    }
}

/// The item that shows `row`, a submenu's items included.
fn item(row: &Row) -> Item {
    match row {
        Row::Separator => Item::Separator(PredefinedMenuItem::separator()),
        Row::Submenu { label: text, rows } => {
            let submenu = Submenu::new(label(text), true);
            let items: Vec<Item> = rows.iter().map(item).collect();
            for item in &items {
                if let Err(error) = submenu.append(item.as_menu_item()) {
                    tracing::warn!(%error, "a submenu item was not added");
                }
            }
            Item::Submenu { submenu, items }
        }
        Row::Text(_) | Row::Detail(_) | Row::Choice { .. } => {
            let (text, choice) =
                line(row).expect("invariant: a text, a detail or a choice is a line");
            Item::Line {
                item: MenuItem::new(text, choice.is_some(), None),
                choice,
            }
        }
    }
}

/// Redraws `item` as `row`, of the same kind.
fn redraw(item: &mut Item, row: &Row) {
    match (item, row) {
        (Item::Submenu { submenu, .. }, Row::Submenu { label: text, .. }) => {
            submenu.set_text(label(text));
        }
        (Item::Line { item, choice }, row) => {
            if let Some((text, chosen)) = line(row) {
                item.set_text(text);
                item.set_enabled(chosen.is_some());
                *choice = chosen;
            }
        }
        (Item::Separator(_) | Item::Submenu { .. }, _) => {}
    }
}
