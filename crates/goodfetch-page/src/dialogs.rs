//! A page's JavaScript dialogs (spec #577's `dialogs`; ADRs 0081 and 0085),
//! which its owner holds (ADR 0146): each open page has one reader of its
//! dialogs, which asks the page's dialog handlers about each one, the latest
//! armed first, through `dialog.handle`, and answers the browser with the
//! first `dialog.accept` or `dialog.dismiss` it gets. A handler that ends
//! without answering falls
//! through to the one armed before; past the last one, or with none, the
//! dialog gets Playwright's default: an `alert`, `confirm` or `prompt` is
//! dismissed, and a `beforeunload` dialog accepted, so a page never stalls
//! on a dialog no handler answers (pitfall B16). A handler whose reply is a
//! failure gives the dialog its default unless it answered already, is
//! removed, ends its subscription with `page_handler_failed`, and leaves
//! that failure for the call the dialog blocked, or else the page's next
//! call ([`PageDialogs::take_failure`]). Each dialog is named `dialog-<n>`,
//! numbered from 1 by the engine; once it is answered or closed, any later
//! answer raises `dialog_already_handled`.
//!
//! An open dialog blocks its page's input, and the gesture that opened it
//! waits for it to close: the reader answers dialogs beside every call, in
//! no page's turn (ADR 0085). A dialog of an out-of-process frame is its
//! frame target's, which no reader follows yet.
#![expect(
    clippy::disallowed_types,
    reason = "handlers are armed and answers delivered as their call arrives, before any `.await`: each lock guards one page's handlers or the slots for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_engine::CallError;
use goodfetch_engine::client::{Client, ClientRequestError, Sent};
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::dialog::{
    DIALOG_ALREADY_HANDLED, DIALOG_HANDLE, Dialog, DialogAlreadyHandledDetails, DialogHandleParams,
    DialogHandled, DialogId, DialogType, DialogsEnded,
};
use goodfetch_protocol::page::route::{PAGE_HANDLER_FAILED, PageHandler, PageHandlerFailedDetails};
use tokio::sync::{mpsc, oneshot, watch};

use crate::cdp::{CdpPage, DialogEvent, DialogEvents, OpenedDialog};
use crate::errors::page_closed;

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An answer to a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Accept it, a prompt with this text, or else its default value.
    Accept(Option<String>),
    /// Dismiss it.
    Dismiss,
}

/// An answer on its way to its dialog, with where its reply goes.
type Delivered = (Answer, oneshot::Sender<Result<(), CallError>>);

/// The open dialogs' slots, by the number the engine gives each, which
/// every answer reaches as its call arrives. Clones share them.
#[derive(Clone, Default)]
pub(crate) struct Dialogs {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// The last dialog number given.
    given: AtomicU64,
    /// Each open dialog's slot, by its number.
    slots: Mutex<BTreeMap<u64, mpsc::Sender<Delivered>>>,
}

/// One page's dialog handlers, which a call arms as it arrives and its
/// reader asks. Clones share them.
#[derive(Clone)]
pub(crate) struct PageDialogs {
    page: Arc<OfPage>,
}

struct OfPage {
    page_id: PageId,
    /// Its handlers, in the order they were armed.
    handlers: Mutex<Vec<Arc<Handler>>>,
    /// The first failure a handler left for the page's next call.
    failure: Mutex<Option<CallError>>,
    closed: watch::Sender<bool>,
}

/// One dialog handler: the Client that answers and the call whose
/// subscription it is.
struct Handler {
    client: Client,
    call: RequestId,
    /// Where its failure goes, once: its subscription's end.
    failed: Mutex<Option<oneshot::Sender<CallError>>>,
}

impl Handler {
    fn fail(&self, error: CallError) {
        if let Some(failed) = lock(&self.failed).take() {
            let _ = failed.send(error);
        }
    }
}

impl PageDialogs {
    /// Arms a dialog handler on the page, answered by `client` for its call
    /// `call`, as the latest; `closed` when the page is.
    pub(crate) fn arm(&self, client: &Client, call: &RequestId) -> Result<Armed, CallError> {
        let (failed, ended) = oneshot::channel();
        let handler = Arc::new(Handler {
            client: client.clone(),
            call: call.clone(),
            failed: Mutex::new(Some(failed)),
        });
        let closed = self.page.closed.subscribe();
        {
            // A close tells `closed` first, so one this arming does not see
            // clears the handlers after it.
            let mut handlers = lock(&self.page.handlers);
            if *closed.borrow() {
                return Err(page_closed(&self.page.page_id));
            }
            handlers.push(Arc::clone(&handler));
        }
        Ok(Armed {
            dialogs: self.clone(),
            handler,
            ended,
            closed,
        })
    }

    /// The failure a dialog handler left on the page for its next call;
    /// none once the page is closed.
    pub(crate) fn take_failure(&self) -> Option<CallError> {
        if *self.page.closed.borrow() {
            return None;
        }
        lock(&self.page.failure).take()
    }

    /// The page goes: its handlers' subscriptions end.
    pub(crate) fn close(&self) {
        self.page.closed.send_replace(true);
        lock(&self.page.handlers).clear();
    }

