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
//!   value, receives the responses the engine observes on a page, those a
//!   service worker answered included, whose URLs its patterns match, each
//!   body up to its byte cap (ADR 0110). A page that registers none keeps
//!   nothing.
//! - **The document reader** ([`DocumentReader`]), no part of the value
//!   either: the interception settings of ADR 0119. It reads each
//!   main-frame document's response before the browser renders it, and
//!   has it read as it came, in a charset it names, or not at all, the
//!   page then left on a blank document so nothing downloads; the
//!   navigation's response keeps the site's headers. A page with none, as
//!   every page a program opens, is read as the browser reads it.
//!
//! [`HandlerPages`] is the seam that gives a page both when it opens; a
//! Collie engine's gives every page none. The pages opened in a page's
//! Session, the engine's own page that sends its `page.request` among
//! them, take the value its Session's pages take.
//!
//! A page a fetch tier serves from a document ([`ServedDocument`]) is no
//! browser page: the seam answers each call on it that it answers itself
//! ([`HandlerPages::call`]), serving it, escalating it or refusing it (ADR
//! 0093), and the Product opens its handlers' browser pages through the
//! [`BrowserPages`] it is handed once.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallContext, CallError, GuardAnswer};
use goodfetch_extractor::BuiltIn;
use goodfetch_protocol::page::{
    FormatsRequest, Navigated, PageFormats, PageId, PageResponse, SessionId, SitePreference,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::settings::session_source::SessionSource;

use crate::blocks::Block;
use crate::url_pattern::UrlPattern;

/// A boxed future of [`BrowserPages`]'s.
pub type Opening<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The browser pages a Product opens for its handlers itself (ADR 0093's
/// browser tier), which [`HandlerPages::opened`] hands it once.
pub trait BrowserPages: Send + Sync {
    /// Opens a page in the Session of the live Run `run_id` that the
    /// Product's Session pool names `pooled.id`, its own browser context,
    /// opened at its first page from `pooled.source` (ADRs 0088 and 0093),
    /// for the call `context`, whose Connection's Client then reaches it,
    /// taking the value [`HandlerPages::value`] gives as it opens, and
    /// navigates it to `url` as `page.goto` does, through its navigation
    /// gate; answers its ID and the navigation's answer. A page whose
    /// navigation failed is closed again.
    ///
    /// # Errors
    ///
    /// Those of `run.new_session`, `run.new_page` and `page.goto`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the page opened closes it, its navigation
    /// with it; dropped before, the Session may stay open until its Run
    /// closes.
    fn open<'a>(
        &'a self,
        run_id: &'a RunId,
        pooled: &'a PooledSession,
        url: &'a str,
        context: &'a CallContext,
    ) -> Opening<'a, Result<(PageId, Navigated), CallError>>;

    /// Closes the page `page_id`, unless it is closed already.
    fn close<'a>(&'a self, page_id: &'a PageId) -> Opening<'a, ()>;

    /// Retires the Session of the Run `run_id` that the Product's Session
    /// pool names `pooled` (spec #594's `Retire`): its pages and browser
    /// context close, and the pool's next Session is a new one. One the
    /// pool never opened a page in, or one closing or closed already,
    /// changes nothing.
    ///
    /// # Errors
    ///
    /// `internal`, for a Session its table does not let the pool retire.
    fn retire<'a>(
        &'a self,
        run_id: &'a RunId,
        pooled: &'a SessionId,
        context: &'a CallContext,
    ) -> Opening<'a, Result<(), CallError>>;

    /// Sends `request` from inside the Session of the live Run `run_id`
    /// that the Product's Session pool names `pooled`, opened as
    /// [`BrowserPages::open`] opens it, through the browser-fetch tier
    /// (ADR 0020): from a page of its browser context the engine opens for
    /// it and closes after, so its TLS and HTTP/2 fingerprint and its
    /// cookies are the Session's own, as `page.request` sends one, but
    /// following no redirect: a 3xx is answered as it came. That page
    /// takes `value`, whose navigation gate the Product sets: one that
    /// gates each hop itself admits it at once.
    ///
    /// # Errors
    ///
    /// Those of `run.new_session` and of `page.request`'s browser-fetch
    /// tier: `egress_refused`, `navigation_failed` with the browser's
    /// network error, `deadline_exceeded` naming `timeout.navigation`, the
    /// Session's proxy's codes and `response_limit_exceeded`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the page it was sent from.
    fn fetch<'a>(
        &'a self,
        run_id: &'a RunId,
        pooled: &'a PooledSession,
        value: PageValue,
        request: &'a EngineFetch,
        context: &'a CallContext,
    ) -> Opening<'a, Result<EngineFetched, CallError>>;

    /// Calls the page function `function`, a JavaScript function's source,
    /// in the guard's isolated world of the main frame of the page
    /// `page_id` (ADR 0019), with `argument`, an object whose `cosmetic` the
    /// engine sets to the selectors of the cosmetic rules the page's live
    /// hiding applies now (ADR 0047), none without live hiding; answers its
    /// value, none when its document went or the function threw. A failure
    /// raises the code of the operation `operation`.
    ///
    /// # Errors
    ///
    /// `closed` for a page closed, and the browser's failures.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads.
    fn read<'a>(
        &'a self,
        page_id: &'a PageId,
        function: &'static str,
        argument: serde_json::Value,
        operation: &'static str,
    ) -> Opening<'a, Result<Option<serde_json::Value>, CallError>>;

    /// The formats `request` names of the page `page_id`, as `page.formats`
    /// answers them: a browser page's, or a page a fetch tier serves from a
    /// document ([`HandlerPages::document`]); what a code-free crawl reads
    /// of each page it fetches (ADR 0103).
    ///
    /// # Errors
    ///
    /// Those of `page.formats`, such as `closed` for a page closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads.
    fn formats<'a>(
        &'a self,
        page_id: &'a PageId,
        request: FormatsRequest,
    ) -> Opening<'a, Result<PageFormats, CallError>>;

    /// The Block cause of a document a fetch tier served, which `response`
    /// answered with the markup `markup`, as the Product's Block
    /// classification tells it (ADR 0035): the rules a browser page's
    /// navigation is judged by, so a crawl's HTTP tiers meet the same
    /// blocks as its browser (spec #600). None for a document no rule
    /// classifies.
    fn classify(&self, response: &PageResponse, markup: &str) -> Option<Block>;
}

