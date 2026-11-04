//! What the Kennel asks of the desktop for a Run's controls: the native
//! confirmation dialog that Stop, Abort and Retry failed in the menu, and
//! Add Seeds in the details window, show before they act, through `rfd`
//! (`NSAlert` on macOS, GTK 3 on Linux, the message box on Windows), and
//! the clipboard "Copy resume command" writes, through `arboard`. Both run
//! on the main thread, while the person chooses.

use std::cell::RefCell;

use goodfetch_kennel::state::Confirmation;
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

/// Asks `confirmation`'s question in a modal dialog: whether the person
/// confirmed.
pub(crate) fn confirmed(confirmation: &Confirmation) -> bool {
    asked(
        &confirmation.title,
        &confirmation.consequence,
        &confirmation.confirm,
    )
}

/// Asks `title` in a modal dialog that says `consequence`, its confirming
/// button labelled `confirm`: whether the person confirmed.
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