    fn latest_first(&self) -> Vec<Arc<Handler>> {
        lock(&self.page.handlers).iter().rev().cloned().collect()
    }

    fn remove(&self, handler: &Arc<Handler>) {
        lock(&self.page.handlers).retain(|armed| !Arc::ptr_eq(armed, handler));
    }

    /// `handler` failed on a dialog of the document at `url`: it goes, its
    /// subscription ends, and its failure waits for the page's next call.
    fn handler_failed(&self, handler: &Arc<Handler>, url: &str, message: &str) {
        handler.fail(handler_failed(url, message));
        self.remove(handler);
        lock(&self.page.failure).get_or_insert(handler_failed(url, message));
    }
}

/// An armed dialog handler, as its subscription holds it: dropped, it takes
/// the handler out of its page's.
pub(crate) struct Armed {
    dialogs: PageDialogs,
    handler: Arc<Handler>,
    ended: oneshot::Receiver<CallError>,
    closed: watch::Receiver<bool>,
}

impl Armed {
    /// Waits for the subscription's end: its handler's failure, or its page
    /// closing.
    ///
    /// # Errors
    ///
    /// The handler's failure, `page_handler_failed`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future, as a `$/cancel` does, removes the handler.
    pub(crate) async fn ended(mut self) -> Result<DialogsEnded, CallError> {
        tokio::select! {
            failed = &mut self.ended => match failed {
                Ok(error) => Err(error),
                Err(_) => Ok(DialogsEnded {}),
            },
            _ = self.closed.wait_for(|closed| *closed) => Ok(DialogsEnded {}),
        }
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        self.dialogs.remove(&self.handler);
    }
}

/// One open dialog's slot, which its reader takes answers from; dropped,
/// it closes.
struct Slot {
    number: u64,
    answers: mpsc::Receiver<Delivered>,
    shared: Arc<Shared>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        lock(&self.shared.slots).remove(&self.number);
    }
}

impl Dialogs {
    /// Starts answering, on a task of `run`, the dialogs of the page
    /// `page_id`, which `cdp` reaches, from now.
    pub(crate) fn start(&self, page_id: PageId, cdp: CdpPage, run: &RunIdentity) -> PageDialogs {
        let dialogs = PageDialogs {
            page: Arc::new(OfPage {
                page_id,
                handlers: Mutex::new(Vec::new()),
                failure: Mutex::new(None),
                closed: watch::Sender::new(false),
            }),
        };
        let events = cdp.dialog_events();
        let reader = Reader {
            shared: Arc::clone(&self.shared),
            dialogs: dialogs.clone(),
            cdp,
            run: run.clone(),
        };
        run.spawn(reader.read(events));
        dialogs
    }

    /// Delivers `answer` to the dialog `dialog_id`, and answers once the
    /// page closed the dialog.
    ///
    /// # Errors
    ///
    /// `dialog_already_handled` when the dialog was answered or closed, or
    /// an answer to it waits already.
    pub(crate) fn answer(
        &self,
        dialog_id: &DialogId,
        answer: Answer,
    ) -> Result<impl Future<Output = Result<(), CallError>> + use<>, CallError> {
        let number = dialog_id
            .0
            .strip_prefix("dialog-")
            .and_then(|number| number.parse::<u64>().ok())
            .ok_or_else(|| already_handled(dialog_id))?;
        let (reply, replied) = oneshot::channel();
        let delivered = lock(&self.shared.slots)
            .get(&number)
            .is_some_and(|slot| slot.try_send((answer, reply)).is_ok());
        if !delivered {
            return Err(already_handled(dialog_id));
        }
        let dialog_id = dialog_id.clone();
        Ok(async move {
            replied
                .await
                .unwrap_or_else(|_| Err(already_handled(&dialog_id)))
        })
    }
}

/// One page's reader of its dialogs.
struct Reader {
    shared: Arc<Shared>,
    dialogs: PageDialogs,
    /// The primitives the page's dialogs are read and answered by.
    cdp: CdpPage,
    run: RunIdentity,
}

/// How asking one handler about a dialog ended.
enum Asked {
    /// It fell through, to the handler before.
    Through,
    /// The dialog is settled: answered, given its default, or closed.
    Settled,
    /// The handler failed before it answered: the dialog gets its default.
    Failed,
}

impl Reader {
    /// Reads the page's dialogs until the page leaves the engine or its
    /// target goes, answering each one it opens before the next.
    async fn read(self, mut events: DialogEvents) {
        let mut closed = self.dialogs.page.closed.subscribe();
        loop {
            let event = tokio::select! {
                biased;
                _ = closed.wait_for(|closed| *closed) => return,
                event = events.next() => event,
            };
            match event {
                Some(DialogEvent::Opened(opened)) => self.answer_dialog(&opened, &mut events).await,
                Some(DialogEvent::Closed) => {}
                None => return,
            }
        }
    }

