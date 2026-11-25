//! A Page's reads (ADR 0146): the engine's reads of a frame's document, a
//! page function's evaluation, the document's serialization and its URL,
//! and a Gesture's calls into the guard's world the Page keeps
//! ([`PageWorlds`]), each sent through the CDP module's primitives. A call
//! of the Page API reads once its Page's owner granted it ([`Reads`], from
//! `PageOwner::reads`), one round trip; the Page's own work, its
//! navigations' judgements and its parts' reads, reads as the owner's own.

use goodfetch_counted_work::Row;
use goodfetch_engine::CallError;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::functions::World;
use serde_json::Value;

use crate::cdp::{CdpPage, NotTaken, SerializedDocument};
use crate::errors::{Call, Unanswered};
use crate::gesture_order::GestureTurn;
use crate::page_functions::Returned;
use crate::page_worlds::PageWorlds;

/// The engine's reads of one Page's frames: the locator waits' checks,
/// `page.read`'s fields and a handler page's reads, granted by its owner,
/// or its own work's.
pub(crate) struct Reads {
    cdp: CdpPage,
}

impl Reads {
    /// The reads of the page `cdp` drives: a call's once its Page's owner
    /// granted them, or the Page's own work's, which needs no grant.
    pub(crate) fn of(cdp: CdpPage) -> Self {
        Self { cdp }
    }

    /// These reads with each command they send counted on `row` as well,
    /// as a guard's check counts its own and `page.read` its own (ADR 0140).
    pub(crate) fn counting(&self, row: Row) -> Self {
        Self {
            cdp: self.cdp.counting(row),
        }
    }

    /// Calls `function`, a JavaScript function's source, with `argument` in
    /// the guard's isolated world of the frame `frame`'s document, made or
    /// found by its name, outside any gesture's turn: the reads engine
    /// scripts make of a frame, such as live hiding's (ADR 0047). A
    /// function that throws answers none.
    ///
    /// The one way an engine read reaches a frame, so it counts C2's work
    /// (ADR 0140): each command it sends, and the world it makes, which,
    /// with no registry of each frame's document yet, it makes on every read
    /// and so counts as one past its document's first (X10).
    ///
    /// # Errors
    ///
    /// When the frame or its document went.
    pub(crate) async fn in_frame(
        &self,
        frame: &str,
        function: &str,
        argument: Value,
    ) -> Result<Option<Value>, NotTaken> {
        let work = self.cdp.work();
        for row in [
            Row::PageReadCommands,
            Row::PageReadDocumentWorlds,
            Row::PageReadWorldCreations,
        ] {
            work.count(row);
        }
        let made = self.cdp.guard_world(frame).await?;
        work.count(Row::PageReadCommands);
        Ok(self
            .cdp
            .call_in_world(made, function, argument)
            .await?
            .unwrap_or(None))
    }

    /// Calls `function` with `argument` in the guard's isolated world of the
    /// page's main frame, outside any gesture's turn, as
    /// [`in_frame`](Self::in_frame) does: the locator waits' checks. A
    /// document gone or not there yet, or a function that throws, answers
    /// none.
    ///
    /// # Errors
    ///
    /// The browser's failure as the code of the operation `operation`.
    pub(crate) async fn in_page(
        &self,
        function: &str,
        argument: Value,
        operation: &'static str,
    ) -> Result<Option<Value>, CallError> {
        match self
            .in_frame(self.cdp.main_frame(), function, argument)
            .await
        {
            Ok(value) => Ok(value),
            Err(not_taken) if not_taken.context_gone() => Ok(None),
            Err(not_taken) => Err(self
                .cdp
                .raised(&Call::Action { operation }, &not_taken)
                .await),
        }
    }
}

/// Evaluates `expression`, a page function's, in the main frame of the page
/// `cdp` drives: in its main world, or in the page functions' isolated
/// world, made or found by its name; its promise awaited and its value
/// answered by value, outside any gesture's turn (ADR 0085), for as long as
/// it takes, as Playwright's `evaluate` has no timeout. A document replaced
/// under it is tried once more in the next one.
///
/// # Errors
///
/// The browser's failure as the code of the operation `operation`.
pub(crate) async fn evaluate(
    cdp: &CdpPage,
    expression: &str,
    world: World,
    operation: &'static str,
) -> Result<Returned, CallError> {
    let call = &Call::Action { operation };
    let mut tries = 2;
    loop {
        tries -= 1;
        let context = match world {
            World::Main => None,
            World::Isolated => Some(cdp.page_function_world(call).await?),
        };
        match cdp.evaluate(expression, context).await {
            Ok(returned) => return Ok(returned),
            Err(not_taken) if not_taken.context_gone() && tries > 0 => {}
            Err(not_taken) => return Err(cdp.raised(call, &not_taken).await),
        }
    }
}