/// A request a Product sends from inside a Session's browser through
/// [`BrowserPages::fetch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineFetch {
    /// Its URL.
    pub url: String,
    /// Its method; the browser's own `GET` when it is `GET`.
    pub method: String,
    /// Headers set over the browser's own, by name without regard to case.
    pub headers: Vec<(String, String)>,
    /// Its body, if any.
    pub body: Option<Vec<u8>>,
}

/// What [`BrowserPages::fetch`] answers: the response and its body, its
/// content codings decoded by the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineFetched {
    /// The URL that answered.
    pub url: String,
    /// Its status.
    pub status: u16,
    /// Its headers, their names lower-cased; a repeated one once per value.
    pub headers: Vec<(String, String)>,
    /// Its body.
    pub body: FetchedBody,
}

/// A body [`BrowserPages::fetch`] read, as it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchedBody {
    /// Whole: its bytes after any content coding.
    Whole(Vec<u8>),
    /// Broken off after `received` bytes of the `expected` its
    /// `Content-Length` declared (ADR 0118).
    Cut {
        /// The bytes that arrived.
        received: u64,
        /// What its `Content-Length` declared, if it declared one.
        expected: Option<u64>,
    },
}

/// A Session of a Product's Session pool, as a browser page opens in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PooledSession {
    /// The pool's ID for it.
    pub id: SessionId,
    /// The Session source it opens from: `None` for the Run's
    /// `session.source`, `Some(None)` for a fresh Session (ADR 0083).
    pub source: Option<Option<SessionSource>>,
}

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

