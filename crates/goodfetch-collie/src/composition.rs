//! Collie's composition (ADR 0087): the contribution of each crate Collie's
//! executable links, a list, in order. Foxhound's composition starts from it.

use goodfetch_engine::Composition;

use crate::sample;

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
    Composition::default()
        .with(goodfetch_engine::contribution())
        .with(sample::contribution())
}

#[cfg(test)]
mod tests;
