//! A page's JavaScript dialogs as its reader sees them: each opening and
//! closing, read once from the page's subscription, and the primitive that
//! answers the open one.

use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::page;
use goodfetch_protocol::page::dialog::{Dialog, DialogType};

use super::CdpPage;
use super::paused::NotTaken;

/// What a page's reader of its dialogs follows.
pub(crate) enum DialogEvent {
    /// The page opened a dialog, which blocks it until it is answered.
    Opened(OpenedDialog),
    /// The open dialog closed.
    Closed,
}

/// A dialog the page opened.
pub(crate) struct OpenedDialog {
    /// The URL of the document that opened it.
    pub(crate) url: String,
    /// The dialog, as its handlers see it.
    pub(crate) dialog: Dialog,
}

impl OpenedDialog {
    fn of(opening: page::JavascriptDialogOpeningEvent) -> Self {
        let kind = match opening.r#type {
            page::DialogType::Alert => DialogType::Alert,
            page::DialogType::Prompt => DialogType::Prompt,
            page::DialogType::Beforeunload => DialogType::BeforeUnload,
            // A kind this protocol revision did not list is answered with
            // the default of a dialog that asks the program something.
            page::DialogType::Confirm | page::DialogType::Unlisted => DialogType::Confirm,
        };
        Self {
            url: opening.url,
            dialog: Dialog {
                kind,
                message: opening.message,
                default_value: opening.default_prompt.unwrap_or_default(),
            },
        }
    }
}

/// A page's dialogs, as the browser tells them.
pub(crate) struct DialogEvents(Subscription);

impl DialogEvents {
    /// The next dialog event; `None` once the page's target went.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<DialogEvent> {
        while let Some(event) = self.0.next().await {
            if let Some(opening) = event.read::<page::JavascriptDialogOpeningEvent>() {
                return Some(DialogEvent::Opened(OpenedDialog::of(opening)));
            }
            if event.read::<page::JavascriptDialogClosedEvent>().is_some() {
                return Some(DialogEvent::Closed);
            }
        }
        None
    }
}

impl CdpPage {
    /// The page's dialogs, from now.
    pub(crate) fn dialog_events(&self) -> DialogEvents {
        DialogEvents(self.page.events())
    }

    /// Answers the page's open dialog: accepts it, a prompt with
    /// `prompt_text`, when `accept`, else dismisses it.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the dialog or the page went first.
    pub(crate) async fn answer_dialog(
        &self,
        accept: bool,
        prompt_text: Option<String>,
    ) -> Result<(), NotTaken> {
        self.taken(&page::HandleJavaScriptDialogParams {
            accept,
            prompt_text,
        })
        .await
    }
}
