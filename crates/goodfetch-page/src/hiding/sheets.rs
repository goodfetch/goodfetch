//! A page's hiding sheets on Chromium (ADRs 0047 and 0048): one style
//! sheet per document, made with `CSS.createStyleSheet`, which no element of
//! the document owns, so the site's `document.styleSheets` never holds it.
//! Live hiding and the Consent prompt runner each write their part of it
//! here, in any frame of the page, out-of-process iframes' targets included;
//! [`Documents`] decides what each change sends, and the page's sender, a
//! task of its Run's, sends the changes one at a time in the order they were
//! made, so no lock is held while their commands go out.
#![expect(
    clippy::disallowed_types,
    reason = "the documents' lock guards one step of bookkeeping, never across an `.await` or another lock, so the page's event loops never wait on a command; there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::Failure;
use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::{css, dom, page};
use goodfetch_engine::runs::Run;
use tokio::sync::{mpsc, oneshot};

use super::documents::{Documents, Key, Part, Write};
use crate::driver::PageDriver;

/// How many changes wait for the page's sender, at most, before the next
/// waits for room.
const QUEUED: usize = 16;

/// One page's hiding sheets.
pub(crate) struct Sheets {
    driver: PageDriver,
    documents: Arc<Mutex<Documents>>,
    /// The changes the page's sender sends, in the order they were made, so
    /// the browser gets each document's texts in the order they were set.
    changes: mpsc::Sender<Queued>,
}

/// One change of a page's sheets, and where its outcome goes once sent.
struct Queued {
    change: Change,
    sent: oneshot::Sender<Result<(), Failure>>,
}

/// What a change of a page's sheets does.
enum Change {
    /// Sets a part of a document's sheet; with an order, only when no
    /// change of the same part of the same document with a later order was
    /// sent before it.
    Set {
        key: Key,
        part: Part,
        text: String,
        order: Option<u64>,
    },
    /// Starts live hiding in a document; with a loader, only in that load's
    /// document.
    Release { key: Key, loader: Option<String> },
    /// Empties every document's sheet but those of the frame a click goes
    /// in and of the page itself.
    Lift { key: Key },
    /// Gives every lifted document its whole sheet again.
    Restore,
    /// Lifts the runner's rules of the selectors from a document's sheet
    /// until its next load.
    LiftConsent { key: Key, selectors: Vec<String> },
}

/// The page's sender: it sends each change's commands, one change at a
/// time.
struct Sender {
    driver: PageDriver,
    documents: Arc<Mutex<Documents>>,
    /// The order of the last ordered change sent, by document and part.
    orders: BTreeMap<(Key, Part), u64>,
}

