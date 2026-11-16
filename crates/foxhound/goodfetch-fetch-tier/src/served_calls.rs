//! Which calls each fetch tier serves exactly (ADR 0093's open table, spec
//! #582): one rule, a call is served on a tier only when its result is
//! exact there; otherwise it escalates the request to the browser under
//! `fetch.tier: auto` and fails with `browser_required` under a forced
//! tier. The document tiers are `http`, `browser_fetch` and
//! `discovered_api`. A pure function of the call, the tier and whether the
//! tier was forced, which the served page consults for each call (the
//! `tiers` module's `calls`).
//!
//! | Row | Document tiers | `light_browser` | `browser` |
//! |---|---|---|---|
//! | [`Row::Read`] | served | served | served |
//! | [`Row::Question`] | from markup | from markup | served |
//! | [`Row::StateWait`] | when already true | when already true | served |
//! | [`Row::NetworkWait`] | `discovered_api` from replayed calls | escalates | served |
//! | [`Row::Script`] | escalates | escalates | served |
//! | [`Row::Layout`] | escalates | escalates | served |
//! | [`Row::Gesture`] | escalates | escalates | served |
//! | [`Row::Navigation`] | over the same tier | escalates | served |
//! | [`Row::Request`] | served | served | served |
//! | [`Row::Lifecycle`] | served | served | served |
//!
//! The `light_browser` tier's page is the document Lightpanda's run of the
//! page's scripts left (the `light_browser` module): its markup answers
//! what a document tier's does, a wait for scripted content included,
//! while a script of the handler's, a network wait and a navigation, which
//! would need a live page, escalate (#771's first build).
//!
//! On a page with no document (ADR 0119), a call of a row that
//! [reads the document](Row::reads_document) fails with
//! `response_not_document` instead, and never escalates.

use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::page::dialog::{DIALOG_ACCEPT, DIALOG_DISMISS, PAGE_ON_DIALOG};
use goodfetch_protocol::page::events::{PAGE_ON, PAGE_WAIT_FOR_REQUEST_FINISHED, RESPONSE_BODY};
use goodfetch_protocol::page::functions::{
    LOCATOR_EVALUATE_ALL, PAGE_ADD_INIT_SCRIPT, PAGE_EVALUATE,
};
use goodfetch_protocol::page::read::PAGE_READ;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::page::route::{
    PAGE_ROUTE, ROUTE_ABORT, ROUTE_CONTINUE, ROUTE_FALLBACK, ROUTE_FETCH, ROUTE_FULFILL,
    SESSION_ROUTE,
};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE};
use goodfetch_protocol::page::session_pages::{PAGE_CLOSE, SESSION_ON, SESSION_PAGES};
use goodfetch_protocol::page::waits::{
    LOCATOR_WAIT_FOR, LOCATOR_WAIT_FOR_COUNT, LOCATOR_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_FUNCTION,
    PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_REQUEST, PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL,
};
use goodfetch_protocol::page::{
    LOCATOR_CLICK, LOCATOR_FILL, LOCATOR_PRESS, PAGE_BODY, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, RUN_NEW_PAGE,
};

/// The row of each call the catalog's page entries hold.
const ROWS: [(&Operation, Row); 42] = [
    (&PAGE_FORMATS, Row::Read),
    (&PAGE_READ, Row::Read),
    (&LOCATOR_WAIT_FOR, Row::StateWait),
    (&LOCATOR_WAIT_FOR_COUNT, Row::StateWait),
    (&PAGE_WAIT_FOR_URL, Row::StateWait),
    (&PAGE_WAIT_FOR_LOAD_STATE, Row::StateWait),
    (&PAGE_WAIT_FOR_REQUEST, Row::NetworkWait),
    (&PAGE_WAIT_FOR_RESPONSE, Row::NetworkWait),
    (&PAGE_WAIT_FOR_REQUEST_FINISHED, Row::NetworkWait),
    (&PAGE_ON, Row::NetworkWait),
    (&SESSION_ON, Row::NetworkWait),
    (&PAGE_ROUTE, Row::NetworkWait),
    (&SESSION_ROUTE, Row::NetworkWait),
    (&ROUTE_CONTINUE, Row::NetworkWait),
    (&ROUTE_FALLBACK, Row::NetworkWait),
    (&ROUTE_FULFILL, Row::NetworkWait),
    (&ROUTE_ABORT, Row::NetworkWait),
    (&ROUTE_FETCH, Row::NetworkWait),
    (&PAGE_EVALUATE, Row::Script),
    (&PAGE_WAIT_FOR_FUNCTION, Row::Script),
    (&LOCATOR_EVALUATE_ALL, Row::Script),
    (&LOCATOR_WAIT_FOR_FUNCTION, Row::Script),
    (&PAGE_ADD_INIT_SCRIPT, Row::Script),
    (&LOCATOR_CLICK, Row::Gesture),
    (&LOCATOR_FILL, Row::Gesture),
    (&LOCATOR_PRESS, Row::Gesture),
    (&PAGE_ON_DIALOG, Row::Gesture),
    (&DIALOG_ACCEPT, Row::Gesture),
    (&DIALOG_DISMISS, Row::Gesture),
    (&PAGE_GOTO, Row::Navigation),
    (&PAGE_RELOAD, Row::Navigation),
    (&PAGE_GO_BACK, Row::Navigation),
    (&PAGE_GO_FORWARD, Row::Navigation),
    (&RUN_NEW_PAGE, Row::Navigation),
    (&SESSION_NEW_PAGE, Row::Navigation),
    (&PAGE_REQUEST, Row::Request),
    (&RESPONSE_BODY, Row::Request),
    (&PAGE_BODY, Row::Request),
    (&PAGE_CLOSE, Row::Lifecycle),
    (&SESSION_CLOSE, Row::Lifecycle),
    (&SESSION_PAGES, Row::Lifecycle),
    (&RUN_NEW_SESSION, Row::Lifecycle),
];

