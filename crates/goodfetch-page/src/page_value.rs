//! What Foxhound sets on the pages it hands to handlers (spec #577; ADRs
//! 0087, 0089, 0093, 0110, 0118 and 0121), through interfaces this crate
//! defines and Foxhound's crates fill, so neither reaches into the other.
//!
//! - **The page's value** ([`PageValue`]) holds exactly two parts: its
//!   [`NavigationGate`], which admits each main-frame navigation, each
//!   redirect or refresh hop, a service worker's request carrying a
//!   navigation and each `page.request` before it leaves, and its
//!   [`LeaseCheck`], asked before each outgoing browser or network command
//!   of the page. Both are empty on every page a program opens: nothing a
//!   Collie program does is refused, delayed or counted by them (its
//!   Session's proxy's limits alone hold its navigation, ADR 0088).
//! - **The response observer** ([`ResponseObserver`]), no part of the
//!   value, receives the responses the engine observes on a page, a
//!   service worker's included, whose URLs its patterns match, each body up
//!   to its byte cap (ADR 0110). A page that registers none keeps nothing.
//!
//! [`HandlerPages`] is the hook that gives a page both when it opens; a
//! Collie engine's gives every page none. The pages opened in a page's
//! Session, the engine's own page that sends its `page.request` among
//! them, take the value its Session's pages take.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::runs::Run;
use goodfetch_protocol::page::{PageId, SessionId, SitePreference};
use goodfetch_protocol::run::RunId;

use crate::url_pattern::UrlPattern;

/// A request the navigation gate is asked to admit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigation {
    /// Its URL, as it leaves.
    pub url: String,
    /// Its method.
    pub method: String,
    /// What it is.
    pub kind: NavigationKind,
}

/// What a request the navigation gate admits is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NavigationKind {
    /// The first request of a main-frame navigation, whatever started it:
    /// a call, a click, the page's own script, a history step, a reload, or
    /// a refresh stub's hop.
    Document,
    /// A redirect hop, which the 3xx before it started: of a main-frame
    /// navigation or of a `page.request`.
    Redirect,
    /// A service worker's request carrying a navigation of a page it
    /// controls (ADR 0121): in `navigate` mode, or to the URL, fragment
    /// removed, of a navigation still pending on such a page.
    ServiceWorker,
    /// The first request of a `page.request`.
    Request,
}

/// The right a navigation gate gives one admitted request to go on, held
/// until the request's hop ends: a 3xx or a refresh stub releases a hop's,
/// the response a `page.request`'s, and a page's `load` event its
/// navigation's last. Releasing it is dropping it.
#[must_use = "a permit is released when dropped"]
pub struct Permit {
    release: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl Permit {
    /// A permit that calls `release` once it is released.
    pub fn new(release: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }

    /// A permit with nothing to release, as an empty gate gives.
    pub fn unlimited() -> Self {
        Self { release: None }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl Debug for Permit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Permit")
            .field("releases", &self.release.is_some())
            .finish()
    }
}

/// Why a navigation gate refused a request or a response: it raises
/// `site_preferences_refused` with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The preference that refused it.
    pub reason: SitePreference,
    /// What the error's message says of it.
    pub message: String,
}

/// A document's response, as the navigation gate's second method judges
/// it: before the page renders any of it.
#[derive(Clone, Copy, Debug)]
pub struct DocumentResponse<'a> {
    /// The URL that answered.
    pub url: &'a str,
    /// Its status.
    pub status: u16,
    /// Its headers, as the browser gives them.
    pub headers: &'a [(String, String)],
    /// Its body, after any content coding, for a signal a `<meta>` carries.
    pub body: &'a [u8],
}

/// A boxed future the interfaces answer, so they stay object-safe.
pub type Admission<'a> = Pin<Box<dyn Future<Output = Result<Permit, Refusal>> + Send + 'a>>;

/// The navigation gate (ADRs 0087 and 0093): consulted at step 6 of the
/// interception, after the domain policy and the Filter lists, for each
/// request [`NavigationKind`] names. Time spent in [`NavigationGate::admit`]
/// counts against no timeout, and its page's [`LeaseCheck`] holds the lease
/// through it.
pub trait NavigationGate: Debug + Send + Sync {
    /// Admits `navigation` with a permit, or refuses it: a refused request
    /// never leaves the browser.
    fn admit<'a>(&'a self, navigation: &'a Navigation) -> Admission<'a>;

