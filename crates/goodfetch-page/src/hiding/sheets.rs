//! A page's hiding sheets on Chromium (ADRs 0047 and 0048): one style
//! sheet per document, made with `CSS.createStyleSheet`, which no element of
//! the document owns, so the site's `document.styleSheets` never holds it.
//! Live hiding and the Consent prompt runner each write their part of it
//! here, in any frame of the page, out-of-process iframes' targets included;
//! [`Documents`] decides what each change sends.
#![expect(
    clippy::disallowed_types,
    reason = "the documents' lock guards one step of bookkeeping, never across an `.await` or another lock, so the page's event loops never wait on a command; there is no lock order to test under loom (ADR 0060)"
)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::Failure;
use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::{css, dom, page};

use super::documents::{Documents, Key, Part, Write};
use crate::driver::PageDriver;

/// One page's hiding sheets.
pub(crate) struct Sheets {
    driver: PageDriver,
    documents: Mutex<Documents>,
    /// Held while a change's commands go out, so the browser gets each
    /// document's texts in the order they were set.
    sending: tokio::sync::Mutex<()>,
}

impl Sheets {
    /// The hiding sheets of the page `driver` drives.
    pub(crate) fn new(driver: PageDriver) -> Self {
        Self {
            driver,
            documents: Mutex::new(Documents::default()),
            sending: tokio::sync::Mutex::new(()),
        }
    }

    /// `session`'s `DOM` and `CSS` domains are on already.
    pub(crate) fn styled(&self, session: &TargetSession) {
        self.documents().styled(session);
    }

    /// `frame` of `session` committed the document of `loader`, which starts
    /// with an empty sheet.
    pub(crate) fn navigated(&self, session: &TargetSession, frame: &page::FrameId, loader: &str) {
        self.documents().navigated(session, frame, loader);
    }

    /// The iframe target `session` detached.
    pub(crate) fn forget_session(&self, session: &TargetSession) {
        self.documents().forget_session(session);
    }

    /// `frame` of `session` detached.
    pub(crate) fn forget_frame(&self, session: &TargetSession, frame: &page::FrameId) {
        self.documents().forget_frame(session, frame);
    }

    /// Sets `part` of the sheet of `frame`'s document on `session` to
    /// `text`, the sheet made on the document's first rule and its target's
    /// domains enabled first.
    ///
    /// # Errors
    ///
    /// The browser's failure when the document went or a domain was
    /// refused.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the browser's sheet with the previous
    /// text; the next change of the document writes it whole.
    pub(crate) async fn set(
        &self,
        session: &TargetSession,
        frame: &page::FrameId,
        part: Part,
        text: String,
    ) -> Result<(), Failure> {
        let _sending = self.sending.lock().await;
        let write = self
            .documents()
            .set(&(session.clone(), frame.clone()), part, text);
        match write {
            Some(write) => self.send(write).await,
            None => Ok(()),
        }
    }

    /// Empties every other document's sheet while a click in `frame` of
    /// `session` goes in, so no hidden owner swallows it, until
    /// [`Self::restore`]; nothing for a click in the page's own frame.
    pub(crate) async fn lift(&self, session: &TargetSession, frame: &page::FrameId) {
        let _sending = self.sending.lock().await;
        let page: Key = (
            self.driver.session().clone(),
            self.driver.main_frame().clone(),
        );
        let writes = self
            .documents()
            .lift(&(session.clone(), frame.clone()), &page);
        for write in writes {
            let _ = self.send(write).await;
        }
    }

    /// Gives every document lifted for a click its whole sheet again.
    pub(crate) async fn restore(&self) {
        let _sending = self.sending.lock().await;
        let writes = self.documents().restore();
        for write in writes {
            let _ = self.send(write).await;
        }
    }

    /// The documents, for one step of bookkeeping.
    fn documents(&self) -> MutexGuard<'_, Documents> {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends `write`'s commands and records what they did; a sheet the
    /// browser no longer knows, as its document went before the page's
    /// listeners saw it go, is made again once.
    async fn send(&self, write: Write) -> Result<(), Failure> {
        let mut sent = self.commands(&write).await;
        if sent.is_err() && write.sheet.is_some() {
            sent = self
                .commands(&Write {
                    sheet: None,
                    ..write.clone()
                })
                .await;
        }
        match &sent {
            Ok(sheet) => self.documents().written(&write, sheet.clone()),
            Err(_) => self.documents().failed(&write),
        }
        sent.map(drop)
    }

    /// `write`'s commands; the document's sheet.
    async fn commands(&self, write: &Write) -> Result<dom::StyleSheetId, Failure> {
        let (session, frame) = &write.key;
        let driver = &self.driver;
        if write.enable {
            driver
                .command_on(session, &dom::EnableParams::default())
                .await?;
            driver.command_on(session, &css::EnableParams {}).await?;
        }
        let sheet = match &write.sheet {
            Some(sheet) => sheet.clone(),
            None => {
                driver
                    .command_on(
                        session,
                        &css::CreateStyleSheetParams {
                            frame_id: frame.clone(),
                            force: None,
                        },
                    )
                    .await?
                    .style_sheet_id
            }
        };
        driver
            .command_on(
                session,
                &css::SetStyleSheetTextParams {
                    style_sheet_id: sheet.clone(),
                    text: write.text.clone(),
                },
            )
            .await?;
        Ok(sheet)
    }
}