/// A row of the table: calls a tier serves alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no call of the catalog's page entries is a question or reads layout yet; their calls join with Collie's tickets (#1091)"
    )
)]
pub(crate) enum Row {
    /// Reads of the document's markup: locators, `count`, `all`,
    /// `textContent`, `innerHTML`, `getAttribute`, `content()`, `formats`
    /// and `read`.
    Read,
    /// Questions about an element's state: `isVisible`, `isHidden`,
    /// `isEnabled`, `isChecked`, answered at once and never waiting.
    Question,
    /// Waits for a state of the document: `waitFor`, `waitForURL`,
    /// `waitForLoadState`; on a document tier a static document never
    /// changes, so they are served when already true and escalate else.
    StateWait,
    /// The page's own network: waits for its requests and responses, its
    /// events and its routes, which only a tier that runs its scripts
    /// sees, or `discovered_api` from the calls it replays.
    NetworkWait,
    /// Scripts run in the page: `evaluate`, `waitForFunction`, init
    /// scripts.
    Script,
    /// Reads of layout or pixels: `innerText`, `boundingBox`,
    /// `ariaSnapshot`, `screenshot`, `pdf`, `mhtml`, `scrollCollect`.
    Layout,
    /// Gestures other than `goto` and `reload`, popups and dialogs,
    /// frames, `page.lend()`, acting AI verbs, `agent` and
    /// `enqueueLinksByClickingElements`.
    Gesture,
    /// `goto`, `reload`, `goBack`, `goForward` and a Session's new page.
    Navigation,
    /// `page.request`, its responses' bodies, and the reading AI verbs.
    Request,
    /// Closing a page or a Session and listing a Session's pages, which
    /// read nothing of the document.
    Lifecycle,
}

/// How a tier serves a row's calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Service {
    /// Exactly, as the browser does.
    Served,
    /// Answered at once from the document's markup: `hidden`, `inert`,
    /// `disabled`, `checked`, an inline `display:none` or
    /// `visibility:hidden`.
    FromMarkup,
    /// Answered at once when the document already meets it; a wait it does
    /// not meet escalates, since a static document never changes.
    WhenAlreadyTrue,
    /// Answered from the calls `discovered_api` replays; a wait none of
    /// them answers escalates.
    FromReplayedCalls,
    /// Fetched over the same tier through the navigation gate.
    OverTheSameTier,
    /// Not exactly: the call escalates, or fails with `browser_required`
    /// under a forced tier.
    Escalates,
}

/// What a call on a page of a tier leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The tier serves it this way.
    Serve(Service),
    /// Under `auto`: the request escalates to the browser.
    Escalate,
    /// Under a forced tier: the call fails with `browser_required`.
    BrowserRequired,
}

impl Row {
    /// The row of the call `operation`, an operation's name, such as
    /// `locator.wait_for`; `None` for an operation on no page.
    pub(crate) fn of(operation: &str) -> Option<Self> {
        ROWS.iter()
            .find(|(call, _)| call.name == operation)
            .map(|(_, row)| *row)
    }

    /// How `tier` serves this row's calls.
    pub(crate) fn on(self, tier: FetchTier) -> Service {
        let document = matches!(
            tier,
            FetchTier::Http | FetchTier::BrowserFetch | FetchTier::DiscoveredApi
        );
        let rendered = tier == FetchTier::LightBrowser;
        match self {
            Self::Question if rendered => Service::FromMarkup,
            Self::StateWait if rendered => Service::WhenAlreadyTrue,
            Self::NetworkWait | Self::Script | Self::Navigation if rendered => Service::Escalates,
            Self::Question if document => Service::FromMarkup,
            Self::StateWait if document => Service::WhenAlreadyTrue,
            Self::NetworkWait if tier == FetchTier::DiscoveredApi => Service::FromReplayedCalls,
            Self::NetworkWait | Self::Script if document => Service::Escalates,
            Self::Navigation if document => Service::OverTheSameTier,
            Self::Layout | Self::Gesture if tier != FetchTier::Browser => Service::Escalates,
            Self::Question
            | Self::StateWait
            | Self::NetworkWait
            | Self::Script
            | Self::Navigation
            | Self::Layout
            | Self::Gesture
            | Self::Read
            | Self::Request
            | Self::Lifecycle => Service::Served,
        }
    }

    /// Whether its calls read, wait on or act on the page's document, so
    /// fail with `response_not_document` on a page that has none.
    pub(crate) fn reads_document(self) -> bool {
        !matches!(
            self,
            Self::NetworkWait | Self::Navigation | Self::Request | Self::Lifecycle
        )
    }
}

/// What the call `operation` on a page of `tier` leads to, `forced` when
/// `fetch.tier` forces that tier; `None` for an operation on no page.
/// Whether `tier` serves the call `operation`, an operation's name, under
/// `auto`: `false` for one it escalates and for any name no row holds,
/// such as the Page check's or `fetch.encoding`'s escalation.
pub fn serves(operation: &str, tier: FetchTier) -> bool {
    matches!(verdict(operation, tier, false), Some(Verdict::Serve(_)))
}

pub(crate) fn verdict(operation: &str, tier: FetchTier, forced: bool) -> Option<Verdict> {
    Some(match Row::of(operation)?.on(tier) {
        Service::Escalates if forced => Verdict::BrowserRequired,
        Service::Escalates => Verdict::Escalate,
        service => Verdict::Serve(service),
    })
}

#[cfg(test)]
mod tests;
