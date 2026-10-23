//! The main-frame navigations pending on a Session's pages (ADR 0121; spec
//! #577's second recognition rule): a request a service worker of the
//! Session sends to the URL, fragment removed, of one of them, in
//! `navigate` mode or any other, carries that navigation, so it takes the
//! navigation's permit from the page's navigation gate, once, and holds it
//! until the page's `load` event. A navigation the worker answers from its
//! own cache sends no request, so it takes none.
#![expect(
    clippy::disallowed_types,
    reason = "each lock guards the table for one operation, never across another lock or an `.await` (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use goodfetch_protocol::page::PageId;

use super::lock;
use crate::page_value::Permit;

/// A Session's pending navigations, by page. Each page's pipeline records
/// its own; its service workers' pipelines take them.
#[derive(Default)]
pub(super) struct Pending(Mutex<BTreeMap<PageId, Entry>>);

struct Entry {
    /// The navigation's URL, fragment removed.
    url: String,
    /// Whether a worker's request took it already.
    taken: bool,
    /// The permit it took, held until the page's `load`.
    permit: Option<Permit>,
}

/// `url` without its fragment.
fn without_fragment(url: &str) -> &str {
    url.split_once('#').map_or(url, |(url, _)| url)
}

impl Pending {
    /// The main frame of `page` started a navigation to `url`: one before
    /// it ended, its permit released.
    pub(super) fn started(&self, page: &PageId, url: &str) {
        let entry = Entry {
            url: without_fragment(url).to_owned(),
            taken: false,
            permit: None,
        };
        let before = lock(&self.0).insert(page.clone(), entry);
        drop(before);
    }

    /// The pending navigation of `page` ended with the page's `load`: its
    /// permit is released.
    pub(super) fn ended(&self, page: &PageId) {
        let ended = lock(&self.0).remove(page);
        drop(ended);
    }

    /// Takes the pending navigation that a request to `url` carries, once:
    /// the page it is pending on.
    pub(super) fn take(&self, url: &str) -> Option<PageId> {
        let url = without_fragment(url);
        let mut pending = lock(&self.0);
        let (page, entry) = pending
            .iter_mut()
            .find(|(_, entry)| !entry.taken && entry.url == url)?;
        entry.taken = true;
        Some(page.clone())
    }

    /// Holds `permit` for the pending navigation of `page` until its
    /// `load`; released at once when it is no longer pending.
    pub(super) fn hold(&self, page: &PageId, permit: Permit) {
        let mut pending = lock(&self.0);
        match pending.get_mut(page) {
            Some(entry) => entry.permit = Some(permit),
            None => {
                drop(pending);
                drop(permit);
            }
        }
    }
}
