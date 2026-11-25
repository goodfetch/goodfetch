//! What live hiding follows of a page's events, each read once from the
//! page's subscription, and the primitives that write a document's hiding
//! sheet (ADRs 0047 and 0048): one style sheet per document, which no
//! element of the document owns, so the site's `document.styleSheets`
//! never holds it.

use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{css, dom, page};
use goodfetch_engine::CallError;

use super::followed::FrameNavigated;
use super::paused::NotTaken;
use super::{CdpPage, TargetSession};

/// What live hiding follows of a page's events.
pub(crate) enum HidingEvent {
    /// A frame committed a navigation.
    Navigated(FrameNavigated),
    /// The document of `frame` reached the lifecycle event `name`.
    Lifecycle { frame: String, name: String },
}

/// A page's events live hiding follows.
pub(crate) struct HidingEvents(Subscription);

impl HidingEvents {
    /// The next event live hiding follows; `None` once the page's target
    /// went.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<HidingEvent> {
        while let Some(event) = self.0.next().await {
            if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
                return Some(HidingEvent::Navigated(FrameNavigated::of(navigated)));
            }
            if let Some(reached) = event.read::<page::LifecycleEventEvent>() {
                return Some(HidingEvent::Lifecycle {
                    frame: reached.frame_id,
                    name: reached.name,
                });
            }
        }
        None
    }
}

impl CdpPage {
    /// The page's events live hiding follows, from now.
    pub(crate) fn hiding_events(&self) -> HidingEvents {
        HidingEvents(self.page.events())
    }

    /// Has the page's target take style sheets, within a deadline of
    /// opening a page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn take_styles_opening(&self) -> Result<(), CallError> {
        self.page
            .opening_command(&dom::EnableParams::default())
            .await?;
        self.page.opening_command(&css::EnableParams {}).await?;
        Ok(())
    }

    /// Has `session`, the page's target or one of its out-of-process
    /// frames', take style sheets.
    ///
    /// # Errors
    ///
    /// When the target went.
    pub(crate) async fn take_styles(&self, session: &TargetSession) -> Result<(), NotTaken> {
        self.page
            .command_on(session, &dom::EnableParams::default())
            .await?;
        self.page.command_on(session, &css::EnableParams {}).await?;
        Ok(())
    }

    /// Makes a style sheet in the current document of `frame` on `session`,
    /// which no element of the document owns: answers its ID.
    ///
    /// # Errors
    ///
    /// When the frame or its document went.
    pub(crate) async fn create_style_sheet(
        &self,
        (session, frame): (&TargetSession, &str),
    ) -> Result<String, NotTaken> {
        let made = self
            .page
            .command_on(
                session,
                &css::CreateStyleSheetParams {
                    frame_id: frame.to_owned(),
                    force: None,
                },
            )
            .await?;
        Ok(made.style_sheet_id)
    }

    /// Sets the text of the style sheet `sheet` on `session` to `text`.
    ///
    /// # Errors
    ///
    /// When the sheet's document went.
    pub(crate) async fn set_style_sheet(
        &self,
        (session, sheet): (&TargetSession, &str),
        text: String,
    ) -> Result<(), NotTaken> {
        self.page
            .command_on(
                session,
                &css::SetStyleSheetTextParams {
                    style_sheet_id: sheet.to_owned(),
                    text,
                },
            )
            .await?;
        Ok(())
    }
}