/// The boxed future [`NavigationGate::sending`] answers.
pub type Sending<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// The navigation gate (ADRs 0087 and 0093): consulted at step 6 of the
/// interception, after the domain policy and the Filter lists, for each
/// request [`NavigationKind`] names. Time spent in [`NavigationGate::admit`]
/// counts against no timeout, and its page's [`LeaseCheck`] holds the lease
/// through it.
pub trait NavigationGate: Debug + Send + Sync {
    /// Admits `navigation` with a permit, or refuses it: a refused request
    /// never leaves the browser.
    fn admit<'a>(&'a self, navigation: &'a Navigation) -> Admission<'a>;

    /// Hears that a request to `url` it admitted left the browser: once the
    /// browser took the command that sends it on, and again once it took
    /// the Session's proxy credentials after a `407`, since the request
    /// goes only then (ADR 0086: a delay counts from a request's real
    /// send, never from its admission). The page waits for it before it
    /// acts on the request's next event.
    fn sending<'a>(&'a self, url: &'a str) -> Sending<'a>;

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
/// lease check, each empty unless Foxhound set it, the Page check its
/// crawl request's Label declares, which judges the page in place of its
/// Session's (ADR 0093), and the main-content step its Label's
/// `content.extractor` chooses, which `page.formats` reads with over its
/// Session's and its Run's (spec #580). Clones share them.
#[derive(Clone, Debug, Default)]
pub struct PageValue {
    gate: Option<Arc<dyn NavigationGate>>,
    lease: Option<Arc<dyn LeaseCheck>>,
    page_check: Option<PageCheck>,
    extractor: Option<BuiltIn>,
}

impl PageValue {
    /// The value of a page a program opens: no gate, no lease check.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A value whose navigation gate is `gate` and lease check `lease`;
    /// every page that takes it shares them.
    pub fn new(gate: impl NavigationGate + 'static, lease: impl LeaseCheck + 'static) -> Self {
        Self {
            gate: Some(Arc::new(gate)),
            lease: Some(Arc::new(lease)),
            page_check: None,
            extractor: None,
        }
    }

    /// The same value, its pages judged by `page_check` in place of their
    /// Session's Page check when it is given.
    #[must_use]
    pub fn with_page_check(mut self, page_check: Option<PageCheck>) -> Self {
        self.page_check = page_check;
        self
    }

    /// The Page check that judges its pages in place of their Session's,
    /// when it carries one.
    pub(crate) fn page_check(&self) -> Option<&PageCheck> {
        self.page_check.as_ref()
    }

    /// The same value, its pages' main content read with `extractor` over
    /// their Session's and their Run's `content.extractor` when it is
    /// given.
    #[must_use]
    pub fn with_extractor(mut self, extractor: Option<BuiltIn>) -> Self {
        self.extractor = extractor;
        self
    }

