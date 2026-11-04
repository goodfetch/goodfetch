//! What the Tray asks of the desktop for a Run's controls: the native
//! confirmation dialog that Stop, Abort and Retry failed show before they
//! act, through `rfd` (`NSAlert` on macOS, the message box on Windows), and
//! the clipboard "Copy resume command" writes, through `arboard`. Both run
//! on the main thread, while the person chooses. On Linux the dialog is
//! GTK's, on the Tray's GTK thread (`tray::confirm`).

use std::cell::RefCell;

#[cfg(not(target_os = "linux"))]
use goodfetch_kennel::state::Confirmation;
#[cfg(not(target_os = "linux"))]
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

/// Asks `confirmation`'s question in a modal dialog: whether the person
/// confirmed. On Linux the Tray's GTK thread asks instead (`tray::confirm`).
#[cfg(not(target_os = "linux"))]
pub(crate) fn confirmed(confirmation: &Confirmation) -> bool {
    // Windows names custom buttons only with Common Controls 6, which the
    // Kennel's executable does not declare: there the buttons read OK and
    // Cancel, and the title names the control.
    let buttons = if cfg!(windows) {
        MessageButtons::OkCancel
    } else {
        MessageButtons::OkCancelCustom(confirmation.confirm.clone(), "Cancel".to_owned())
    };
    let answer = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(confirmation.title.as_str())
        .set_description(confirmation.consequence.as_str())
        .set_buttons(buttons)
        .show();
    match answer {
        MessageDialogResult::Ok | MessageDialogResult::Yes => true,
        MessageDialogResult::Custom(label) => label == confirmation.confirm,
        MessageDialogResult::No | MessageDialogResult::Cancel => false,
    }
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
