//! The main-frame navigations pending on a Session's pages (ADR 0121; spec
//! #577's second recognition rule): a request a service worker of the
//! Session sends to the URL, fragment removed, of one of them, in
//! `navigate` mode or any other, carries that navigation, so it takes the
//! navigation's permit from the page's navigation gate, once, and holds it
//! until the page's `load` event or its end. The navigation stays its
//! page's: the wait for its permit is the page's, and its refusal raises
//! from the page's navigation call. A navigation the worker answers from
//! its own cache sends no request, so it takes none.
#![expect(
    clippy::disallowed_types,
    reason = "each lock guards the table for one operation, never across another lock or an `.await` (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Mutex, Weak};

use goodfetch_protocol::page::PageId;

use super::{PageState, lock};
use crate::driver::PageDriver;
use crate::page_events::Refusals;
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
    /// The interception state of the page it is pending on.
    page: Weak<PageState>,
}

/// What a page's navigations are admitted and refused through when a
/// service worker's request carries one: the page's driver, whose gate,
/// permit waits and navigation calls are the navigation's, and the
/// refusals its requests and waiters read.
pub(crate) struct NavigatingPage {
    pub(super) driver: PageDriver,
    pub(super) refusals: Refusals,
}

/// `url` without its fragment.
fn without_fragment(url: &str) -> &str {
    url.split_once('#').map_or(url, |(url, _)| url)
}

impl Pending {
    /// The main frame of `page`, which `navigating` is, started a
    /// navigation to `url`: one before it ended, its permit released.
    pub(super) fn started(&self, page: &PageId, url: &str, navigating: &Weak<PageState>) {
        let entry = Entry {
            url: without_fragment(url).to_owned(),
            taken: false,
            permit: None,
            page: Weak::clone(navigating),
        };
        let before = lock(&self.0).insert(page.clone(), entry);
        drop(before);
    }

    /// The pending navigation of `page` ended with the page's `load`, or
    /// with the page: its permit is released.
    pub(super) fn ended(&self, page: &PageId) {
        let ended = lock(&self.0).remove(page);
        drop(ended);
    }

    /// Takes the pending navigation that a request to `url` carries, once:
    /// the page it is pending on.
    pub(super) fn take(&self, url: &str) -> Option<(PageId, Weak<PageState>)> {
        let url = without_fragment(url);
        let mut pending = lock(&self.0);
        let (page, entry) = pending
            .iter_mut()
            .find(|(_, entry)| !entry.taken && entry.url == url)?;
        entry.taken = true;
        Some((page.clone(), Weak::clone(&entry.page)))
    }

    /// Holds `permit` for the pending navigation of `page` until its
    /// `load`; released at once when it is no longer pending.
    pub(super) fn hold(&self, page: &PageId, permit: Permit) {
        let mut pending = lock(&self.0);
        if let Some(entry) = pending.get_mut(page) {
            entry.permit = Some(permit);
        } else {
            drop(pending);
            drop(permit);
        }
    }
}
