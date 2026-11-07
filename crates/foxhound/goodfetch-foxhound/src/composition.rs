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
    composition_with(FetchTiers::new())
}

/// Foxhound's composition on the fetch tiers `tiers`, such as a test's or
/// a benchmark's that serve their pages from memory: what [`composition`]
/// is, on the tiers it is given.
///
/// # Examples
///
/// ```
/// use goodfetch_fetch_tier::FetchTiers;
///
/// let composition = goodfetch_foxhound::composition_with(FetchTiers::new());
///
/// assert!(composition.operations().any(|operation| operation == "crawl.start"));
/// ```
pub fn composition_with(tiers: FetchTiers) -> Composition {
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
