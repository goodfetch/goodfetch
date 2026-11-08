//! Consent walls (ADR 0048): who approved accepting one, the walls the
//! runner left, which a page's next call raises as `blocked` with
//! `consent_required`, and where the runner's first detection pass on each
//! page's main document stands, which a call waits on once it found a
//! prompt. A domain's approval and the waits are tested without a browser.
#![expect(
    clippy::disallowed_types,
    reason = "the walls table's one lock guards one insert or removal, never held across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::needs_you::consent::ConsentDomains;
use tokio::sync::Notify;

use goodfetch_protocol::page::PageId;
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::Value;

use crate::interception::domain_key;

/// The approval to accept a consent wall: `consent.domains`, as the Run
/// resolved it, and the layer that set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Approval {
    /// The registrable domains, or `*`, written explicitly.
    domains: ConsentDomains,
    /// The layer whose value the Run resolved.
    pub(crate) layer: SettingsLayer,
}

impl Approval {
    /// The approval `value`, `consent.domains` as `layer` set it; none for
    /// an empty list, which approves nothing.
    pub(crate) fn of(value: &Value, layer: SettingsLayer) -> Option<Self> {
        let domains = ConsentDomains::of(value);
        (!domains.is_empty()).then_some(Self { domains, layer })
    }

    /// The domain of the document at `url`, as ADR 0004 keys it, when the
    /// approval covers it.
    pub(crate) fn covers(&self, url: &str) -> Option<String> {
        let domain = domain_key(url)?;
        self.domains.covers(&domain).then_some(domain)
    }
}

/// A consent wall the runner left in a page's main frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wall {
    /// The browser's ID of the load of the document it is in.
    pub(crate) loader: String,
    /// The wall test's rule that found it.
    pub(crate) rule: &'static str,
}

/// Where the runner's first detection pass on a page's main document
/// stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirstPass {
    /// It found a prompt, and its verdict is still to come.
    Detected,
    /// It ended.
    Settled,
}

/// What each page's runner left for its next call: its main document's
/// wall, and where its first pass there stands.
#[derive(Default)]
struct Pages {
    walls: BTreeMap<PageId, Wall>,
    passes: BTreeMap<PageId, FirstPass>,
}

/// The walls each page's runner left for its next call, and the first
/// passes calls wait on. Clones share them.
#[derive(Clone, Default)]
pub(crate) struct Walls(Arc<Shared>);

#[derive(Default)]
struct Shared {
    pages: Mutex<Pages>,
    /// Woken whenever a first pass ends or a document goes.
    changed: Notify,
}

impl Walls {
    /// The page `page` shows `wall`.
    pub(crate) fn put(&self, page: PageId, wall: Wall) {
        self.lock().walls.insert(page, wall);
    }

    /// The wall the page `page` showed, taken, so it is raised once.
    pub(crate) fn take(&self, page: &PageId) -> Option<Wall> {
        self.lock().walls.remove(page)
    }

    /// The page `page`'s main frame holds a new document: the last one's
    /// wall and first pass go.
    pub(crate) fn new_document(&self, page: &PageId) {
        let mut pages = self.lock();
        pages.walls.remove(page);
        pages.passes.remove(page);
        drop(pages);
        self.0.changed.notify_waiters();
    }

    /// The runner's first pass on the page `page`'s main document found a
    /// prompt, unless it ended already.
    pub(crate) fn detected(&self, page: &PageId) {
        self.lock()
            .passes
            .entry(page.clone())
            .or_insert(FirstPass::Detected);
    }

    /// The runner's first pass on the page `page`'s main document ended.
    pub(crate) fn settled(&self, page: &PageId) {
        self.lock().passes.insert(page.clone(), FirstPass::Settled);
        self.0.changed.notify_waiters();
    }

    /// Whether the runner's first pass on the page `page`'s main document
    /// found a prompt whose verdict is still to come.
    pub(crate) fn pending(&self, page: &PageId) -> bool {
        self.lock().passes.get(page) == Some(&FirstPass::Detected)
    }

    /// Waits, at most `limit` on `clock`, while the runner's first pass on
    /// the page `page`'s main document found a prompt and has not ended;
    /// whether it ended in time, or none was pending.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: it only waits.
    pub(crate) async fn verdict(&self, page: &PageId, limit: Duration, clock: Clock) -> bool {
        let end = clock.now() + limit;
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.pending(page) {
                return true;
            }
            tokio::select! {
                () = changed => {}
                () = clock.sleep_until(end) => return !self.pending(page),
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Pages> {
        self.0.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