    /// The main-content step its pages read with over their Session's and
    /// their Run's, when it carries one.
    pub(crate) fn extractor(&self) -> Option<BuiltIn> {
        self.extractor
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

/// A response the engine observed on a page, one a service worker of its
/// Session answered included.
#[derive(Clone, Copy, Debug)]
pub struct ObservedResponse<'a> {
    /// The page whose request got it, so a page a handler opened in its
    /// Session is told from the handler's own (ADR 0110).
    pub page_id: &'a PageId,
    /// The URL that answered.
    pub url: &'a str,
    /// The length of its whole body, after any content coding: more than
    /// the observer receives when its byte cap cut it.
    pub body_length: usize,
    /// Its status.
    pub status: u16,
    /// Its headers, as the browser gives them.
    pub headers: &'a [(String, String)],
    /// Whether a service worker's fetch handler answered it, from its cache
    /// or its own fetch, as the browser marks it (`fromServiceWorker()`).
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

/// A registered [`ResponseObserver`], which every page of its Session
/// shares.
#[derive(Clone, Debug)]
pub struct Observer(Arc<dyn ResponseObserver>);

impl Observer {
    /// `observer`, registered.
    pub fn new(observer: impl ResponseObserver + 'static) -> Self {
        Self(Arc::new(observer))
    }

    /// What it keeps, read once when its page opens.
    pub(crate) fn kept(&self) -> Kept {
        self.0.kept()
    }

    /// Passes it `response` and its `body`, cut at [`Kept::byte_cap`].
    pub(crate) fn on_response(&self, response: &ObservedResponse<'_>, body: &[u8]) {
        self.0.on_response(response, body);
    }
}

/// A main-frame document's response on a handler's page, as its document
/// reader reads it at the interception, before the browser renders any of
/// it (ADR 0119).
#[derive(Clone, Copy, Debug)]
pub struct ReadDocument<'a> {
    /// The page.
    pub page_id: &'a PageId,
    /// The browser's ID of its request, as its `PageResponse` names it.
    pub request_id: &'a str,
    /// Whether it is the document of the crawl request the page was opened
    /// for, the first its navigation reached, rather than one of the
    /// handler's own navigations.
    pub own: bool,
    /// The URL that answered.
    pub url: &'a str,
    /// Its status.
    pub status: u16,
    /// Its headers, their names lower-cased, as the site sent them.
    pub headers: &'a [(String, String)],
}

/// How a document reader has the browser read a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// As it came.
    AsItCame,
    /// The reader needs its body, after any content coding, to say.
    Body,
    /// In the encoding this WHATWG name names: its `Content-Type`'s
    /// charset is set to it for the browser alone, the handler's response
    /// keeping the site's headers.
    Charset(String),
    /// Not at all: its navigation stops at the interception, the page left
    /// on a blank document, so nothing renders or downloads it; the
    /// handler's response keeps the site's status and headers.
    Stop,
}

/// Foxhound's document reader (ADR 0119), which Foxhound registers through
/// [`HandlerPages`]; no part of the page's value. It reads each main-frame
/// document's response of a page it was registered on, a redirect's
/// excepted.
pub trait DocumentReader: Debug + Send + Sync {
    /// How the browser reads `document`: asked first with no `body`, and
    /// once more with it when the first answer was [`Reading::Body`].
    fn read(&self, document: &ReadDocument<'_>, body: Option<&[u8]>) -> Reading;
}

/// The most responses whose site's headers a page keeps for the navigation
/// answering them (STYLE.md: every buffer is bounded).
const SITES_KEPT: usize = 8;

/// The site's own status and headers of a response a document reader had
/// read otherwise, kept for the navigation it answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SiteResponse {
    /// Its headers, their names lower-cased.
    pub(crate) headers: std::collections::BTreeMap<String, String>,
    /// Whether its navigation was stopped, the page holding no document.
    pub(crate) stopped: bool,
}

/// A registered [`DocumentReader`], as one page holds it: whether its next
/// document is its crawl request's own, and the site's responses its
/// navigations answer with.
#[derive(Clone, Debug)]
pub struct Reader {
    inner: Arc<dyn DocumentReader>,
    #[expect(
        clippy::disallowed_types,
        reason = "one value read or written in one step, never held across another lock or an `.await` (ADR 0060)"
    )]
    held: Arc<std::sync::Mutex<Held>>,
}

/// What a page's [`Reader`] holds between its documents.
#[derive(Debug, Default)]
struct Held {
    /// Whether the page's next document is its crawl request's own.
    own: bool,
    /// The site's own responses the reader had read otherwise, latest last.
    sites: std::collections::VecDeque<(String, SiteResponse)>,
}

impl Reader {
    /// `reader`, registered on one page.
    pub fn new(reader: impl DocumentReader + 'static) -> Self {
        Self {
            inner: Arc::new(reader),
            held: Arc::default(),
        }
    }

