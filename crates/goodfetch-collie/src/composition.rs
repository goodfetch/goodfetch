//! Collie's composition (ADR 0087): the contribution of each crate Collie's
//! executable links, a list, in order. Foxhound's composition starts from it.

use std::sync::Arc;

use goodfetch_engine::Composition;
use goodfetch_page::page_value::{HandlerPages, ProgramPages};

use crate::block_detection::Detector;
use crate::{pins, sample};

/// What Collie's engine is composed of: the engine's own operations, then
/// the contribution of each Collie crate.
///
/// # Examples
///
/// ```
/// let composition = goodfetch_collie::composition();
///
/// assert!(composition.operations().any(|operation| operation == "engine.info"));
/// ```
pub fn composition() -> Composition {
    composition_with(ProgramPages)
}

/// What Collie's engine is composed of, as [`composition`], each page it
/// hands a handler set by `handler_pages` (ADR 0087): the seam through which
/// Foxhound's composition sets the pages of its crawl's handlers.
pub fn composition_with(handler_pages: impl HandlerPages + 'static) -> Composition {
    Composition::default()
        .with(goodfetch_engine::contribution())
        .with(sample::contribution())
        .with(goodfetch_secret_providers::contribution())
        .with(goodfetch_page::contribution_with(
            pins::pinned_artifacts(),
            handler_pages,
            Arc::new(Detector),
        ))
}

#[cfg(test)]
mod tests;
