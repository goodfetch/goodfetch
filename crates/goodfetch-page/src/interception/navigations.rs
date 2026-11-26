//! The main-frame navigations pending on a Session's pages (ADR 0121; spec
//! #577's second recognition rule): a request a service worker of the
//! Session sends to the URL, fragment removed, of one of them, in
//! `navigate` mode or any other, carries that navigation, so it takes the
//! navigation's permit from the page's navigation gate, once, and holds it
//! until the page's `load` event or its end. The navigation stays its
//! page's: the wait for its permit is the page's, and its refusal raises
//! from the page's navigation call. A navigation the worker answers from
//! its own cache sends no request, so it takes none.
//!
//! Each page holds its own pending navigation, in its side of the
//! interception, which its owner holds (ADR 0146); the Session lists its
//! pages, which its service workers' pipelines search.
#![expect(
    clippy::disallowed_types,
    reason = "each lock guards one list or one page's navigation for one operation, never across another lock or an `.await` (ADR 0060)"
)]

use std::sync::{Arc, Mutex, Weak};

use super::{PageState, lock};
use crate::driver::PageDriver;
use crate::page_events::Refusals;
use crate::page_value::Permit;

/// A Session's pages, whose pending navigations its service workers'
/// pipelines take: each page's own pipeline records its own.
#[derive(Default)]
pub(super) struct Pending(Mutex<Vec<Weak<PageState>>>);

/// One page's main-frame navigation, while one is pending.
#[derive(Default)]
pub(super) struct PendingNavigation(Mutex<Option<Entry>>);

struct Entry {
    /// The navigation's URL, fragment removed.
    url: String,
    /// Whether a worker's request took it already.
    taken: bool,
    /// The permit it took, held until the page's `load`.
    permit: Option<Permit>,
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
    /// Lists `page`, one of the Session's pages, whose navigations its
    /// service workers' requests may carry; a page gone leaves the list.
    pub(super) fn join(&self, page: &Arc<PageState>) {
        let mut pages = lock(&self.0);
        pages.retain(|page| page.strong_count() > 0);
        pages.push(Arc::downgrade(page));
    }

    /// The Session's pages that are still held.
    fn pages(&self) -> Vec<Arc<PageState>> {
        lock(&self.0).iter().filter_map(Weak::upgrade).collect()
    }

    /// Whether any navigation is pending.
    pub(super) fn any(&self) -> bool {
        self.pages()
            .iter()
            .any(|page| lock(&page.navigation.0).is_some())
    }

    /// Takes the pending navigation that a request to `url` carries, once:
    /// the page it is pending on.
    pub(super) fn take(&self, url: &str) -> Option<Arc<PageState>> {
        let url = without_fragment(url);
        self.pages()
            .into_iter()
            .find(|page| page.navigation.take(url))
    }
}

impl PendingNavigation {
    /// The page's main frame started a navigation to `url`: one before it
    /// ended, its permit released.
    pub(super) fn started(&self, url: &str) {
        let entry = Entry {
            url: without_fragment(url).to_owned(),
            taken: false,
            permit: None,
        };
        let before = lock(&self.0).replace(entry);
        drop(before);
    }

    /// The pending navigation ended with the page's `load`, or with the
    /// page: its permit is released.
    pub(super) fn ended(&self) {
        let ended = lock(&self.0).take();
        drop(ended);
    }

    /// Takes the pending navigation when a request to `url`, fragment
    /// removed, carries it and no request took it before: whether it did.
    fn take(&self, url: &str) -> bool {
        let mut pending = lock(&self.0);
        match pending.as_mut() {
            Some(entry) if !entry.taken && entry.url == url => {
                entry.taken = true;
                true
            }
            _ => false,
        }
    }

    /// Holds `permit` for the pending navigation until the page's `load`;
    /// released at once when it is no longer pending.
    pub(super) fn hold(&self, permit: Permit) {
        let mut pending = lock(&self.0);
        if let Some(entry) = pending.as_mut() {
            entry.permit = Some(permit);
        } else {
            drop(pending);
            drop(permit);
        }
    }
}
