//! Foxhound's composition (ADR 0087): Collie's, then the contribution of
//! each Foxhound crate its executable links, a list, in order.

use goodfetch_engine::Composition;
use goodfetch_fetch_tier::FetchTiers;

use crate::sample;

/// What Foxhound's engine is composed of: everything Collie's is, its pages
/// set by the fetch tiers' value (ADR 0087), then Foxhound's own
/// contributions, its crawl fetching on those tiers.
///
/// # Examples
///
/// ```
/// let composition = goodfetch_foxhound::composition();
///
/// assert!(composition.operations().any(|operation| operation == "sample.crawl"));
/// ```
pub fn composition() -> Composition {
    let tiers = FetchTiers::new();
    goodfetch_collie::composition_with(tiers.clone())
        .with(sample::contribution())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_session_pool::contribution())
        .with(goodfetch_dataset::contribution(goodfetch_crawl::url_of))
        .with(goodfetch_crawl::contribution(tiers))
        .with(goodfetch_sql_tab::contribution())
}

#[cfg(test)]
mod tests;