    /// Asks the page's handlers about the dialog `opened`, the latest
    /// first, and answers the browser with the first answer, or the
    /// dialog's default.
    async fn answer_dialog(&self, opened: &OpenedDialog, events: &mut DialogEvents) {
        for handler in self.dialogs.latest_first() {
            match self.ask(&handler, opened, events).await {
                Asked::Through => {}
                Asked::Settled => return,
                Asked::Failed => break,
            }
        }
        let default = if opened.dialog.kind == DialogType::BeforeUnload {
            Answer::Accept(None)
        } else {
            Answer::Dismiss
        };
        let _ = self.close_dialog(&opened.dialog, default).await;
    }

    /// Asks `handler` about the dialog `opened`, and acts on its first
    /// answer.
    async fn ask(
        &self,
        handler: &Arc<Handler>,
        opened: &OpenedDialog,
        events: &mut DialogEvents,
    ) -> Asked {
        let number = self.shared.given.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, answers) = mpsc::channel(1);
        lock(&self.shared.slots).insert(number, sender);
        let mut slot = Slot {
            number,
            answers,
            shared: Arc::clone(&self.shared),
        };
        let params = DialogHandleParams {
            id: handler.call.clone(),
            dialog_id: DialogId(format!("dialog-{number}")),
            page_id: self.dialogs.page.page_id.clone(),
            dialog: opened.dialog.clone(),
        };
        let Ok(mut sent) = handler
            .client
            .send::<DialogHandled>(&DIALOG_HANDLE, &params)
            .await
        else {
            // Its Connection ended: no handler answers there any more.
            self.dialogs.remove(handler);
            return Asked::Through;
        };
        loop {
            tokio::select! {
                biased;
                Some((answer, reply)) = slot.answers.recv() => {
                    drop(slot);
                    let _ = reply.send(self.close_dialog(&opened.dialog, answer).await);
                    self.watch(handler, &opened.url, sent);
                    return Asked::Settled;
                }
                ended = sent.answer() => {
                    return match ended {
                        Err(ClientRequestError::Failed { message, .. }) => {
                            self.dialogs.handler_failed(handler, &opened.url, &message);
                            Asked::Failed
                        }
                        Err(ClientRequestError::Gone) => {
                            self.dialogs.remove(handler);
                            Asked::Through
                        }
                        // Ended without an answer: it falls through.
                        Ok(DialogHandled {}) => Asked::Through,
                        Err(error) => {
                            tracing::warn!(%error, "a dialog handler's reply was not read, so it falls through");
                            Asked::Through
                        }
                    };
                }
                event = events.next() => match event {
                    // The page closed the dialog itself, as a navigation
                    // does, or its target went: no answer is due.
                    Some(DialogEvent::Closed) | None => return Asked::Settled,
                    Some(DialogEvent::Opened(_)) => {}
                },
            }
        }
    }

    /// Watches the handler that answered until it ends: one that fails
    /// then still loses its handler and leaves its failure.
    fn watch(&self, handler: &Arc<Handler>, url: &str, mut sent: Sent<DialogHandled>) {
        let (handler, dialogs, url) = (Arc::clone(handler), self.dialogs.clone(), url.to_owned());
        self.run.spawn(async move {
            if let Err(ClientRequestError::Failed { message, .. }) = sent.answer().await {
                dialogs.handler_failed(&handler, &url, &message);
            }
        });
    }

    /// Closes `dialog` with `answer`.
    async fn close_dialog(&self, dialog: &Dialog, answer: Answer) -> Result<(), CallError> {
        let (accept, prompt_text) = match answer {
            Answer::Accept(text) => (
                true,
                (dialog.kind == DialogType::Prompt)
                    .then(|| text.unwrap_or_else(|| dialog.default_value.clone())),
            ),
            Answer::Dismiss => (false, None),
        };
        self.cdp
            .answer_dialog(accept, prompt_text)
            .await
            .map_err(|failure| {
                tracing::debug!(%failure, "a dialog closed before its answer reached it");
                dialog_gone(&self.dialogs.page.page_id)
            })
    }
}

/// `page_handler_failed` for a dialog handler that failed on a dialog of
/// the document at `url`.
fn handler_failed(url: &str, message: &str) -> CallError {
    CallError::new(
        &PAGE_HANDLER_FAILED,
        format!("a dialog handler failed on a dialog of {url}: {message}"),
        &PageHandlerFailedDetails {
            handler: PageHandler::Dialog,
            url: url.to_owned(),
            failure: message.to_owned(),
        },
    )
}

/// `dialog_already_handled` for `dialog_id`.
fn already_handled(dialog_id: &DialogId) -> CallError {
    CallError::new(
        &DIALOG_ALREADY_HANDLED,
        format!(
            "the dialog `{}` was answered already, or it closed",
            dialog_id.0
        ),
        &DialogAlreadyHandledDetails {
            dialog_id: dialog_id.clone(),
        },
    )
}

/// The failure of an answer the page's dialog could no longer take.
fn dialog_gone(page_id: &PageId) -> CallError {
    CallError::new(
        &CLOSED,
        format!(
            "the dialog of the page `{}` closed before its answer reached it",
            page_id.0
        ),
        &serde_json::json!({ "page_id": page_id }),
    )
}
