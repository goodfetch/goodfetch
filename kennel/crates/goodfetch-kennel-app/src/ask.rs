//! What the Kennel asks of the desktop for a Run's controls: the native
//! confirmation dialog that Stop, Abort and Retry failed in the menu, and
//! Add Seeds in the details window, show before they act, and the
//! clipboard "Copy resume command" writes, through `arboard`. Each runs on
//! the main thread, while the person chooses.
//!
//! - **macOS and Windows.** `rfd`: `NSAlert` on macOS, the message box on
//!   Windows.
//! - **Linux.** GTK's message dialog: the Tray's on its own GTK thread
//!   (`tray::confirm`); the details window's here, on the window's main
//!   thread, since that process runs no other GTK thread.

use std::cell::RefCell;

#[cfg(not(target_os = "linux"))]
use goodfetch_kennel::state::Confirmation;
#[cfg(not(target_os = "linux"))]
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

/// Asks `confirmation`'s question in a modal dialog: whether the person
/// confirmed. On Linux the Tray's GTK thread asks instead (`tray::confirm`).
#[cfg(not(target_os = "linux"))]
pub(crate) fn confirmed(confirmation: &Confirmation) -> bool {
    asked(
        &confirmation.title,
        &confirmation.consequence,
        &confirmation.confirm,
    )
}

/// Asks `title` in a modal dialog that says `consequence`, its confirming
/// button labelled `confirm`: whether the person confirmed.
#[cfg(not(target_os = "linux"))]
pub(crate) fn asked(title: &str, consequence: &str, confirm: &str) -> bool {
    // Windows names custom buttons only with Common Controls 6, which the
    // Kennel's executable does not declare: there the buttons read OK and
    // Cancel, and the title names the control.
    let buttons = if cfg!(windows) {
        MessageButtons::OkCancel
    } else {
        MessageButtons::OkCancelCustom(confirm.to_owned(), "Cancel".to_owned())
    };
    let answer = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(consequence)
        .set_buttons(buttons)
        .show();
    match answer {
        MessageDialogResult::Ok | MessageDialogResult::Yes => true,
        MessageDialogResult::Custom(label) => label == confirm,
        MessageDialogResult::No | MessageDialogResult::Cancel => false,
    }
}

/// Asks `title` in GTK's modal message dialog that says `consequence`, its
/// confirming button labelled `confirm`: whether the person confirmed. The
/// details window's only, on its main thread, the one GTK starts on in
/// that process; GTK that cannot start answers no.
#[cfg(target_os = "linux")]
pub(crate) fn asked(title: &str, consequence: &str, confirm: &str) -> bool {
    use gtk::prelude::{DialogExt as _, GtkWindowExt as _, MessageDialogExt as _};

    if let Err(error) = gtk::init() {
        tracing::warn!(%error, "GTK could not start, so nothing was asked or done");
        return false;
    }
    let dialog = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Warning,
        gtk::ButtonsType::None,
        title,
    );
    dialog.set_secondary_text(Some(consequence));
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button(confirm, gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let answer = dialog.run();
    dialog.close();
    // The closed dialog leaves the screen once GTK handles its events.
    while gtk::events_pending() {
        gtk::main_iteration();
    }
    answer == gtk::ResponseType::Accept
}

thread_local! {
    /// The clipboard, held for the Kennel's life: on X11 the text stays on
    /// the clipboard only while its owner lives.
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

/// Puts `text` on the clipboard.
pub(crate) fn copy(text: &str) {
    CLIPBOARD.with(|cell| {
        let mut clipboard = cell.borrow_mut();
        if clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(opened) => *clipboard = Some(opened),
                Err(error) => {
                    tracing::warn!(%error, "the clipboard cannot be opened");
                    return;
                }
            }
        }
        if let Some(clipboard) = clipboard.as_mut()
            && let Err(error) = clipboard.set_text(text)
        {
            tracing::warn!(%error, "the text was not copied");
        }
    });
}