    /// The same reader, registered on another page, a popup of this one's.
    pub(crate) fn for_page(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            held: Arc::default(),
        }
    }

    /// What it holds, for one step.
    fn held(&self) -> std::sync::MutexGuard<'_, Held> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Marks the page's next document as its crawl request's own.
    pub(crate) fn expect_own(&self) {
        self.held().own = true;
    }

    /// Whether the page's document now read is its crawl request's own,
    /// which only its first is.
    pub(crate) fn take_own(&self) -> bool {
        std::mem::take(&mut self.held().own)
    }

    /// Asks the reader how the browser reads `document`.
    pub(crate) fn read(&self, document: &ReadDocument<'_>, body: Option<&[u8]>) -> Reading {
        self.inner.read(document, body)
    }

    /// Keeps `site`, the site's own response to the request `request_id`,
    /// for the navigation it answers.
    pub(crate) fn keep(&self, request_id: &str, site: SiteResponse) {
        let mut held = self.held();
        if held.sites.len() == SITES_KEPT {
            held.sites.pop_front();
        }
        held.sites.push_back((request_id.to_owned(), site));
    }

    /// Takes the site's own response to the request `request_id`, when the
    /// reader had it read otherwise.
    pub(crate) fn site(&self, request_id: &str) -> Option<SiteResponse> {
        let mut held = self.held();
        let at = held.sites.iter().position(|(id, _)| id == request_id)?;
        held.sites.remove(at).map(|(_, site)| site)
    }
}

/// A document a Product's fetch tier fetched without the browser, served
/// to a handler as a page (spec #579; ADR 0093): `page.formats` reads it,
/// and the Product answers every other call on it through
/// [`HandlerPages::call`].
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
    /// The main-content step its crawl request's Label chooses with
    /// `content.extractor`, over its Run's; none when the Label leaves it
    /// to the Run.
    pub extractor: Option<BuiltIn>,
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

/// The seam through which a Product sets what the pages it hands to
/// handlers carry, asked as each page of the Session `session` of the Run
/// `run` opens; every page of one Session takes the same.
pub trait HandlerPages: Send + Sync {
    /// The page's value.
    fn value(&self, run: &RunId, session: &SessionId) -> PageValue;

    /// The page's response observer, when one is registered.
    fn observer(&self, run: &RunId, session: &SessionId) -> Option<Observer>;

    /// The page's document reader, when one is registered: the
    /// interception settings ADR 0119 sets on a handler's page, no part of
    /// its value. None by default, as on every page a program opens.
    fn reader(&self, run: &RunId, session: &SessionId) -> Option<Reader> {
        let _ = (run, session);
        None
    }

    /// The document the page `page_id` serves, when a fetch tier opened it
    /// from a document rather than in a browser; none by default.
    fn document(&self, page_id: &PageId) -> Option<ServedDocument> {
        let _ = page_id;
        None
    }

    /// The answer to the call `operation`, its params `params`, on the
    /// page `page_id`, when a fetch tier serves that page from a document
    /// and answers the call itself: served, escalated or refused (ADR
    /// 0093); `None` lets the page crate answer it, as it answers
    /// `page.formats` from [`HandlerPages::document`]. None by default.
    fn call(
        &self,
        page_id: &PageId,
        operation: &'static str,
        params: &serde_json::Value,
        context: &CallContext,
    ) -> Option<GuardAnswer> {
        let _ = (page_id, operation, params, context);
        None
    }

    /// Takes the handle through which the Product opens browser pages for
    /// its handlers itself, given once as the engine is composed; ignored
    /// by default.
    fn opened(&self, pages: Arc<dyn BrowserPages>) {
        let _ = pages;
    }
}

/// The seam of an engine that hands no pages to handlers, Collie's: every
/// page's value is empty and none registers an observer.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProgramPages;

impl HandlerPages for ProgramPages {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::empty()
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        None
    }
}
