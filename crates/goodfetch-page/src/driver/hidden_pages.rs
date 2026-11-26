//! A browser's hidden pages (`Target.createTarget`'s `hidden`, as an
//! engine page is) still open, by their browser context: Chromium keeps a
//! hidden page open past its context's disposal, its commands in flight
//! left unanswered, so a Session's close closes its own first. Each page
//! leaves the list as it closes. The list is the browser owner's own
//! (ADR 0146).
#![expect(
    clippy::disallowed_types,
    reason = "each lock is held for one map operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::generated::{browser, target};

/// Hidden pages by their browser context.
type ByContext = BTreeMap<browser::BrowserContextID, BTreeSet<target::TargetID>>;

/// A browser's hidden pages still open, by their browser context.
#[derive(Default)]
pub(super) struct HiddenPages(Mutex<ByContext>);

impl HiddenPages {
    /// Locks the list, which no panic leaves half-written.
    fn lock(&self) -> MutexGuard<'_, ByContext> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The page `page` opened hidden in the context `context`.
    pub(super) fn opened(&self, context: &browser::BrowserContextID, page: &target::TargetID) {
        self.lock()
            .entry(context.clone())
            .or_default()
            .insert(page.clone());
    }

    /// The page `page` of the context `context` closed, if it was hidden.
    pub(super) fn closed(&self, context: &browser::BrowserContextID, page: &target::TargetID) {
        let mut contexts = self.lock();
        if let Some(pages) = contexts.get_mut(context) {
            pages.remove(page);
            if pages.is_empty() {
                contexts.remove(context);
            }
        }
    }

    /// The hidden pages of the context `context` still open, which leave
    /// the list.
    pub(super) fn take(&self, context: &browser::BrowserContextID) -> BTreeSet<target::TargetID> {
        self.lock().remove(context).unwrap_or_default()
    }
}