/// The document of the page `cdp` drives as the DOM holds it now,
/// serialized once, with its URL and base URL: two commands, each counted
/// on `C2.serialization_commands` (ADR 0140).
///
/// # Errors
///
/// The browser's failure as reading the document's code.
pub(crate) async fn document(cdp: &CdpPage) -> Result<SerializedDocument, CallError> {
    cdp.counting(Row::PageSerializationCommands)
        .serialized_document(&Call::Document)
        .await
}

/// The URL of the document the main frame of the page `cdp` drives holds:
/// the one it could not reach when it shows the browser's error page.
///
/// # Errors
///
/// The browser's failure as reading the document's code.
pub(crate) async fn document_url(cdp: &CdpPage) -> Result<String, CallError> {
    cdp.document_url(&Call::Document).await
}

/// A Gesture's turn on its Page, granted by the Page's owner: its commands,
/// sent while no other Gesture's are, and its calls into the guard's world
/// the Page keeps.
pub(crate) struct Turn {
    cdp: CdpPage,
    worlds: PageWorlds,
    granted: GestureTurn,
    call: Call,
}

impl Turn {
    /// The turn `granted` to the Gesture `operation` on the page `cdp`
    /// drives, whose worlds are `worlds`; its failures raise the
    /// operation's codes.
    pub(crate) fn granted(
        cdp: CdpPage,
        worlds: PageWorlds,
        operation: &'static str,
        granted: GestureTurn,
    ) -> Self {
        Self {
            cdp,
            worlds,
            granted,
            call: Call::Action { operation },
        }
    }

    /// Calls `function`, a JavaScript function's source, with `argument` in
    /// the guard's isolated world of the page's main frame, made on first
    /// use in each document, and answers its value.
    pub(crate) async fn call(
        &mut self,
        function: &str,
        argument: Value,
    ) -> Result<Value, Unanswered> {
        let context = self.guard_world().await?;
        self.called(function, argument, context).await
    }

    /// Calls the guard's check `check`, a JavaScript function's source, with
    /// `argument`, as [`call`](Self::call) does, counting the command it sends
    /// and the source it carries (ADR 0140's C3). The world it may make
    /// first is its document's, made once there, so no check's own.
    pub(crate) async fn check(
        &mut self,
        check: &str,
        argument: Value,
    ) -> Result<Value, Unanswered> {
        let context = self.guard_world().await?;
        let work = self.cdp.work();
        work.count(Row::ActionCheckCommands);
        work.count(Row::ActionScriptSends);
        self.called(check, argument, context).await
    }

    /// The call its commands are sent for, whose deadline bounds them and
    /// whose codes their failures raise.
    pub(crate) fn gesture_call(&self) -> &Call {
        &self.call
    }

    /// Lends the page's pointer between two of the gesture's checks
    /// ([`GestureTurn::lend_pointer`]).
    ///
    /// # Cancel safety
    ///
    /// That of [`GestureTurn::lend_pointer`].
    pub(crate) async fn lend_pointer(&mut self) {
        self.granted.lend_pointer().await;
    }

    /// Inserts `text` where the focused element's selection is, as an
    /// input method commits text: one trusted `input` event.
    ///
    /// # Errors
    ///
    /// The browser's failure as the Gesture's code.
    pub(crate) async fn insert_text(&mut self, text: &str) -> Result<(), CallError> {
        self.cdp.insert_text(text, &self.call).await
    }

    /// The context of the guard's world in the page's main frame, the one
    /// the Page keeps, or made and kept now, on first use in its document.
    async fn guard_world(&self) -> Result<i64, Unanswered> {
        if let Some(context) = self.worlds.guard() {
            return Ok(context);
        }
        match self.cdp.guard_world(self.cdp.main_frame()).await {
            Ok(context) => {
                self.worlds.keep_guard(context);
                Ok(context)
            }
            Err(not_taken) => Err(self.unanswered(&not_taken).await),
        }
    }

    /// Calls `function` with `argument` in the guard's world, whose context
    /// is `context`, and answers its value; a lost document is
    /// [`Unanswered::WorldGone`], its world no longer kept.
    async fn called(
        &self,
        function: &str,
        argument: Value,
        context: i64,
    ) -> Result<Value, Unanswered> {
        match self.cdp.call_in_world(context, function, argument).await {
            Ok(Ok(value)) => Ok(value.unwrap_or(Value::Null)),
            Ok(Err(thrown)) => Err(Unanswered::Failed(CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the guard's script threw in the page: {}", thrown.0),
            ))),
            Err(not_taken) => {
                let unanswered = self.unanswered(&not_taken).await;
                if matches!(unanswered, Unanswered::WorldGone) {
                    // Another command may have made the next document's.
                    self.worlds.forget_guard(context);
                }
                Err(unanswered)
            }
        }
    }

    /// Why a command of the guard's world did not answer, from what the
    /// browser said of it.
    async fn unanswered(&self, not_taken: &NotTaken) -> Unanswered {
        if not_taken.context_gone() {
            Unanswered::WorldGone
        } else {
            Unanswered::Failed(self.cdp.raised(&self.call, not_taken).await)
        }
    }
}
