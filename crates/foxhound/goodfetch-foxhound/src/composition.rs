//! Foxhound's composition (ADR 0087): Collie's, then the contribution of
//! each Foxhound crate its executable links, a list, in order.

use goodfetch_engine::Composition;

use crate::sample;

/// What Foxhound's engine is composed of: everything Collie's is, then
/// Foxhound's own contributions.
///
/// # Examples
///
/// ```
/// let composition = goodfetch_foxhound::composition();
///
/// assert!(composition.operations().any(|operation| operation == "sample.crawl"));
/// ```
pub fn composition() -> Composition {
    goodfetch_collie::composition()
        .with(sample::contribution())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_crawl::contribution())
}

#[cfg(test)]
mod tests;