impl Sheets {
    /// The hiding sheets of the page `driver` drives for `run`, whose
    /// sender runs as a task of the Run's until the sheets go.
    pub(crate) fn new(driver: PageDriver, run: &Run) -> Self {
        let documents = Arc::new(Mutex::new(Documents::default()));
        let (changes, mut queued) = mpsc::channel::<Queued>(QUEUED);
        let mut sender = Sender {
            driver: driver.clone(),
            documents: Arc::clone(&documents),
            orders: BTreeMap::new(),
        };
        let live = run.cancellation().clone();
        run.spawn(async move {
            loop {
                let Queued { change, sent } = tokio::select! {
                    () = live.cancelled() => return,
                    queued = queued.recv() => match queued {
                        Some(queued) => queued,
                        None => return,
                    },
                };
                let outcome = sender.apply(change).await;
                // A caller that went waits for no outcome.
                let _ = sent.send(outcome);
            }
        });
        Self {
            driver,
            documents,
            changes,
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
    /// domains enabled first; with `order`, only when no change of that part
    /// with a later order was sent before it.
    ///
    /// # Errors
    ///
    /// The browser's failure when the document went or a domain was
    /// refused, and `Disconnected` once the page's Run stopped its sender.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the change is queued leaves it to be sent
    /// all the same.
    pub(crate) async fn set(
        &self,
        (session, frame): (&TargetSession, &page::FrameId),
        part: Part,
        text: String,
        order: Option<u64>,
    ) -> Result<(), Failure> {
        self.queue(Change::Set {
            key: (session.clone(), frame.clone()),
            part,
            text,
            order,
        })
        .await
    }

    /// Starts live hiding in the sheet of `frame`'s document on `session`,
    /// which waited for the Consent prompt runner's first pass there; with
    /// `loader`, only in the document of that load.
    ///
    /// # Errors
    ///
    /// As [`Self::set`].
    pub(crate) async fn release(
        &self,
        session: &TargetSession,
        frame: &page::FrameId,
        loader: Option<&str>,
    ) -> Result<(), Failure> {
        self.queue(Change::Release {
            key: (session.clone(), frame.clone()),
            loader: loader.map(ToOwned::to_owned),
        })
        .await
    }

    /// The selectors of the rules the Consent prompt runner hides prompts
    /// with in the page's main document, which extraction removes too
    /// (ADR 0048).
    pub(crate) fn runner_hidden(&self) -> Vec<String> {
        self.documents().consent_selectors(&page_key(&self.driver))
    }

    /// The selectors of [`Self::runner_hidden`] that still hide there: those
    /// the guard did not lift.
    pub(crate) fn runner_applied(&self) -> Vec<String> {
        self.documents().consent_applied(&page_key(&self.driver))
    }

    /// Lifts the runner's rules of `selectors` from the sheet of the page's
    /// main document until its next load, whatever the runner sets there
    /// meanwhile, as the guard found them hiding an action's target.
    ///
    /// # Errors
    ///
    /// As [`Self::set`].
    pub(crate) async fn lift_runner(&self, selectors: Vec<String>) -> Result<(), Failure> {
        self.queue(Change::LiftConsent {
            key: page_key(&self.driver),
            selectors,
        })
        .await
    }

    /// Empties every other document's sheet while a click in `frame` of
    /// `session` goes in, so no hidden owner swallows it, until
    /// [`Self::restore`]; nothing for a click in the page's own frame.
    pub(crate) async fn lift(&self, session: &TargetSession, frame: &page::FrameId) {
        let _ = self
            .queue(Change::Lift {
                key: (session.clone(), frame.clone()),
            })
            .await;
    }

    /// Gives every document lifted for a click its whole sheet again.
    pub(crate) async fn restore(&self) {
        let _ = self.queue(Change::Restore).await;
    }

    /// Hands `change` to the page's sender and waits until it was sent.
    async fn queue(&self, change: Change) -> Result<(), Failure> {
        let (sent, outcome) = oneshot::channel();
        if self.changes.send(Queued { change, sent }).await.is_err() {
            return Err(Failure::Disconnected);
        }
        outcome.await.unwrap_or(Err(Failure::Disconnected))
    }

    /// The documents, for one step of bookkeeping.
    fn documents(&self) -> MutexGuard<'_, Documents> {
        locked(&self.documents)
    }
}

/// The page `driver` drives' own document.
fn page_key(driver: &PageDriver) -> Key {
    (driver.session().clone(), driver.main_frame().clone())
}

/// `documents`, for one step of bookkeeping.
fn locked(documents: &Mutex<Documents>) -> MutexGuard<'_, Documents> {
    documents.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Sender {
    /// Sends what `change` asks the browser for.
    async fn apply(&mut self, change: Change) -> Result<(), Failure> {
        let write = match change {
            Change::Set {
                key,
                part,
                text,
                order,
            } => {
                if let Some(order) = order {
                    let last = self.orders.entry((key.clone(), part)).or_insert(order);
                    if order < *last {
                        return Ok(());
                    }
                    *last = order;
                }
                self.documents().set(&key, part, text)
            }
            Change::Release { key, loader } => self.documents().release(&key, loader.as_deref()),
            Change::LiftConsent { key, selectors } => {
                self.documents().lift_consent(&key, &selectors)
            }
            Change::Lift { key } => {
                let writes = self.documents().lift(&key, &page_key(&self.driver));
                return self.send_each(writes).await;
            }
            Change::Restore => {
                let writes = self.documents().restore();
                return self.send_each(writes).await;
            }
        };
        match write {
            Some(write) => self.send(write).await,
            None => Ok(()),
        }
    }

    /// Sends each of `writes`, whatever the others did.
    async fn send_each(&self, writes: Vec<Write>) -> Result<(), Failure> {
        for write in writes {
            let _ = self.send(write).await;
        }
        Ok(())
    }

    /// The documents, for one step of bookkeeping.
    fn documents(&self) -> MutexGuard<'_, Documents> {
        locked(&self.documents)
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
