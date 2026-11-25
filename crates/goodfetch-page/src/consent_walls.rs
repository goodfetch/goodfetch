//! Consent walls (ADR 0048): who approved accepting one, the wall the
//! Consent prompt runner left in a page's main document, which the page's
//! next call raises as `blocked` with `consent_required`, and where the
//! runner's first detection pass on that document stands, which a call
//! waits on once it found a prompt. Each page's owner holds its own (ADR
//! 0146), so they sit apart from the runner, which acts on the page through
//! the guard and its owner. A domain's approval and the waits are tested
//! without a browser.
#![expect(
    clippy::disallowed_types,
    reason = "a page's walls' one lock guards one read or write, never held across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::needs_you::consent::ConsentDomains;
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::Value;
use tokio::sync::Notify;

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

/// What the page's runner left for its next call: its main document's
/// wall, and where its first pass there stands.
#[derive(Default)]
struct Page {
    wall: Option<Wall>,
    pass: Option<FirstPass>,
}

/// The wall a page's runner left for its next call, and the first pass
/// calls wait on. Clones share them.
#[derive(Clone, Default)]
pub(crate) struct Walls(Arc<Shared>);

#[derive(Default)]
struct Shared {
    page: Mutex<Page>,
    /// Woken whenever a first pass ends or a document goes.
    changed: Notify,
}

impl Walls {
    /// The page shows `wall`.
    pub(crate) fn put(&self, wall: Wall) {
        self.lock().wall = Some(wall);
    }

    /// The wall the page showed, taken, so it is raised once.
    pub(crate) fn take(&self) -> Option<Wall> {
        self.lock().wall.take()
    }

    /// The page's main frame holds a new document: the last one's wall and
    /// first pass go.
    pub(crate) fn new_document(&self) {
        *self.lock() = Page::default();
        self.0.changed.notify_waiters();
    }

    /// The runner's first pass on the page's main document found a prompt,
    /// unless it ended already.
    pub(crate) fn detected(&self) {
        self.lock().pass.get_or_insert(FirstPass::Detected);
    }

    /// The runner's first pass on the page's main document ended.
    pub(crate) fn settled(&self) {
        self.lock().pass = Some(FirstPass::Settled);
        self.0.changed.notify_waiters();
    }

    /// Whether the runner's first pass on the page's main document found a
    /// prompt whose verdict is still to come.
    pub(crate) fn pending(&self) -> bool {
        self.lock().pass == Some(FirstPass::Detected)
    }

    /// Waits, at most `limit` on `clock`, while the runner's first pass on
    /// the page's main document found a prompt and has not ended; whether
    /// it ended in time, or none was pending.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: it only waits.
    pub(crate) async fn verdict(&self, limit: Duration, clock: Clock) -> bool {
        let end = clock.now() + limit;
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.pending() {
                return true;
            }
            tokio::select! {
                () = changed => {}
                () = clock.sleep_until(end) => return !self.pending(),
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Page> {
        self.0.page.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