    /// Judges the document a main-frame navigation's response carries, for
    /// a signal the response itself sends (aipref's `Content-Usage` header,
    /// `TDMRep`'s headers and `<meta>`, RSL's `Link` header): a refusal
    /// raises from the call that asked for the navigation, and the page is
    /// left on a blank document.
    ///
    /// # Errors
    ///
    /// The refusal.
    fn judge_document(&self, response: &DocumentResponse<'_>) -> Result<(), Refusal>;
}

/// What a page carries for Foxhound (ADR 0087): its navigation gate and its
/// lease check, each empty unless Foxhound set it. Clones share them.
#[derive(Clone, Debug, Default)]
pub struct PageValue {
    gate: Option<Arc<dyn NavigationGate>>,
    lease: Option<Arc<dyn LeaseCheck>>,
}

impl PageValue {
    /// The value of a page a program opens: no gate, no lease check.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A value whose navigation gate is `gate` and lease check `lease`.
    pub fn new(gate: Arc<dyn NavigationGate>, lease: Arc<dyn LeaseCheck>) -> Self {
        Self {
            gate: Some(gate),
            lease: Some(lease),
        }
    }

    /// Its navigation gate, when it has one.
    pub(crate) fn gate(&self) -> Option<&Arc<dyn NavigationGate>> {
        self.gate.as_ref()
    }

    /// Its lease check, when it has one.
    pub(crate) fn lease(&self) -> Option<&Arc<dyn LeaseCheck>> {
        self.lease.as_ref()
    }
}

/// A response the engine observed on a page, or on a service worker of its
/// Session.
#[derive(Clone, Copy, Debug)]
pub struct ObservedResponse<'a> {
    /// The URL that answered.
    pub url: &'a str,
    /// Its status.
    pub status: u16,
    /// Its headers, as the browser gives them.
    pub headers: &'a [(String, String)],
    /// Whether a service worker's own request got it.
    pub from_service_worker: bool,
}

/// What a response observer keeps: the responses whose URLs its patterns
/// match, through the engine's one matcher (ADR 0081), each body up to its
/// byte cap.
#[derive(Clone, Debug)]
pub struct Kept {
    /// The URL patterns.
    pub urls: Vec<UrlPattern>,
    /// The most bytes of each body it receives.
    pub byte_cap: usize,
}

/// Foxhound's response observer (ADR 0110), which Foxhound registers through
/// [`HandlerPages`]; no part of the page's value.
pub trait ResponseObserver: Debug + Send + Sync {
    /// What it keeps, read once when its page opens.
    fn kept(&self) -> Kept;

    /// Receives `response` and its `body`, cut at [`Kept::byte_cap`].
    fn on_response(&self, response: &ObservedResponse<'_>, body: &[u8]);
}

/// A document a Product's fetch tier fetched without the browser, served
/// to a handler as a page (spec #579; ADR 0093): `page.formats` reads it,
/// and every other call on it answers `closed` until the tier that serves
/// or escalates it is built (#767).
#[derive(Clone)]
pub struct ServedDocument {
    /// The Run it was fetched for, on whose blocking owner it is read.
    pub run: Run,
    /// Its URL, after redirects.
    pub url: String,
    /// Its base URL.
    pub base_url: String,
    /// Its HTML, decoded.
    pub html: String,
}

impl Debug for ServedDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServedDocument")
            .field("run", self.run.id())
            .field("url", &self.url)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// The hook through which a Product sets what the pages it hands to
/// handlers carry, asked as each page of the Session `session` of the Run
/// `run` opens; every page of one Session takes the same.
pub trait HandlerPages: Send + Sync {
    /// The page's value.
    fn value(&self, run: &RunId, session: &SessionId) -> PageValue;

    /// The page's response observer, when one is registered.
    fn observer(&self, run: &RunId, session: &SessionId) -> Option<Arc<dyn ResponseObserver>>;

    /// The document the page `page_id` serves, when a fetch tier opened it
    /// from a document rather than in a browser; none by default.
    fn document(&self, page_id: &PageId) -> Option<ServedDocument> {
        let _ = page_id;
        None
    }
}

/// The hook of an engine that hands no pages to handlers, Collie's: every
/// page's value is empty and none registers an observer.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProgramPages;

impl HandlerPages for ProgramPages {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::empty()
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Arc<dyn ResponseObserver>> {
        None
    }
}
