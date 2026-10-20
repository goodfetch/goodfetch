//! The Pinned artifacts Collie's version pins (ADRs 0024 and 0098): the
//! builds its launches take from the Browser cache, which `browsers install`
//! fetches and whose entries the clean-up keeps. Foxhound's version pins the
//! same, since it launches what Collie launches.
//!
//! The list is empty until the launch of area 5 pins Chrome for Testing's
//! builds, with the digests of their archives and of their files.

use goodfetch_engine::browser_cache::PinnedArtifact;

/// Every artifact this version pins.
///
/// # Examples
///
/// ```
/// let pins = goodfetch_collie::pinned_artifacts();
/// assert!(pins.iter().all(|pin| pin.url.starts_with("https://")));
/// ```
pub fn pinned_artifacts() -> Vec<PinnedArtifact> {
    Vec::new()
}
