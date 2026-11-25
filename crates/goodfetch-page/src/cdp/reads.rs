//! The primitives a Page's reads send (ADR 0146): a world made or found in
//! a frame's document, a function called there, a page function's
//! evaluation in its own world or in the main world, the page's document
//! serialized once and its URL, and text inserted where the focused
//! element's selection is.

use bytestring::ByteString;
use goodfetch_cdp::generated::{dom, input, page};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use serde_json::Value;

use super::CdpPage;
use super::driver::{GUARD_WORLD, world};
use super::functions::{evaluation, page_function_world};
use super::paused::NotTaken;
use super::runtime_values::returned;
use crate::errors::Call;
use crate::page_functions::Returned;

/// A page's document as it stands, serialized once.
pub(crate) struct SerializedDocument {
    /// Its HTML.
    pub(crate) html: ByteString,
    /// Its URL.
    pub(crate) url: String,
    /// Its base URL.
    pub(crate) base_url: String,
}

/// What a function called in a world threw, as the browser words it.
pub(crate) struct Thrown(pub(crate) String);

impl NotTaken {
    /// Whether the command failed as the document it ran in went away, by
    /// a navigation.
    pub(crate) fn context_gone(&self) -> bool {
        matches!(
            self.0,
            goodfetch_cdp::Failure::Refused {
                refusal: goodfetch_cdp::Refusal::ContextGone,
                ..
            }
        )
    }
}

impl CdpPage {
    /// This page with each command it sends counted on `row` as well, as a
    /// guarded action's attempt counts its commands by its kind, a guard's
    /// check its own and a serialization its two (ADR 0140).
    pub(crate) fn counting(&self, row: Row) -> Self {
        Self {
            page: self.page.counting(row),
        }
    }

    /// The Counted work of the page's Run.
    pub(crate) fn work(&self) -> &CountedWork {
        self.page.work()
    }

    /// The page's main frame.
    pub(crate) fn main_frame(&self) -> &str {
        self.page.main_frame()
    }

    /// The Error code `not_taken` raises from `call`, naming how the
    /// browser ended when it was lost.
    pub(crate) async fn raised(&self, call: &Call, not_taken: &NotTaken) -> CallError {
        self.page.raised(call, &not_taken.0).await
    }

    /// Makes the guard's world in the current document of `frame`, one of
    /// the page's own frames, or finds the one there: answers its context.
    ///
    /// # Errors
    ///
    /// When the frame or its document went.
    pub(crate) async fn guard_world(&self, frame: &str) -> Result<i64, NotTaken> {
        self.isolated_world((self.page.session(), frame), GUARD_WORLD)
            .await
    }

    /// Calls `function`, a JavaScript function's source, with `argument` in
    /// the world whose context is `context`, on the page's own target, its
    /// promise awaited: answers its value, none for one the browser cannot
    /// return by value, or what it threw.
    ///
    /// # Errors
    ///
    /// When the world's document went, or the page's target.
    pub(crate) async fn call_in_world(
        &self,
        context: i64,
        function: &str,
        argument: Value,
    ) -> Result<Result<Option<Value>, Thrown>, NotTaken> {
        let called = self
            .page
            .command(&world::call(function, argument, context))
            .await?;
        Ok(match called.exception_details {
            Some(thrown) => Err(Thrown(thrown.text)),
            None => Ok(called.result.value),
        })
    }

    /// Makes the page functions' world in the main frame's current
    /// document, or finds the one there, within `call`'s deadline: answers
    /// its context.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    pub(crate) async fn page_function_world(&self, call: &Call) -> Result<i64, CallError> {
        let made = self
            .page
            .command_for(&page_function_world(self.page.main_frame().clone()), call)
            .await?;
        Ok(made.execution_context_id)
    }

    /// Evaluates `expression` in the world whose context is `world`, or in
    /// the main world of the page's main frame when none, its promise
    /// awaited and its value answered by value, for as long as it takes.
    ///
    /// # Errors
    ///
    /// When the world's document went, or the page's target.
    pub(crate) async fn evaluate(
        &self,
        expression: &str,
        world: Option<i64>,
    ) -> Result<Returned, NotTaken> {
        Ok(returned(
            self.page.command(&evaluation(expression, world)).await?,
        ))
    }

    /// The page's document as the DOM holds it now, serialized once, every
    /// shadow root, open or closed, as a `<template shadowrootmode>`, with
    /// its URL and base URL: two commands, within `call`'s deadline.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    pub(crate) async fn serialized_document(
        &self,
        call: &Call,
    ) -> Result<SerializedDocument, CallError> {
        let root = self
            .page
            .command_for(
                &dom::GetDocumentParams {
                    depth: Some(0),
                    pierce: None,
                },
                call,
            )
            .await?
            .root;
        let html = self
            .page
            .command_for(
                &dom::GetOuterHTMLParams {
                    node_id: Some(root.node_id),
                    backend_node_id: None,
                    object_id: None,
                    // Every shadow root, open or closed, serialized as a
                    // `<template shadowrootmode>`, so that extraction-side
                    // removal walks them (ADR 0047).
                    include_shadow_dom: Some(true),
                },
                call,
            )
            .await?
            .outer_html;
        let url = root.document_url.unwrap_or_default();
        Ok(SerializedDocument {
            html: ByteString::from(html),
            base_url: root.base_url.unwrap_or_else(|| url.clone()),
            url,
        })
    }

    /// The URL of the document the main frame holds: the one it could not
    /// reach when it shows the browser's error page.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    pub(crate) async fn document_url(&self, call: &Call) -> Result<String, CallError> {
        let tree = self
            .page
            .command_for(&page::GetFrameTreeParams {}, call)
            .await?;
        let frame = tree.frame_tree.frame;
        Ok(frame.unreachable_url.unwrap_or(frame.url))
    }

    /// Inserts `text` where the focused element's selection is, as an
    /// input method commits text: one trusted `input` event, within
    /// `call`'s deadline.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refuses.
    pub(crate) async fn insert_text(&self, text: &str, call: &Call) -> Result<(), CallError> {
        self.page
            .command_for(
                &input::InsertTextParams {
                    text: text.to_owned(),
                },
                call,
            )
            .await
            .map(drop)
    }
}
