//! The browser pool's rules (ADR 0040, SA-09), pure: which of a Run's
//! browsers a Session goes to, and when a browser is recycled. A Session
//! shares the first browser of its Run with room for one more browser
//! context, never one being recycled or one that holds a Session of a
//! per-process Fingerprint method, and gets a new browser otherwise; a
//! browser is recycled once it opened its pages, or, once it opened one,
//! past its memory cap: the absolute one when set, else twice what it used
//! once its first page had run. A browser holding a page under a person or a raw CDP attach is
//! recycled only once that page is released.

use std::time::Duration;

use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_protocol::governor::{
    BROWSER_CONTEXTS_PER_PROCESS, BROWSER_MEMORY_CAP, BROWSER_RECYCLE_IDLE, BROWSER_RECYCLE_PAGES,
};

/// What bounds a Run's browsers, from its settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// `governor.browser.contexts_per_process`.
    pub(crate) contexts_per_process: u64,
    /// `governor.browser.recycle_pages`.
    pub(crate) pages: u64,
    /// `governor.browser.recycle_idle`.
    pub(crate) idle: Duration,
    /// `governor.browser.memory_cap`, when set.
    pub(crate) memory_cap: Option<u64>,
}

impl Limits {
    /// The limits the Run's settings give, each key's catalog default when
    /// the Run has none.
    pub(crate) fn of(settings: Option<&Resolved>) -> Self {
        Self {
            contexts_per_process: get_or_default(settings, &BROWSER_CONTEXTS_PER_PROCESS),
            pages: get_or_default(settings, &BROWSER_RECYCLE_PAGES),
            idle: get_or_default(settings, &BROWSER_RECYCLE_IDLE),
            memory_cap: get_or_default(settings, &BROWSER_MEMORY_CAP),
        }
    }
}

/// What the pool knows of one browser when it decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Use {
    /// How many pages it opened.
    pub(crate) served: u64,
    /// The memory it and its processes use now, when measured.
    pub(crate) footprint: Option<u64>,
    /// What it used once its first page had run, when measured.
    pub(crate) warm_up: Option<u64>,
    /// Whether it holds a page in a Human pause, under a person in the
    /// Live view or under a raw CDP attach bound to its lease.
    pub(crate) held: bool,
}

/// Why a browser is recycled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recycle {
    /// It opened `governor.browser.recycle_pages` pages.
    Pages,
    /// It passed its memory cap.
    Memory,
}

impl Recycle {
    /// Its value of `goodfetch.browser.recycled`'s `reason` label.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pages => "pages",
            Self::Memory => "memory",
        }
    }
}

/// Whether `browser` is recycled under `limits`, and why.
pub(crate) fn recycle(browser: &Use, limits: &Limits) -> Option<Recycle> {
    if browser.held {
        return None;
    }
    if browser.served >= limits.pages {
        return Some(Recycle::Pages);
    }
    // A browser that opened no page has not grown: a replacement would be
    // past the same cap.
    if browser.served == 0 {
        return None;
    }
    let cap = limits
        .memory_cap
        .or_else(|| browser.warm_up.map(|warm_up| warm_up.saturating_mul(2)))?;
    browser
        .footprint
        .filter(|footprint| *footprint > cap)
        .map(|_| Recycle::Memory)
}

/// Where a browser of the pool is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// It opens pages.
    Serving,
    /// It is recycled: it closes once none is open, and opens a page only
    /// for a Session a move would race ([`leaves`]).
    Retiring,
    /// It closes.
    Closing,
}

/// The page a Session asks its browser to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Opens {
    /// A page a program opens.
    ProgramPage,
    /// A page the engine opens for itself; it `races` when a move of the
    /// Session now could capture its storage before a program's navigation
    /// wrote it.
    EnginePage { races: bool },
}

/// Whether a Session leaves its browser, at `stage` once judged, before it
/// opens the page `opens`. It leaves a recycled browser unless a move would
/// race a program's navigation, when the page opens beside that navigation
/// and the Session's next page moves it; it always leaves one that closes,
/// which takes its context whether it moves or not.
pub(crate) fn leaves(opens: Opens, stage: Stage) -> bool {
    match (stage, opens) {
        (Stage::Serving, _) | (Stage::Retiring, Opens::EnginePage { races: true }) => false,
        (Stage::Retiring, Opens::ProgramPage | Opens::EnginePage { races: false })
        | (Stage::Closing, _) => true,
    }
}

/// Where a Session's browser context goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Into a browser it shares with the Run's other Sessions.
    Shared,
    /// Into a browser of its own, as a per-process Fingerprint method
    /// needs (ADR 0040).
    OwnProcess,
}

/// What the pool knows of one browser when it places a Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seats {
    /// The browser contexts of Sessions it holds.
    pub(crate) contexts: u64,
    /// Whether it is being recycled.
    pub(crate) retiring: bool,
    /// Whether it is a Session's own process.
    pub(crate) own: bool,
}

/// The browser, among `browsers` by their number in order, a Session goes
/// to under `placement`; `None` when it needs a new one.
pub(crate) fn place(
    browsers: impl IntoIterator<Item = (u64, Seats)>,
    placement: Placement,
    limits: &Limits,
) -> Option<u64> {
    if placement == Placement::OwnProcess {
        return None;
    }
    let room = limits.contexts_per_process.max(1);
    browsers
        .into_iter()
        .find(|(_, seats)| !seats.retiring && !seats.own && seats.contexts < room)
        .map(|(number, _)| number)
}

#[cfg(test)]
mod tests;
