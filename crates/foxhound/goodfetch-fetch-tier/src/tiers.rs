//! The tiers that serve a crawl request (spec #579; ADRs 0020, 0087 and
//! 0093), and the crate's one composition value, [`FetchTiers`]: the seam
//! Foxhound's composition sets on Collie's pages, and the tiers its crawl
//! serves requests with. On the `http` tier, [`HttpTier`] fetches a
//! request's document over reqwest through the fetch driver, and
//! [`FetchTiers::serve`] opens a page served from it, which the handler
//! reads through `page.formats` and whose other calls the `calls` module
//! serves, escalates or refuses; on the `browser` tier,
//! [`FetchTiers::browser`] opens Collie's own page for the handler. The
//! tier each request takes is the `choice` module's; a forced tier not
//! built yet refuses the crawl's start.
//!
//! A served page's document is read in the encoding ADR 0119's order finds
//! (the `charset` module); the links a locator matches in it are read for
//! `enqueueLinks` with that encoding, in which their queries are
//! percent-encoded (ADR 0120). Its request's Label takes only the types its
//! `crawl.content_types` lists (the `content_types` module): another ends
//! the request `skipped` with `content_type`, its body unread, and an
//! accepted type that is no HTML or XHTML opens a page holding no
//! document, which the Frontier learns nothing from and the Page check
//! does not judge. Each page keeps its request's own response, whose body
//! `response.body` reads.
//!
//! Size: the crate's one composition value and the pages it serves, each
//! method a step of one served page's life, from its fetch on the tier
//! its Session takes to its close.
#![expect(
    clippy::disallowed_types,
    reason = "the served pages' table is read and written in one step, before any `.await` and never under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use bytestring::ByteString;
use encoding_rs::Encoding;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::ExceptionKind;
use goodfetch_engine::retry_after::{self, RetryAfter};
use goodfetch_engine::robots_txt::{Answer, MAX_SIZE};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::{Effective, Resolved, RunSettings, get_or_default};
use goodfetch_engine::tls::{
    CertificateExceptions, ClientCertificates, ExceptionUsed, PinBook, Trust,
};
use goodfetch_engine::{Answered, CallContext, CallError, GuardAnswer, ReadParams};
use goodfetch_extractor::BuiltIn;
use goodfetch_extractor::fields::Found;
use goodfetch_page::blocks::Block;
use goodfetch_page::document_request::{DocumentRequest, Unread};
use goodfetch_page::page_value::{
    BrowserPages, HandlerPages, KeptFrom, KeptTree, Observer, PageValue, PooledSession,
    ProgramPages, Reader, ServedDocument, ServedFormatsAhead,
};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::{CRAWL_CONTENT_TYPES, CRAWL_START, CrawlRequest};
use goodfetch_protocol::foxhound::fetch::{
    FETCH_ENCODING, FETCH_HEDGE_ENABLED, FETCH_LIGHT_BROWSER, FETCH_REDIRECT_MAX, FETCH_TIER,
    FETCH_TIER_RACE, FetchTier, REDIRECT_MAX_LIMIT,
};
use goodfetch_protocol::foxhound::frontier::CRAWL_REQUEST_LINKS;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::foxhound::provenance::{
    CertificateExceptionKind, CertificateExceptionMark,
};
use goodfetch_protocol::foxhound::provenance::{
    EncodingProvenance, EncodingSource, ForcedBy, TierChosen, TierProvenance,
};
use goodfetch_protocol::foxhound::report::BROWSER_VERSION_UNREAD;
use goodfetch_protocol::foxhound::session_pool::SESSION_POOL_MAX_SIZE;
use goodfetch_protocol::foxhound::settings::FETCH_BLOCK_CLASSIFICATION;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::proxy::{PROXY_KIND, PROXY_PROVIDER, PROXY_SERVER};
use goodfetch_protocol::page::read::PageField;
use goodfetch_protocol::page::{
    BLOCKED, BlockedDetails, BodyHandle, FormatsRequest, Navigated, PageFormats, PageId,
    PageResponse, SessionId,
};
use goodfetch_protocol::run::{RunId, RunWarning};
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SETTING_INVALID, SettingInvalidDetails,
    SettingsLayer,
};
use goodfetch_protocol::tls::{TLS_ACCEPT_INVALID_CERTS, TLS_CLIENT_CERTIFICATES};
use url::Url;

use crate::canonical::canonical;
use crate::chain::{
    Followed, Following, HopRead, Reading, Redirected, Skip, Stop, Unfollowed, follow_reading,
};
use crate::charset::{Kind, Read, Text};
use crate::choice::{
    ENCODING_CALL, LabelSettings, SetTier, TierChoice, Unserved, undeclared, unserved,
};
use crate::content_types::{Accepts, ContentTypes, is_document, typed};
use crate::cookies::{Jars, WithJar};
use crate::discovered_api::Plan;
use crate::document::{self, Matched, SelectorInvalid, matched};
use crate::driver::{FetchBounds, Hop, OwnDocument, Transport, fetch_accepting, fetch_start};
use crate::expected_page::{self, Verdict};
use crate::gate::{Admitted, ChainGate, Gate, HandOver, Lease, RobotsTxtFetch};
use crate::light_browser::LightBrowser;
use crate::proxied::SessionProxies;
use crate::redundant::{Redundant, Requester};
use crate::transports::{AddressCheck, BrowserFetch, LocalSource, Reqwest, TransportConfig};
use crate::untouched::{self, Passed, UntouchedReaders};
use crate::use_signals::UseSignals;

pub(crate) mod calls;
mod reading;
mod sitemap;
#[cfg(test)]
mod tests;

/// Foxhound's fetch tiers: the pages they serve, which Collie's page
/// crate reads through it as its [`HandlerPages`] seam, and
/// the tier each Run's crawl requests take. Cheap to clone; every clone
/// shares the pages.
#[derive(Clone, Default)]
pub struct FetchTiers {
    served: Arc<Served>,
}

#[derive(Default)]
struct Served {
    pages: Mutex<Pages>,
    /// What each Run's attempts handed over while their handlers run, in
    /// the order they did, each under its number.
    handed: Mutex<HandedOvers>,
    /// How the browser tier opens Collie's pages, which the page crate
    /// hands over once the engine is composed.
    browser: OnceLock<Arc<dyn BrowserPages>>,
    /// The light-browser tier's Lightpanda build, prepared at its first
    /// run (ADR 0020).
    light: LightBrowser,
    /// What every Run's HTTP tier sends its hops with in place of reqwest:
    /// the fetch tiers' test seam (ADR 0129's primitive), which a
    /// benchmark fills with pages served from memory (ADR 0134); `None` in
    /// every Product.
    transport: Option<Arc<dyn Transport>>,
}

#[derive(Default)]
struct HandedOvers {
    /// The number of the last hand-over.
    last: u64,
    by_run: BTreeMap<RunId, Vec<(u64, Arc<HandOver>)>>,
}

/// A Run's pages handed over by one attempt, taken back when dropped.
#[must_use = "the pages are taken back when this is dropped"]
pub struct HandedOver {
    served: Arc<Served>,
    run: RunId,
    number: u64,
}

impl Drop for HandedOver {
    fn drop(&mut self) {
        let mut handed = lock(&self.served.handed);
        if let Some(attempts) = handed.by_run.get_mut(&self.run) {
            attempts.retain(|(number, _)| *number != self.number);
            if attempts.is_empty() {
                handed.by_run.remove(&self.run);
            }
        }
    }
}

tokio::task_local! {
    /// The hand-over of the attempt whose own browser page
    /// [`FetchTiers::browser`] opens on this task: the page takes its
    /// value as it opens, in its Session, while the Run's other attempts
    /// hand its pages over at once.
    static OPENING: Arc<HandOver>;
}

#[derive(Default)]
struct Pages {
    /// The number of the next page served.
    next: u64,
    /// Each open page's state, shared with the calls that read it: a
    /// change after a call took it copies it, never a call (E1-5).
    open: BTreeMap<PageId, Arc<ServedPage>>,
    /// The `Link` header fields of each open page that had any, where
    /// its `rel=canonical` may be.
    links: BTreeMap<PageId, Vec<(String, String)>>,
    /// The AI-use signals each open page's response carried.
    use_signals: BTreeMap<PageId, UseSignals>,
    /// The encoding each open page's document was decoded in.
    encodings: BTreeMap<PageId, &'static Encoding>,
    /// What the browser tier's document reader learned of each browser
    /// page a handler holds (ADR 0119).
    browser: BTreeMap<PageId, reading::BrowserRead>,
    /// The plan each `discovered_api` page replays its calls by, and the
    /// waits armed on it (ADR 0020).
    replays: BTreeMap<PageId, Replaying>,
}

/// The calls a `discovered_api` page replays, and the waits armed for
/// them.
struct Replaying {
    plan: Plan,
    waiters: Vec<Waiter>,
}

/// A `page.wait_for_response` armed on a `discovered_api` page: answered
/// with the first replayed call its pattern matches, or told none came.
struct Waiter {
    pattern: goodfetch_page::url_pattern::UrlPattern,
    answer: tokio::sync::oneshot::Sender<Replayed>,
}

/// What a wait is told of the calls replayed for it.
#[derive(Clone)]
enum Replayed {
    /// The call its pattern matched.
    Answered(Box<goodfetch_protocol::page::events::PageExchange>),
    /// That call's answer passed a limit: `response_limit_exceeded`.
    PastALimit(goodfetch_engine::response_limits::LimitExceeded),
    /// None came, so it escalates.
    NoneCame,
}

/// Why a page's links could not be read.
#[derive(Debug)]
pub enum LinksUnread {
    /// The locator's selector is no CSS selector.
    Selector(SelectorInvalid),
    /// The browser page's read failed with this Error code.
    Page(CallError),
}

/// The elements a locator matched in a served page's document, with what
/// their links resolve against.
#[derive(Clone, Debug)]
pub struct ServedLinks {
    /// The URL the document was reached at.
    pub url: String,
    /// The document's base URL.
    pub base_url: String,
    /// The encoding it was decoded in.
    pub encoding: &'static Encoding,
    /// Each element matched, in document order.
    pub elements: Vec<Matched>,
}

/// A page served from a document: the document, the tier that fetched it
/// and how that tier was chosen, the call that escalated it, if one did,
/// and the bodies of the responses its `page.request`s got.
#[derive(Clone)]
struct ServedPage {
    document: Arc<ServedDocument>,
    /// The status its document was answered with.
    status: u16,
    http: HttpTier,
    choice: TierChoice,
    /// The first call its tier could not serve, under `auto`.
    escalated: Option<&'static str>,
    /// Its `page.request`s and navigations so far.
    requests: u64,
    bodies: BTreeMap<String, BodyHandle>,
    /// The URLs of its history's entries, oldest first, and the entry it
    /// is at.
    history: Vec<String>,
    at: usize,
    /// The type of its response when that is no HTML document (ADR
    /// 0119): the page then holds none.
    not_document: Option<String>,
    /// The encoding its document was read in and where that came from;
    /// none for a page holding no document.
    read: Option<Read>,
    /// What the readers of its untouched page read of its crawl request's
    /// own document, while the page holds it; nothing once a navigation
    /// replaced it.
    untouched: Arc<untouched::Read>,
    /// The Block cause its document's hand-off to the CPU pool classified
    /// (issue 2189); not classified once a navigation replaced the
    /// document.
    classified: Classified,
    /// The response of its crawl request's own document and its body,
    /// stored in the Run's folder only once `response.body` asks for it.
    own: Option<(PageResponse, Bytes)>,
    /// The invalid-certificate exception its document's host was reached
    /// through (ADR 0042).
    certificate_exception: Option<CertificateExceptionMark>,
    /// What the attempt it belongs to handed over: the navigation gate and
    /// the lease its navigations and `page.request`s go through.
    hand_over: Arc<HandOver>,
    /// Each redirect hop its crawl request's chain followed to its
    /// document, in order, which the Frontier learns from (ADR 0120).
    redirects: Arc<[Redirected]>,
}

impl ServedPage {
    /// The encoding its document was read in and where it came from, as
    /// Provenance records them (ADR 0119); `None` for a page holding no
    /// document, and on `light_browser`, where Lightpanda decoded the
    /// document itself, in an encoding the engine does not learn.
    fn encoding(&self) -> Option<EncodingProvenance> {
        self.read
            .filter(|_| self.choice.tier != FetchTier::LightBrowser)
            .map(|read| EncodingProvenance {
                encoding: read.encoding.name().to_owned(),
                source: Some(read.source.provenance()),
            })
    }
}

/// What a served page holds of its crawl request's own document, read as
/// one value ([`FetchTiers::facts`] while it is open, and
/// [`FetchTiers::close`] as it closes): a run classifies its page and
/// records its tier from it, so no read can come after the page closed
/// and silently find nothing.
#[derive(Clone, Debug)]
pub struct PageFacts {
    /// The response of its crawl request's own document, whose body
    /// `response.body` reads under its `request_id`.
    pub response: Option<PageResponse>,
    /// The type of its response when that is no HTML document, the page
    /// then holding none (ADR 0119).
    pub not_document: Option<String>,
    /// The encoding its document was read in and where it came from, as
    /// Provenance records them (ADR 0119).
    pub encoding: Option<EncodingProvenance>,
    /// The invalid-certificate exception its document's host was reached
    /// through, as Provenance marks it (ADR 0042, pitfall H03); `None`
    /// when its certificate verified.
    pub certificate_exception: Option<CertificateExceptionMark>,
    /// The AI-use signals its response carried.
    pub use_signals: UseSignals,
    /// The call that escalated it to the browser, the first its tier could
    /// not serve exactly under `auto`, as of the read: a document only a
    /// guess could read is escalated as it is served (ADR 0119), and a
    /// handler's call while it runs.
    pub escalated: Option<&'static str>,
}

/// A document a fetch answered, read as a served page holds it: its text,
/// its encoding, its `Link` header fields, its AI-use signals and what the
/// readers of its untouched page read of it.
struct Opened {
    document: Arc<ServedDocument>,
    /// The response's type when it is no HTML or XHTML document, its page
    /// then holding none (ADR 0119).
    not_document: Option<String>,
    encoding: &'static Encoding,
    /// The encoding ADR 0119's order found, and where; `None` when only a
    /// guess could read the document, its text then read as UTF-8 and
    /// never served.
    read: Option<Read>,
    links: Vec<(String, String)>,
    use_signals: UseSignals,
    /// What the readers of the untouched page read of a crawl request's
    /// own document; nothing of a handler's own navigation's.
    untouched: Arc<untouched::Read>,
    /// The Block cause its hand-off to the CPU pool classified.
    classified: Classified,
}

/// What a served document's hand-off to the CPU pool read of its Block
/// cause (issue 2189).
#[derive(Clone, Copy, Debug)]
enum Classified {
    /// The hand-off did not classify it: Block classification reads it in a
    /// hand-off of its own when asked.
    NotRead,
    /// The hand-off classified it: its Block cause, none for a document no
    /// rule classifies.
    Read(Option<Block>),
}

/// What a crawl request's own document's one hand-off to the CPU pool
/// reads beside its untouched readers (issue 2189): its formats, ahead of
/// the code-free crawl's `page.formats`, and its Block cause. Neither
/// takes a hand-off of its own when the document has one.
#[derive(Clone, Default)]
struct ReadAhead {
    /// The reader of the formats a code-free crawl reads of each page.
    formats: Option<Arc<dyn ServedFormatsAhead>>,
    /// The Block classification of the engine's pages, under
    /// `fetch.block_classification`.
    blocks: Option<Arc<dyn BrowserPages>>,
}

/// A document a fetch answered, decoded, and read by the readers of its
/// untouched page in one streaming pass when they can, on the page's own
/// task (issue 2189): what its hand-off to the CPU pool, when it takes
/// one, reads on.
struct Decoding {
    /// The response's type when it is no HTML or XHTML document.
    not_document: Option<String>,
    encoding: &'static Encoding,
    read: Option<Read>,
    /// Its text: empty for a response holding no document; a UTF-8 body
    /// read in place, the bytes the fetch kept (issue 2187).
    html: ByteString,
    /// What the readers of its untouched page read of it in one streaming
    /// pass; none when they read its tree.
    streamed: Option<untouched::StreamRead>,
}

/// A document a fetch answered, decoded and parsed once, before the
/// readers of its untouched page read its tree (ADR 0138).
struct Decoded {
    /// The response's type when it is no HTML or XHTML document.
    not_document: Option<String>,
    encoding: &'static Encoding,
    read: Option<Read>,
    /// Its text: empty for a response holding no document; a UTF-8 body
    /// read in place, the bytes the fetch kept (issue 2187).
    html: ByteString,
    /// What the readers of its untouched page read it on.
    untouched: Untouched,
}

/// What the readers of a document's untouched page read it on.
enum Untouched {
    /// The tree of its text, the one parse of the document.
    Tree(goodfetch_counted_work::ParsedHtml),
    /// What they read of it in one streaming pass, with no tree.
    Streamed(untouched::StreamRead),
}

impl Decoding {
    /// The document `fetched` answered for `run`, decoded in the encoding
    /// ADR 0119's order finds, `forced` being `fetch.encoding`'s: a
    /// response with no `Content-Type` is typed as Chromium's MIME sniffer
    /// types it, and one of a type that is no HTML or XHTML holds no
    /// document. `readers` read it in one streaming pass when they can:
    /// `passed`, the pass the fetch read its text in as it arrived, when
    /// that text is the one read here and `readers` read it (issue 2188),
    /// else a pass over the whole text.
    fn of(
        run: &Run,
        fetched: &crate::driver::Fetched,
        (forced, readers, passed): (Option<&'static Encoding>, &UntouchedReaders, Option<Passed>),
    ) -> Self {
        let url = fetched.url.as_str();
        let header = fetched
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str());
        let (essence, charset) = typed(header, &fetched.body, fetched.url.path());
        let not_document = (!is_document(&essence)).then_some(essence.clone());
        // The fetch read the text of a document its `Content-Type` typed as
        // its bytes arrived, under the same `fetch.encoding` unless a
        // coalesced fetch read it for another request.
        let text = not_document.is_none().then(|| match &fetched.text {
            Some(streamed) if streamed.read_under(forced) => streamed.text(&fetched.body),
            _ => (
                Text::of(
                    &fetched.body,
                    Kind::of(&essence),
                    charset.as_deref(),
                    forced,
                ),
                false,
            ),
        });
        // The pass read the text as it was handed out: it reads this text
        // only when this is the text handed out.
        let passed = passed
            .filter(|_| text.as_ref().is_some_and(|(_, as_handed)| *as_handed))
            .and_then(|passed| passed.by(readers));
        let (html, read) = text.map_or_else(Default::default, |(text, _)| (text.html, text.read));
        let encoding = read.map_or(encoding_rs::UTF_8, |read| {
            tracing::debug!(run_id = %run.id(), url, encoding = read.encoding.name(), source = read.source.as_str(), "a document's encoding was found");
            read.encoding
        });
        let streamed = passed.map_or_else(|| readers.stream(&html), Passed::read);
        Self {
            not_document,
            encoding,
            read,
            html,
            streamed,
        }
    }

    /// Whether reading it further needs the CPU pool, its one hand-off
    /// (issue 2189): its readers need its tree, or `ahead` reads its
    /// formats, which a document only a guess could read never has.
    fn on_pool(&self, ahead: &ReadAhead) -> bool {
        self.streamed.is_none()
            || (ahead.formats.is_some() && self.not_document.is_none() && self.read.is_some())
    }

    /// It, parsed once when its streaming pass did not read it.
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0138: a fetched document's one parse, for its untouched readers, when their streaming pass cannot vouch for it"
    )]
    fn parsed(self, run: &Run) -> Decoded {
        let Self {
            not_document,
            encoding,
            read,
            html,
            streamed,
        } = self;
        let untouched = match streamed {
            Some(streamed) => Untouched::Streamed(streamed),
            None => Untouched::Tree(
                run.metrics()
                    .counted_work()
                    .parse_html(Row::PageParses, &html),
            ),
        };
        Decoded {
            not_document,
            encoding,
            read,
            html,
            untouched,
        }
    }
}

impl Decoded {
    /// The refresh directive the markup of the HTML hop `fetched` holds,
    /// read on its tree with `readers`: a body whose type sniffs as no
    /// document is read whole for it, as the chain reads every HTML hop.
    fn refresh(
        &self,
        fetched: &crate::driver::Fetched,
        readers: &UntouchedReaders,
        work: &CountedWork,
    ) -> Option<crate::refresh::Refresh> {
        if self.not_document.is_some() {
            return crate::refresh::in_markup(&fetched.url, &fetched.body, work);
        }
        match &self.untouched {
            Untouched::Tree(tree) => readers.refresh(&fetched.url, tree),
            Untouched::Streamed(streamed) => streamed.refresh(&fetched.url),
        }
    }

    /// The served page's read of this document, which `fetched` answered
    /// for `run`, its formats read with `extractor`, its Label's
    /// `content.extractor`: its `<base>` and AI-use signals, then, for a
    /// crawl request's `own` document, what `ahead` reads of it on the
    /// CPU pool, its formats on a copy of its tree, and what `readers`
    /// read of it, its Page signature last, which consumes the tree.
    fn opened(
        self,
        run: &Run,
        fetched: &crate::driver::Fetched,
        (extractor, readers, own): (Option<BuiltIn>, &UntouchedReaders, bool),
        ahead: Option<&ReadAhead>,
    ) -> Opened {
        let Self {
            not_document,
            encoding,
            read,
            html,
            untouched,
        } = self;
        let url = fetched.url.to_string();
        let base_href = match &untouched {
            Untouched::Tree(tree) => readers.base_href(tree),
            Untouched::Streamed(streamed) => streamed.base_href(),
        };
        let base_url = goodfetch_frontier::document_base(&url, base_href.as_deref())
            .map_or_else(|| url.clone(), |base| base.to_string());
        let signals = UseSignals::of(&fetched.headers);
        let work = run.metrics().counted_work();
        // What the hand-off reads ahead, of a crawl request's own document.
        let ahead = ahead.filter(|_| own && not_document.is_none());
        let formats_of = |tree: Option<&goodfetch_counted_work::ParsedHtml>| {
            let reader = ahead?.formats.as_ref().filter(|_| read.is_some())?;
            Some(reader.read((&html, &url, &base_url), tree, work))
        };
        let (use_signals, untouched, kept, formats) = match untouched {
            Untouched::Tree(tree) => {
                let use_signals = readers.use_signals(signals, (&html, &tree), &fetched.url);
                // The formats read a copy of the tree, before the Page
                // signature takes it as served.
                let formats = formats_of(Some(&tree));
                let (untouched, kept) = if own {
                    let signed = |tree| {
                        goodfetch_extractor::page_signature_of(
                            tree,
                            &goodfetch_extractor::Document {
                                html: &html,
                                url: &url,
                                base_url: &base_url,
                                serialization: goodfetch_extractor::Serialization::Served,
                            },
                            chosen_extractor(run, extractor).extractor(),
                            work,
                        )
                    };
                    readers.read(tree, (&url, &base_url), (signed, work))
                } else {
                    (untouched::Read::default(), Some(tree))
                };
                (use_signals, untouched, kept.map(KeptFrom::Tree), formats)
            }
            Untouched::Streamed(streamed) => (
                UntouchedReaders::streamed_use_signals(signals, &fetched.url),
                if own {
                    readers.read_streamed(streamed, &url)
                } else {
                    untouched::Read::default()
                },
                None,
                formats_of(None),
            ),
        };
        let classified =
            ahead
                .and_then(|ahead| ahead.blocks.as_ref())
                .map_or(Classified::NotRead, |pages| {
                    work.count(Row::PageBlockClassifications);
                    let response = calls::page_response(
                        &fetched.url,
                        fetched.status,
                        fetched.headers.clone(),
                        String::new(),
                    );
                    Classified::Read(pages.classify(&response, &html))
                });
        // A handler's reads keep the page's untouched tree, or its HTML,
        // which the first read that needs a tree parses (ADR 0138).
        let tree = (readers.keeps() && not_document.is_none())
            .then(|| KeptTree::keep(run, kept.unwrap_or_else(|| KeptFrom::Html(html.clone()))));
        let links = fetched
            .headers
            .iter()
            .filter(|(name, _)| name == "link")
            .cloned()
            .collect();
        Opened {
            document: Arc::new(ServedDocument {
                run: run.clone(),
                url,
                base_url,
                html,
                extractor,
                tree,
                formats,
            }),
            not_document,
            encoding,
            read,
            links,
            use_signals,
            untouched: Arc::new(untouched),
            classified,
        }
    }
}

impl Opened {
    /// The document `fetched` answered a handler's own navigation of
    /// `run`, as [`Decoding::of`] reads it with the pass it holds, and
    /// [`Decoded::opened`] with `extractor` and `readers`, on the CPU pool.
    fn navigated(
        run: &Run,
        mut fetched: crate::driver::Fetched,
        extractor: Option<BuiltIn>,
        readers: &UntouchedReaders,
    ) -> Self {
        let passed = fetched.pass.take();
        Decoding::of(run, &fetched, (None, readers, passed))
            .parsed(run)
            .opened(run, &fetched, (extractor, readers, false), None)
    }
}

/// How a crawl request's chain reads each HTML hop's document: decoded and
/// streamed on the page's own task, then its refresh directive on its one
/// tree, and the document the chain ends on whole, on the same tree, with
/// what `ahead` reads of it, in one hand-off to the CPU pool when it needs
/// one (ADR 0138; issue 2189).
struct OpenedReading {
    run: Run,
    forced: Option<&'static Encoding>,
    extractor: Option<BuiltIn>,
    readers: Arc<UntouchedReaders>,
    ahead: ReadAhead,
}

impl OpenedReading {
    /// Whether reading `decoding`, the document `fetched` answered, needs
    /// the CPU pool: its readers need its tree, `ahead` reads its formats,
    /// or its body, typed as no document, is parsed for a refresh
    /// directive.
    fn pooled(&self, decoding: &Decoding) -> bool {
        decoding.not_document.is_some() || decoding.on_pool(&self.ahead)
    }
}

impl Reading for OpenedReading {
    type Read = Opened;
    type Decoded = (crate::driver::Fetched, Decoding);

    fn decode(&self, mut fetched: crate::driver::Fetched) -> Self::Decoded {
        let passed = fetched.pass.take();
        let decoding = Decoding::of(&self.run, &fetched, (self.forced, &self.readers, passed));
        (fetched, decoding)
    }

    fn on_pool(&self, (_, decoding): &Self::Decoded) -> bool {
        self.pooled(decoding)
    }

    fn read(
        &self,
        (fetched, decoding): Self::Decoded,
        follows: &dyn Fn(&crate::refresh::Refresh) -> bool,
    ) -> HopRead<Opened> {
        // `ahead` reads only in a hand-off the document takes anyway.
        let ahead = self.pooled(&decoding).then_some(&self.ahead);
        let decoded = decoding.parsed(&self.run);
        let work = self.run.metrics().counted_work();
        if let Some(refresh) = decoded
            .refresh(&fetched, &self.readers, work)
            .filter(|refresh| follows(refresh))
        {
            return HopRead::Refresh(refresh);
        }
        HopRead::Document(decoded.opened(
            &self.run,
            &fetched,
            (self.extractor, &self.readers, true),
            ahead,
        ))
    }

    fn work(&self) -> &CountedWork {
        self.run.metrics().counted_work()
    }
}

impl Pages {
    /// Keeps what the served page `page_id` reads of its document
    /// `opened` beside it: its encoding, links and AI-use signals.
    fn opened(&mut self, page_id: &PageId, opened: Opened) {
        self.encodings.insert(page_id.clone(), opened.encoding);
        if opened.links.is_empty() {
            self.links.remove(page_id);
        } else {
            self.links.insert(page_id.clone(), opened.links);
        }
        if opened.use_signals == UseSignals::default() {
            self.use_signals.remove(page_id);
        } else {
            self.use_signals.insert(page_id.clone(), opened.use_signals);
        }
    }
}

/// Where a navigation's document goes in its served page's history.
#[derive(Clone, Copy, Debug)]
enum History {
    /// A new entry after the one the page was at: `goto`.
    Pushed,
    /// This entry: `reload`, `goBack` and `goForward`.
    At(usize),
}

/// How a served page fared against its Label's Page check.
#[derive(Debug)]
pub enum Judged {
    /// It is the expected page: its handler is given it.
    Passed,
    /// Under `auto`, it failed its expected page, or its markup cannot
    /// judge it: its request runs in the browser instead.
    Escalated,
    /// It fails with this error, its handler never given it.
    Failed(CallError),
}

/// What Provenance names as the cause of an escalation by the Page check,
/// in place of a call: the setting.
pub const PAGE_CHECK_CALL: &str = "session.page_check";

/// What each of a code-free crawl's fields found in a page, in the order of
/// their names, with the document's base URL (ADRs 0123 and 0147).
#[derive(Clone, Debug)]
pub struct FieldsFound {
    /// The document's base URL.
    pub base_url: String,
    /// What each field found.
    pub found: Vec<Found>,
}

/// What the Frontier learns from a served page (ADRs 0005 and 0120): the
/// redirect hops its crawl request's chain followed, and its document.
#[derive(Clone, Debug)]
pub struct Lesson {
    /// Each redirect hop its chain followed to its document, in order.
    pub redirects: Arc<[Redirected]>,
    /// Its document's URL, page signature and `rel=canonical`; `None` for
    /// a page holding no document, which teaches nothing more.
    pub signed: Option<Signed>,
}

/// What the Frontier learns from a served page's document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    /// The URL it was fetched at.
    pub url: String,
    /// Its page signature.
    pub signature: [u8; 32],
    /// The URL its `rel=canonical` names, resolved, when it names one.
    pub canonical: Option<Url>,
}

/// The `proxy.*` keys that send a Run's Sessions through a proxy.
const PROXY_KEYS: [&Setting; 3] = [&PROXY_SERVER, &PROXY_PROVIDER, &PROXY_KIND];

/// The first `proxy.*` key `settings` send the Run's Sessions through a
/// proxy with, and how it is set.
fn proxied(settings: &Resolved) -> Option<(&'static str, &Effective)> {
    PROXY_KEYS.iter().find_map(|setting| {
        settings
            .effective(setting)
            .filter(|effective| !effective.value.is_null())
            .map(|effective| (setting.name, effective))
    })
}

/// Refuses a Run that allows the light-browser tier beside a proxy: the
/// light-browser bridge connects to sites directly, so Lightpanda's
/// requests would show them the machine's own address the proxy hides,
/// until the bridge chains through the Session's proxy.
///
/// # Errors
///
/// `capability_unavailable` naming `fetch.light_browser`.
fn refuse_light_beside_a_proxy(settings: &Resolved) -> Result<(), CallError> {
    let Some(allowed) = settings
        .effective(&FETCH_LIGHT_BROWSER)
        .filter(|effective| effective.value == serde_json::Value::Bool(true))
    else {
        return Ok(());
    };
    let Some((key, proxied)) = proxied(settings) else {
        return Ok(());
    };
    Err(CallError::new(
        &CAPABILITY_UNAVAILABLE,
        format!(
            "`fetch.light_browser` allows the `light_browser` tier in {}, whose requests Lightpanda sends past the Run's proxy, which `{key}` sets in {}: switch `fetch.light_browser` off, or unset `{key}`.",
            allowed.layer.as_str(),
            proxied.layer.as_str()
        ),
        &CapabilityUnavailableDetails {
            key: FETCH_LIGHT_BROWSER.name.to_owned(),
            layer: allowed.layer,
            kind: FetchTier::LightBrowser.as_str().to_owned(),
            fix: "`fetch.light_browser: false`".to_owned(),
        },
    ))
}

/// `capability_unavailable` for `fetch.tier: light_browser` forced in
/// `layer`, or by `label`, where `settings` do not allow the tier: naming
/// `fetch.light_browser` while it is off, else this platform, for which
/// Lightpanda builds nothing (ADRs 0020 and 0093).
fn light_unavailable(settings: &Resolved, layer: SettingsLayer, label: Option<&str>) -> CallError {
    let set = label.map_or_else(
        || layer.as_str().to_owned(),
        |label| format!("the Label `{label}`"),
    );
    let switched_on: bool = settings.get(&FETCH_LIGHT_BROWSER);
    let (key, fix, message) = if switched_on {
        (
            FETCH_TIER.name,
            "`fetch.tier: auto`",
            format!(
                "Lightpanda builds nothing for this platform, so no crawl request is fetched on the `light_browser` tier, which `fetch.tier` forces in {set}: set `fetch.tier` to `auto`, `http` or `browser`."
            ),
        )
    } else {
        (
            FETCH_LIGHT_BROWSER.name,
            "`fetch.light_browser: true`",
            format!(
                "`fetch.tier` forces the `light_browser` tier in {set}, which the Run does not allow: switch `fetch.light_browser` on, or set `fetch.tier` to `auto`, `http` or `browser`."
            ),
        )
    };
    CallError::new(
        &CAPABILITY_UNAVAILABLE,
        message,
        &CapabilityUnavailableDetails {
            key: key.to_owned(),
            layer,
            kind: FetchTier::LightBrowser.as_str().to_owned(),
            fix: fix.to_owned(),
        },
    )
}

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl FetchTiers {
    /// No tier serving any page yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// No tier serving any page yet, every Run's HTTP tier sending its hops
    /// with `transport` in place of reqwest: the fetch tiers' test seam,
    /// through which a crawl is fed from memory with no network (ADRs 0071
    /// and 0134). Its address check and egress policy still judge each URL
    /// first, by the Run's settings; a Session holding its own local
    /// addresses (`HttpTier::of_session`) still sends on reqwest. Through
    /// it no hop goes through the Run's proxy.
    pub fn serving(transport: Arc<dyn Transport>) -> Self {
        Self {
            served: Arc::new(Served {
                transport: Some(transport),
                ..Served::default()
            }),
        }
    }

    /// Hands the pages that open in `hand_over`'s Run over to its attempt
    /// until the answer drops: each takes the attempt's navigation gate and
    /// lease check (the `gate` module).
    pub fn hand_over(&self, hand_over: Arc<HandOver>) -> HandedOver {
        let run = hand_over.run().id().clone();
        let mut handed = lock(&self.served.handed);
        handed.last += 1;
        let number = handed.last;
        handed
            .by_run
            .entry(run.clone())
            .or_default()
            .push((number, hand_over));
        HandedOver {
            served: Arc::clone(&self.served),
            run,
            number,
        }
    }

    /// Refuses a Run whose `fetch.tier`, its own or one of its Labels' in
    /// `labels`, forces a tier this engine does not build yet, with
    /// `capability_unavailable` naming the setting, one that hedges or
    /// races its tiers where the second copy could never start, one
    /// whose `fetch.encoding` names no encoding, and one whose
    /// `crawl.content_types` is no list of media ranges.
    ///
    /// # Errors
    ///
    /// `capability_unavailable` for `discovered_api`, which replays a plan
    /// only a browser sample gives, for `light_browser` where
    /// `fetch.light_browser` does not allow it, and naming
    /// `fetch.light_browser` when it allows the tier beside a proxy, which
    /// Lightpanda's requests would leave past; `setting_invalid`
    /// for a Run's `fetch.encoding` that is no WHATWG Encoding label, its
    /// `crawl.content_types` that is no list of media ranges (ADR 0119)
    /// and a `fetch.redirect.max` outside 1 to 19 (ADR 0118);
    /// `setting_not_applicable` for hedges or `fetch.tier_race` switched on
    /// while a domain admits one request at a time (ADR 0086).
    pub fn check(settings: &Resolved, labels: &LabelSettings) -> Result<(), CallError> {
        goodfetch_frontier::refuse_second_copies(settings, &FETCH_HEDGE_ENABLED)?;
        goodfetch_frontier::refuse_second_copies(settings, &FETCH_TIER_RACE)?;
        refuse_redirect_max(settings)?;
        refuse_light_beside_a_proxy(settings)?;
        if let Some(effective) = settings.effective(&FETCH_ENCODING) {
            crate::choice::encoding("the Run", &effective.value)?;
        }
        if let Some(effective) = settings.effective(&CRAWL_CONTENT_TYPES) {
            ContentTypes::of(&effective.value).map_err(|reason| {
                CallError::new(
                    &SETTING_INVALID,
                    format!(
                        "`crawl.content_types` is {} in {}, which {reason}.",
                        effective.value,
                        effective.layer.as_str()
                    ),
                    &SettingInvalidDetails {
                        key: CRAWL_CONTENT_TYPES.name.to_owned(),
                        layer: Some(effective.layer),
                        reason: format!("{} {reason}", effective.value),
                    },
                )
            })?;
        }
        let run = forced(settings).map(|(tier, layer)| (tier, layer, None));
        let by_label = labels
            .forced()
            .map(|(label, tier)| (tier, SettingsLayer::Run, Some(label)));
        let light = Self::light_allowed(settings);
        for (tier, layer, label) in run.into_iter().chain(by_label) {
            if matches!(
                tier,
                FetchTier::Http | FetchTier::BrowserFetch | FetchTier::Browser
            ) || (tier == FetchTier::LightBrowser && light)
            {
                continue;
            }
            if tier == FetchTier::LightBrowser {
                return Err(light_unavailable(settings, layer, label));
            }
            let name = tier.as_str();
            let set = label.map_or_else(
                || layer.as_str().to_owned(),
                |label| format!("the Label `{label}`"),
            );
            return Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "This engine fetches no crawl request on the `{name}` tier, which `fetch.tier` forces in {set}: set `fetch.tier` to `auto`, `http`, `browser_fetch` or `browser`."
                ),
                &CapabilityUnavailableDetails {
                    key: FETCH_TIER.name.to_owned(),
                    layer,
                    kind: name.to_owned(),
                    fix: "`fetch.tier: auto`".to_owned(),
                },
            ));
        }
        Ok(())
    }

    /// The Run's own `fetch.tier`: the tier it forces, else `auto`, set or
    /// the built-in default, under which the engine chooses each request's
    /// tier.
    pub fn run_tier(settings: &Resolved) -> SetTier {
        forced(settings).map_or(SetTier::Auto, |(tier, _)| SetTier::Forced(tier))
    }

    /// An HTTP tier for `run` under its settings, one per Run so its hops
    /// share connections: the one its crawl requests on the `http` tier
    /// take, and its navigation gate fetches robots.txt with, sending from
    /// `local` when the Run sets `egress.local_addresses` (ADR 0044), and
    /// through a proxy of its own when the Run sets one (#1640). Each pool
    /// Session's own local addresses and proxy take
    /// [`HttpTier::of_session`].
    ///
    /// # Errors
    ///
    /// A message when the transport cannot be built: the system's resolver
    /// configuration, the Session's egress policy or `tls.client_certificates`
    /// cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the tier.
    pub async fn built(
        &self,
        run: &Run,
        settings: &Resolved,
        local: Option<LocalSource>,
    ) -> Result<HttpTier, String> {
        let egress = Policy::of(settings).map_err(|error| error.to_string())?;
        let trust = Trust::of(settings, run.owner()).await;
        let client_certificates = ClientCertificates::listed(
            settings.value(&TLS_CLIENT_CERTIFICATES),
            run.secrets().clone(),
            settings
                .effective(&TLS_CLIENT_CERTIFICATES)
                .map(|effective| effective.layer),
            Some(run.id().clone()),
        )
        .map_err(|error| error.to_string())?;
        let config = TransportConfig {
            trust,
            exceptions: exceptions_of(run, settings),
            client_certificates: Some(client_certificates),
            proxy: None,
            resolve: HostMapping::of(settings),
            egress: egress.clone(),
            clock: run.clock(),
            resolver: ResolverConfiguration::system().map_err(|error| error.to_string())?,
            local,
            delays: Delays::of(settings),
        };
        // Every Session of the Run launches one build, so the document
        // request its own transport presents is read once, here.
        let presents = match self.served.browser.get() {
            Some(pages) => match pages.document_request(run).await {
                Ok(request) => request,
                Err(unread) => {
                    warn_version_unread(run, &unread).await;
                    None
                }
            },
            None => None,
        };
        let reqwest = Reqwest::new(config.clone())
            .map_err(|error| error.to_string())?
            .presenting(presents.clone());
        let addresses = reqwest.address_check();
        let (transport, proxies): (Arc<dyn Transport>, _) = match &self.served.transport {
            Some(transport) => (Arc::clone(transport), None),
            None if proxied(settings).is_some() => {
                let proxies = SessionProxies::new(
                    run,
                    settings,
                    config.clone(),
                    presents.clone(),
                    Arc::new(reqwest),
                );
                (proxies.of_run(), Some(Arc::new(proxies)))
            }
            None => (Arc::new(reqwest), None),
        };
        Ok(HttpTier {
            addresses,
            transport,
            proxies,
            egress,
            bounds: FetchBounds::of(settings),
            redundant: Arc::new(Redundant::of(settings)),
            following: Following::of(settings),
            sessions: Arc::new(SessionTransports::new(
                config,
                presents,
                usize::try_from(settings.get::<u64>(&SESSION_POOL_MAX_SIZE)).unwrap_or(usize::MAX),
            )),
            jars: Arc::default(),
        })
    }

    /// The HTTP tier of an attempt of `run` in a Session that may move to
    /// the browser, `pooled` of the crawl's Session pool: `http` sending
    /// through browser-fetch from inside that Session's browser context,
    /// for the call `context`, the pages it sends from holding the lease of
    /// `hand_over`, the attempt's (ADR 0020). Its document, and each
    /// `page.request` of the page served from it, then carry the Session's
    /// cookies and TLS fingerprint, as its browser tier does (pitfall
    /// H20). `http` itself when the engine composed no browser pages.
    pub fn in_browser(
        &self,
        http: &HttpTier,
        run: &Run,
        pooled: &PooledSession,
        hand_over: &Arc<HandOver>,
        context: &CallContext,
    ) -> HttpTier {
        let Some(pages) = self.served.browser.get() else {
            return http.clone();
        };
        HttpTier {
            transport: Arc::new(BrowserFetch {
                pages: Arc::clone(pages),
                run_id: run.id().clone(),
                pooled: pooled.clone(),
                value: PageValue::new(Admitted, Lease::new(Arc::clone(hand_over))),
                context: context.clone(),
                egress: http.egress.clone(),
            }),
            ..http.clone()
        }
    }

    /// Fetches `request`'s document on `tier` for `run`, sent for
    /// `requester`, and opens a page served from it, whose ID the handler
    /// is given, its calls served as `choice` says. An identical request in
    /// flight from its Session answers its first hop instead, and a slow
    /// one is hedged (ADR 0040; the `redundant` module). Its redirects and
    /// refresh stubs are followed as the browser follows them (ADR 0118;
    /// the `chain` module), each hop after the first through the
    /// Frontier's checks and the navigation gate of `hand_over`, the
    /// attempt's.
    ///
    /// # Errors
    ///
    /// [`Stop::Failed`] with the fetch driver's errors (`deadline_exceeded`,
    /// `navigation_failed` with its network error, `response_truncated`)
    /// or the chain's (`too_many_redirects`, `response_not_ok`,
    /// `response_unavailable`), and `browser_required` under a forced tier
    /// for a document only a guess could read, which under `auto` opens a
    /// page already escalated at `fetch.encoding` instead (ADR 0119);
    /// [`Stop::Skipped`] for a hop the Frontier or
    /// the gate refuses, and with `domain_policy` for one the transport's
    /// egress check refuses; each with the redirect hops its chain followed
    /// ([`Unfollowed`]).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the fetch's connections and opens no
    /// page.
    pub async fn serve(
        &self,
        tier: &HttpTier,
        run: &Run,
        request: &CrawlRequest,
        requester: &Requester<'_>,
        choice: &TierChoice,
        hand_over: &Arc<HandOver>,
    ) -> Result<PageId, Unfollowed> {
        let hop = first_hop(request)?;
        let gate = ChainGate {
            hand_over: Arc::clone(hand_over),
            crawl_request: request.id.clone(),
        };
        let accepts = Accepts {
            types: hand_over.content_types.clone(),
            handled: tier.following.handled.clone(),
        };
        let clock = run.clock();
        let mut first = true;
        let ahead = self.read_ahead(run, hand_over).await;
        let reading = Arc::new(OpenedReading {
            run: run.clone(),
            forced: hand_over.encoding,
            extractor: hand_over.extractor,
            readers: Arc::clone(&hand_over.untouched),
            ahead: ahead.clone(),
        });
        let on = (&clock, run.owner(), &reading);
        let own = OwnDocument {
            accepts: &accepts,
            encoding: hand_over.encoding,
            readers: Some(&hand_over.untouched),
        };
        let fetched = follow_reading(hop, &tier.following, &gate, on, |hop| {
            let lead = std::mem::replace(&mut first, false);
            let clock = &clock;
            async move {
                if lead {
                    tier.redundant
                        .fetch(
                            tier.transport.as_ref(),
                            &tier.bounds,
                            run,
                            (&hop, Some(own)),
                            requester,
                        )
                        .await
                } else {
                    fetch_accepting(
                        tier.transport.as_ref(),
                        &hop,
                        &tier.bounds,
                        clock,
                        Some(own),
                    )
                    .await
                }
            }
        })
        .await;
        // The document's fetch ended: the page being analysed holds no
        // place among the requests in flight (ADR 0138).
        hand_over.fetch_admission.give_back();
        let followed = fetched?;
        if let Some(refused) = &followed.fetched.unaccepted {
            return Err(Unfollowed {
                stop: Stop::Skipped(Skip {
                    reason: SkipReason::ContentType,
                    message: format!(
                        "`{}` answered `{refused}`, which its Label's `crawl.content_types` does not list: its body was not read.",
                        followed.fetched.url
                    ),
                }),
                redirects: followed.redirects.into(),
            });
        }
        self.open_served(
            tier,
            run,
            choice,
            followed,
            (hand_over, hand_over.encoding, ahead),
        )
        .await
        .map_err(Unfollowed::from)
    }

    /// What the hand-off of each document a crawl request of `run` ends on
    /// reads beside its untouched readers (issue 2189): a code-free crawl's
    /// formats, which the hand-over's readers name, read with its
    /// extractor, and its Block cause, under `fetch.block_classification`.
    /// Nothing before the engine's pages were handed over.
    async fn read_ahead(&self, run: &Run, hand_over: &HandOver) -> ReadAhead {
        let Some(pages) = self.served.browser.get() else {
            return ReadAhead::default();
        };
        let formats = match hand_over.untouched.formats() {
            Some(request) => {
                pages
                    .served_formats(run, request, hand_over.extractor)
                    .await
            }
            None => None,
        };
        ReadAhead {
            formats,
            blocks: classifies_blocks(run).then(|| Arc::clone(pages)),
        }
    }

    /// Opens a page served from `fetched` on `tier` for `run`, its calls
    /// served as `choice` says and gated by `hand_over`, the attempt's, its
    /// document read with `forced` from `fetch.encoding`, its formats with
    /// the hand-over's extractor, its Label's `content.extractor`, and its
    /// untouched page with the hand-over's readers, and what `ahead` reads
    /// of it in its hand-off to the CPU pool: `followed`, the document its
    /// chain ended on, with what the chain read of it and the redirect hops
    /// it followed.
    ///
    /// # Errors
    ///
    /// `browser_required` under a forced tier for a document only a guess
    /// could read, which under `auto` opens a page already escalated at
    /// `fetch.encoding` instead (ADR 0119).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future opens no page; the document's read, on the
    /// CPU pool, runs to its end once it started.
    async fn open_served(
        &self,
        tier: &HttpTier,
        run: &Run,
        choice: &TierChoice,
        Followed {
            fetched,
            read: opened,
            redirects,
        }: Followed<Opened>,
        (hand_over, forced, ahead): (&Arc<HandOver>, Option<&'static Encoding>, ReadAhead),
    ) -> Result<PageId, Stop> {
        let status = fetched.status;
        let (url, headers, body) = (
            fetched.url.clone(),
            fetched.headers.clone(),
            fetched.body.clone(),
        );
        let certificate_exception = fetched.certificate_exception.as_ref().map(mark);
        // The document is decoded and streamed on the page's own task, and
        // parsed and read on the CPU pool, in one hand-off, never on an
        // async worker (STYLE.md; ADR 0138; issue 2189).
        let opened = if let Some(opened) = opened {
            opened
        } else {
            let (reading, extractor) = (run.clone(), hand_over.extractor);
            let readers = Arc::clone(&hand_over.untouched);
            let mut fetched = fetched;
            let passed = fetched.pass.take();
            let decoding = Decoding::of(run, &fetched, (forced, &readers, passed));
            if decoding.on_pool(&ahead) {
                run.owner()
                    .analysis(run.metrics().counted_work(), move || {
                        decoding.parsed(&reading).opened(
                            &reading,
                            &fetched,
                            (extractor, &readers, true),
                            Some(&ahead),
                        )
                    })
                    .await
            } else {
                decoding
                    .parsed(run)
                    .opened(run, &fetched, (extractor, &readers, true), None)
            }
        };
        // A document only a guess could read is never served: under a
        // forced tier it fails, and under `auto` its page escalates before
        // its handler runs (ADR 0119). A page holding no document is never
        // decoded.
        let escalated = match (&opened.read, &choice.forced) {
            _ if opened.not_document.is_some() => None,
            (Some(_), _) => None,
            (None, Some(forced)) => {
                return Err(Stop::failed(undeclared(
                    choice.tier,
                    forced,
                    &opened.document.url,
                )));
            }
            (None, None) => Some(ENCODING_CALL),
        };
        let mut pages = lock(&self.served.pages);
        pages.next += 1;
        let page_id = PageId(format!("document-{}", pages.next));
        let mut response =
            calls::page_response(&url, status, headers, format!("{}-response", page_id.0));
        response.encoding = opened.read.map(|read| read.encoding.name().to_owned());
        pages.open.insert(
            page_id.clone(),
            Arc::new(ServedPage {
                history: vec![opened.document.url.clone()],
                at: 0,
                document: Arc::clone(&opened.document),
                status,
                http: tier.clone(),
                choice: choice.clone(),
                escalated,
                requests: 0,
                bodies: BTreeMap::new(),
                not_document: opened.not_document.clone(),
                read: opened.read,
                untouched: Arc::clone(&opened.untouched),
                classified: opened.classified,
                own: Some((response, body)),
                certificate_exception,
                hand_over: Arc::clone(hand_over),
                redirects: redirects.into(),
            }),
        );
        pages.opened(&page_id, opened);
        Ok(page_id)
    }

    /// Loads `request`'s URL on the `light_browser` tier for `run`:
    /// Lightpanda, out of process, its build prepared in the Browser cache
    /// of `home` at the tier's first run by the Product version
    /// `pinned_by`, its requests sent only where `tier`, the Run's HTTP
    /// tier, may connect (ADRs 0017 and 0020). It opens a page served from
    /// the document the page's scripts left, whose calls are served as
    /// `choice` says, and whose type its Label must take (ADR 0119).
    ///
    /// # Errors
    ///
    /// [`Stop::Failed`] with the Browser cache's Error codes or
    /// `navigation_failed`; [`Stop::Skipped`] with `content_type` for a
    /// type its Label does not take.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future kills Lightpanda and opens no page.
    pub async fn light(
        &self,
        tier: &HttpTier,
        run: &Run,
        request: &CrawlRequest,
        choice: &TierChoice,
        hand_over: &Arc<HandOver>,
        (home, pinned_by): (&Home, &PinnedBy),
    ) -> Result<PageId, Stop> {
        let prepared = self
            .served
            .light
            .prepared(run, home, pinned_by)
            .await
            .map_err(Stop::failed)?;
        let rendered = crate::light_browser::render(
            run,
            &prepared.executable,
            &request.url,
            (tier.address_check(), &tier.bounds.limits),
        )
        .await
        .map_err(Stop::failed)?;
        let header = rendered
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.clone());
        let (essence, _) = typed(
            header.as_deref(),
            rendered.html.as_bytes(),
            rendered.url.path(),
        );
        if !hand_over.content_types.accepts(&essence) {
            return Err(Stop::Skipped(Skip {
                reason: SkipReason::ContentType,
                message: format!(
                    "`{}` answered `{essence}`, which its Label's `crawl.content_types` does not list.",
                    rendered.url
                ),
            }));
        }
        // The document is the one its scripts left, serialized as UTF-8:
        // Lightpanda decoded what the site sent.
        let mut headers: Vec<(String, String)> = rendered
            .headers
            .into_iter()
            .filter(|(name, _)| name != "content-type")
            .collect();
        if is_document(&essence) {
            headers.push((
                "content-type".to_owned(),
                format!("{essence}; charset=utf-8"),
            ));
        } else if let Some(header) = header {
            headers.push(("content-type".to_owned(), header));
        }
        let fetched = crate::driver::Fetched {
            url: rendered.url,
            status: rendered.status,
            headers,
            body: rendered.html.into_bytes().into(),
            unaccepted: None,
            // Lightpanda makes its own TLS connections through the bridge,
            // which relays them unread: no `tls.accept_invalid_certs`
            // exception is taken on this tier.
            certificate_exception: None,
            text: None,
            pass: None,
        };
        // Lightpanda follows its redirects itself: the chain records none.
        let followed = Followed {
            fetched,
            read: None,
            redirects: Vec::new(),
        };
        let ahead = self.read_ahead(run, hand_over).await;
        self.open_served(tier, run, choice, followed, (hand_over, None, ahead))
            .await
    }

    /// Whether a Run's resolved `settings` let its crawl take the
    /// `light_browser` tier: `fetch.light_browser` is on, and this platform
    /// has a pinned Lightpanda build (ADR 0020).
    pub fn light_allowed(settings: &Resolved) -> bool {
        settings.get::<bool>(&FETCH_LIGHT_BROWSER) && crate::light_browser::pinned_here().is_some()
    }

    /// What the served page `page_id` holds of its crawl request's own
    /// document now, read as one value ([`PageFacts`]); `None` once the
    /// page closed, when [`FetchTiers::close`] answered its last.
    pub fn facts(&self, page_id: &PageId) -> Option<PageFacts> {
        let pages = lock(&self.served.pages);
        let page = pages.open.get(page_id)?;
        Some(PageFacts {
            response: page.own.as_ref().map(|(response, _)| response.clone()),
            not_document: page.not_document.clone(),
            encoding: page.encoding(),
            certificate_exception: page.certificate_exception.clone(),
            use_signals: pages.use_signals.get(page_id).cloned().unwrap_or_default(),
            escalated: page.escalated,
        })
    }

    /// What the served page `page_id` teaches the Frontier (ADRs 0005 and
    /// 0120): the redirect hops its chain followed, and its document's URL,
    /// page signature and `rel=canonical`, read on its Run's blocking
    /// owner; `None` once the page closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn lesson(&self, page_id: &PageId) -> Option<Lesson> {
        let page = self.served_page(page_id)?;
        Some(Lesson {
            redirects: Arc::clone(&page.redirects),
            signed: self.signed(page_id, &page).await,
        })
    }

    /// The URL, the page signature and the `rel=canonical` of `page`, the
    /// served page `page_id`, read on its Run's blocking owner; `None` for
    /// a page holding no document.
    async fn signed(&self, page_id: &PageId, page: &ServedPage) -> Option<Signed> {
        if page.not_document.is_some() {
            return None;
        }
        let served = Arc::clone(&page.document);
        let links = lock(&self.served.pages)
            .links
            .get(page_id)
            .cloned()
            .unwrap_or_default();
        // The readers of its untouched page signed it as they read it.
        if let Some((signature, in_head)) = page.untouched.signed() {
            let canonical = in_head.cloned().or_else(|| {
                Url::parse(&served.url)
                    .ok()
                    .and_then(|url| crate::canonical::in_headers(&links, &url))
            });
            return Some(Signed {
                url: served.url.clone(),
                signature,
                canonical,
            });
        }
        let run = served.run.clone();
        let work = run.metrics().counted_work().clone();
        let extractor = chosen_extractor(&run, served.extractor);
        let signed = run
            .owner()
            .analysis(run.metrics().counted_work(), move || {
                let signature = goodfetch_extractor::page_signature(
                    &goodfetch_extractor::Document {
                        html: &served.html,
                        url: &served.url,
                        base_url: &served.base_url,
                        serialization: goodfetch_extractor::Serialization::Served,
                    },
                    extractor.extractor(),
                    &work,
                );
                let canonical = match (Url::parse(&served.base_url), Url::parse(&served.url)) {
                    (Ok(base), Ok(url)) => canonical(&served.html, &links, &base, &url, &work),
                    _ => None,
                };
                Signed {
                    url: served.url.clone(),
                    signature,
                    canonical,
                }
            })
            .await;
        Some(signed)
    }

    /// The elements the CSS selector `selector` matches in the page
    /// `page_id`'s document: a served page's, read on its Run's blocking
    /// owner; a browser page's, read by the page itself in the guard's
    /// isolated world with what a person sees of each element (the
    /// Frontier's `trap_links`, ADR 0035); `None` when no tier serves that
    /// page, as a closed one.
    ///
    /// # Errors
    ///
    /// When `selector` is no CSS selector, or the browser page's read
    /// failed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn links(
        &self,
        page_id: &PageId,
        selector: &str,
        work: &CountedWork,
    ) -> Option<Result<ServedLinks, LinksUnread>> {
        let opened = {
            let pages = lock(&self.served.pages);
            pages.open.get(page_id).map(|page| {
                let encoding = pages
                    .encodings
                    .get(page_id)
                    .copied()
                    .unwrap_or(encoding_rs::UTF_8);
                (
                    Arc::clone(&page.document),
                    encoding,
                    Arc::clone(&page.untouched),
                )
            })
        };
        let Some((served, encoding, untouched)) = opened else {
            return self.browser_links(page_id, selector, work).await;
        };
        // The readers of its untouched page read the links the Run asked
        // for as they read it.
        if let Some(read) = untouched.links(selector) {
            return Some(
                read.map(|elements| ServedLinks {
                    url: served.url.clone(),
                    base_url: served.base_url.clone(),
                    encoding,
                    elements,
                })
                .map_err(LinksUnread::Selector),
            );
        }
        // Any other selector reads the page's kept tree while a handler
        // reads it, else a parse of its document (ADR 0138).
        let selector = selector.to_owned();
        let work = served.run.metrics().counted_work().clone();
        let document = Arc::clone(&served);
        let read = calls::on_tree(&served, move |tree| {
            matched(tree, &selector, &work).map(|elements| ServedLinks {
                url: document.url.clone(),
                base_url: document.base_url.clone(),
                encoding,
                elements,
            })
        })
        .await?;
        Some(read.map_err(LinksUnread::Selector))
    }

    /// What each of `fields` finds in the page `page_id`'s document, in
    /// the order of their names: a served page's, read on its Run's
    /// blocking owner, by CSS selectors and `XPath` 1.0 expressions alike,
    /// as a browser reads them; a browser page's, read by the page itself
    /// in one engine read (ADRs 0123 and 0147). `None` when no tier serves
    /// that page, as a closed one.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a CSS selector or an `XPath` expression the
    /// engine does not read, and those of the browser page's read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn field_elements(
        &self,
        page_id: &PageId,
        fields: &BTreeMap<String, PageField>,
    ) -> Option<Result<FieldsFound, CallError>> {
        let served = {
            let pages = lock(&self.served.pages);
            pages
                .open
                .get(page_id)
                .map(|page| (Arc::clone(&page.document), Arc::clone(&page.untouched)))
        };
        let Some((served, untouched)) = served else {
            return self.browser_field_elements(page_id, fields).await;
        };
        let invalid =
            |sentence: String| CallError::protocol(ProtocolFailure::InvalidParams, sentence);
        // The readers of its untouched page read the value fields the Run
        // asked for as they read it.
        if let Some(locators) = document::value_locators(fields)
            && let Some(read) = untouched.fields(&locators)
        {
            return Some(
                read.map(|elements| FieldsFound {
                    base_url: served.base_url.clone(),
                    found: elements.into_iter().map(Found::One).collect(),
                })
                .map_err(|refused| invalid(refused.to_string())),
            );
        }
        let fields = fields.clone();
        let work = served.run.metrics().counted_work().clone();
        let base_url = served.base_url.clone();
        let read = calls::on_tree(&served, move |tree| {
            document::found(tree, &fields, &work).map(|found| FieldsFound { base_url, found })
        })
        .await?;
        Some(read.map_err(invalid))
    }

    /// What each of `fields` finds in the browser page `page_id`'s
    /// document, as [`FetchTiers::field_elements`] reads them; `None` for a
    /// page no browser holds.
    async fn browser_field_elements(
        &self,
        page_id: &PageId,
        fields: &BTreeMap<String, PageField>,
    ) -> Option<Result<FieldsFound, CallError>> {
        let pages = self.served.browser.get()?;
        let answer = match pages
            .read(
                page_id,
                goodfetch_page::fields::READ,
                goodfetch_page::fields::argument(fields),
                CRAWL_START.name,
            )
            .await
        {
            Ok(Some(answer)) => answer,
            // Its document went, or the page closed: no tier serves it now.
            Ok(None) => return None,
            Err(error) if error.code() == CLOSED.name => return None,
            Err(error) => return Some(Err(error)),
        };
        Some(
            goodfetch_page::fields::answered(&answer, fields, CRAWL_START.name).map(|answered| {
                FieldsFound {
                    base_url: answered.base_url,
                    found: answered.found,
                }
            }),
        )
    }

    /// The elements `selector` matches in the browser page `page_id`'s
    /// document, as [`FetchTiers::links`] reads them; `None` for a page no
    /// browser holds.
    async fn browser_links(
        &self,
        page_id: &PageId,
        selector: &str,
        work: &CountedWork,
    ) -> Option<Result<ServedLinks, LinksUnread>> {
        let pages = self.served.browser.get()?;
        // Checked through the engine's cache, so a Run's pages compile the
        // selector once between them (#1884's C2-3).
        if let Err(invalid) = document::matcher(selector, work) {
            return Some(Err(LinksUnread::Selector(invalid)));
        }
        let answer = match pages
            .read(
                page_id,
                goodfetch_frontier::trap_links::READ,
                goodfetch_frontier::trap_links::read_params(selector),
                CRAWL_REQUEST_LINKS.name,
            )
            .await
        {
            Ok(Some(answer)) => answer,
            // Its document went, or the page closed: no tier serves it now.
            Ok(None) => return None,
            Err(error) if error.code() == CLOSED.name => return None,
            Err(error) => return Some(Err(LinksUnread::Page(error))),
        };
        let Some((url, base_url, encoding, elements)) = document::browser_matched(&answer) else {
            return Some(Err(LinksUnread::Page(CallError::protocol(
                ProtocolFailure::InternalError,
                "the browser page's read of its links answered no list of elements",
            ))));
        };
        Some(Ok(ServedLinks {
            url,
            base_url,
            encoding: Encoding::for_label(encoding.as_bytes()).unwrap_or(encoding_rs::UTF_8),
            elements,
        }))
    }

    /// Judges the served page `page_id`'s document against `check`, its
    /// Label's Page check, before its handler is given it (ADRs 0006 and
    /// 0093), read on its Run's blocking owner: a page failing its expected
    /// page, or one its markup cannot judge, escalates under `auto` and
    /// fails under a forced tier, with `page_check_failed` or, unjudged,
    /// `browser_required`; a login page fails with `logged_out` on every
    /// tier. A page holding no document has no markup to judge: it passes.
    /// `None` once the page closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the verdict; the page stays open.
    pub async fn judge(&self, page_id: &PageId, check: &PageCheck) -> Option<Judged> {
        let page = self.served_page(page_id)?;
        if page.not_document.is_some() {
            return Some(Judged::Passed);
        }
        let ServedPage {
            document,
            status,
            choice,
            own,
            untouched,
            ..
        } = &*page;
        let document = Arc::clone(document);
        let status = *status;
        let run = document.run.clone();
        let url = document.url.clone();
        let headers = own
            .as_ref()
            .map(|(response, _)| response.headers.clone())
            .unwrap_or_default();
        // The readers of its untouched page judged it as they read it.
        let (verdict, noindex) = if let Some((verdict, robots)) = untouched.judged(check) {
            let noindex = verdict == Verdict::NotExpected
                && goodfetch_page::noindex::marked(&headers, robots);
            (verdict, noindex)
        } else {
            let check = check.clone();
            let work = run.metrics().counted_work().clone();
            run.owner()
                .analysis(run.metrics().counted_work(), move || {
                    expected_page::judge(&check, &document.url, &document.html, &headers, &work)
                })
                .await
        };
        Some(match verdict {
            Verdict::Expected => Judged::Passed,
            Verdict::LoginPage => Judged::Failed(expected_page::logged_out(&url, status)),
            Verdict::NotExpected | Verdict::Unjudged if choice.forced.is_none() => {
                Judged::Escalated
            }
            Verdict::NotExpected => Judged::Failed(expected_page::failed(&url, status, noindex)),
            Verdict::Unjudged => match unserved(choice, PAGE_CHECK_CALL, false) {
                Unserved::Escalated(_) => Judged::Escalated,
                Unserved::Refused(error) => Judged::Failed(error),
            },
        })
    }

    /// The Block cause of the served page `page_id`'s document, as the
    /// Product's Block classification tells it (ADR 0035), with the
    /// `blocked` its crawl request ends with when nothing moves it: the
    /// rules a browser page's navigation is judged by, so a crawl's HTTP
    /// tiers meet the blocks its browser does (spec #600). `None` for a
    /// document no rule classifies, a page holding no document, one closed,
    /// or before the engine's pages were handed over.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the verdict; the page stays open.
    pub async fn block(&self, page_id: &PageId) -> Option<(Block, CallError)> {
        let page = self.served_page(page_id)?;
        if page.not_document.is_some() {
            return None;
        }
        let response = page.own.as_ref().map(|(response, _)| response.clone())?;
        let run = page.document.run.clone();
        // Its document's hand-off classified it, when it took one.
        if let Classified::Read(classified) = page.classified {
            let received = run.clock().wall();
            return classified.map(|block| (block, blocked(block, &response, received)));
        }
        self.classified(&run, response, Arc::clone(&page.document))
            .await
    }

    /// The Block cause of `document`, which `response` answered for
    /// `run`, as the Product's Block classification tells it (ADR 0035),
    /// with the `blocked` its crawl request ends with when nothing moves
    /// it, read on the CPU pool, and counted on `F1.block_classifications`.
    /// `None` for a document no rule classifies, under
    /// `fetch.block_classification` off, which reads none (ADR 0138), or
    /// before the engine's pages were handed over.
    async fn classified(
        &self,
        run: &Run,
        response: PageResponse,
        document: Arc<ServedDocument>,
    ) -> Option<(Block, CallError)> {
        if !classifies_blocks(run) {
            return None;
        }
        let pages = Arc::clone(self.served.browser.get()?);
        run.metrics()
            .counted_work()
            .count(Row::PageBlockClassifications);
        let received = run.clock().wall();
        let (block, response) = run
            .owner()
            .analysis(run.metrics().counted_work(), move || {
                let block = pages.classify(&response, &document.html);
                block.map(|block| (block, response))
            })
            .await?;
        Some((block, blocked(block, &response, received)))
    }

    /// The formats `request` names of the page `page_id`, served or a
    /// browser's, as `page.formats` answers them: what a code-free crawl
    /// reads of each page (ADR 0103). `None` before the engine's pages were
    /// handed over.
    ///
    /// # Errors
    ///
    /// Those of `page.formats`, such as `closed` for a page closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn formats(
        &self,
        page_id: &PageId,
        request: FormatsRequest,
    ) -> Option<Result<PageFormats, CallError>> {
        let pages = Arc::clone(self.served.browser.get()?);
        Some(pages.formats(page_id, request).await)
    }

    /// Closes the served page `page_id`: later calls on it answer
    /// `closed`. Answers what it held as it closed, the call that
    /// escalated it while its handler ran included; `None` for a page
    /// already closed.
    pub fn close(&self, page_id: &PageId) -> Option<PageFacts> {
        let mut pages = lock(&self.served.pages);
        pages.links.remove(page_id);
        pages.encodings.remove(page_id);
        pages.replays.remove(page_id);
        let use_signals = pages.use_signals.remove(page_id).unwrap_or_default();
        let page = Arc::unwrap_or_clone(pages.open.remove(page_id)?);
        Some(PageFacts {
            encoding: page.encoding(),
            response: page.own.map(|(response, _)| response),
            not_document: page.not_document,
            certificate_exception: page.certificate_exception,
            use_signals,
            escalated: page.escalated,
        })
    }

    /// Has the served page `page_id`, on `discovered_api`, replay the
    /// calls of `plan`, its Label's, for the documents its navigations
    /// reach (ADR 0020).
    pub fn replay_on(&self, page_id: &PageId, plan: Plan) {
        let mut pages = lock(&self.served.pages);
        if pages.open.contains_key(page_id) {
            pages.replays.insert(
                page_id.clone(),
                Replaying {
                    plan,
                    waiters: Vec::new(),
                },
            );
        }
    }

    /// Opens Collie's page on the browser tier for a handler of the live
    /// Run `run_id`, in the Session of the crawl's Session pool that
    /// serves the handler, `pooled`, its own browser context, for the call
    /// `context`, whose
    /// Connection's Client then reaches it, and navigates it to `url`
    /// through its navigation gate: the attempt hands its Run's pages over
    /// first, as `hand_over`, and the page takes that hand-over's value,
    /// reader and observer, however many of the Run's attempts handed its
    /// pages over since. Answers its ID and the navigation's answer.
    ///
    /// # Errors
    ///
    /// Those of `run.new_page` and `page.goto`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the page opened closes it, its navigation
    /// with it.
    pub async fn browser(
        &self,
        hand_over: &Arc<HandOver>,
        pooled: &PooledSession,
        url: &str,
        context: &CallContext,
    ) -> Result<(PageId, Navigated), CallError> {
        let Some(pages) = self.served.browser.get() else {
            return Err(CallError::protocol(
                ProtocolFailure::InternalError,
                "the engine's composition handed the fetch tiers no browser pages",
            ));
        };
        let run_id = hand_over.run().id();
        OPENING
            .scope(
                Arc::clone(hand_over),
                pages.open(run_id, pooled, url, context),
            )
            .await
    }

    /// The invalid-certificate exception the browser page `page_id`
    /// reached the host of `url`, its document's, through, as its Records'
    /// Provenance marks it (ADR 0042): the one its Session's egress relay
    /// terminated that host's connections under; none without one.
    pub async fn browser_exception(
        &self,
        page_id: &PageId,
        url: &str,
    ) -> Option<CertificateExceptionMark> {
        let pages = self.served.browser.get()?;
        let url = Url::parse(url).ok()?;
        let host = url.host_str()?;
        pages
            .certificate_exception(page_id, host)
            .await
            .as_ref()
            .map(mark)
    }

    /// Retires the browser context of the Session `pooled` of the Run
    /// `run_id`'s Session pool, once the pool retired that Session and no
    /// attempt holds it: its pages close, and the pool's replacement opens
    /// a new one. One that never opened a browser page changes nothing.
    ///
    /// # Errors
    ///
    /// `internal`, for a Session its table does not let the pool retire.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the browser context open, until its
    /// Run ends.
    pub async fn retire_browser(
        &self,
        run_id: &RunId,
        pooled: &SessionId,
        context: &CallContext,
    ) -> Result<(), CallError> {
        match self.served.browser.get() {
            Some(pages) => pages.retire(run_id, pooled, context).await,
            None => Ok(()),
        }
    }

    /// The type of the browser page `page_id`'s crawl request's own
    /// document, when its Label's `crawl.content_types` does not take it:
    /// its navigation stopped at the interception, before anything read
    /// its body, and its request ends `skipped` (ADR 0119).
    pub fn browser_refused(&self, page_id: &PageId) -> Option<String> {
        lock(&self.served.pages)
            .browser
            .get(page_id)
            .and_then(|read| read.refused.clone())
    }

    /// The encoding the browser page `page_id`'s crawl request's own
    /// document was read in, as `encoding`, its `document.characterSet`,
    /// names it, and where ADR 0119's order found it; no source where only
    /// the browser's guess read it.
    pub fn browser_encoding(
        &self,
        page_id: &PageId,
        encoding: Option<&str>,
    ) -> Option<EncodingProvenance> {
        let own = lock(&self.served.pages)
            .browser
            .get(page_id)
            .and_then(|read| read.own);
        let encoding = encoding
            .map(str::to_owned)
            .or_else(|| own.map(|read| read.encoding.name().to_owned()))?;
        Some(EncodingProvenance {
            encoding,
            source: own.map(|read| read.source.provenance()),
        })
    }

    /// The URL of the last main-frame document the browser page `page_id`
    /// reached, which a script's redirect moves past its navigation's
    /// answer (ADR 0118); `None` before it reached one.
    pub fn browser_ended_at(&self, page_id: &PageId) -> Option<String> {
        lock(&self.served.pages)
            .browser
            .get(page_id)
            .and_then(|read| read.ended_at.clone())
    }

    /// Closes the browser page `page_id` [`FetchTiers::browser`] opened.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the page open, until its Run ends.
    pub async fn close_browser(&self, page_id: &PageId) {
        lock(&self.served.pages).browser.remove(page_id);
        if let Some(pages) = self.served.browser.get() {
            pages.close(page_id).await;
        }
    }

    /// The served page `page_id`, while it is open.
    fn served_page(&self, page_id: &PageId) -> Option<Arc<ServedPage>> {
        lock(&self.served.pages).open.get(page_id).map(Arc::clone)
    }

    /// The hand-over whose value, reader and observer a page of `run`
    /// opening now takes: that of the attempt whose own browser page
    /// [`FetchTiers::browser`] is opening, whichever attempt of the Run
    /// handed its pages over since (#2083); else that of the attempt that
    /// handed `run`'s pages over last.
    fn handed_over(&self, run: &RunId) -> Option<Arc<HandOver>> {
        if let Ok(opening) = OPENING.try_with(Arc::clone)
            && opening.run().id() == run
        {
            return Some(opening);
        }
        lock(&self.served.handed)
            .by_run
            .get(run)
            .and_then(|attempts| attempts.last())
            .map(|(_, hand_over)| Arc::clone(hand_over))
    }

    /// The ID of the next `page.request` of the served page `page_id`.
    fn next_request(&self, page_id: &PageId) -> String {
        let mut pages = lock(&self.served.pages);
        let number = pages.open.get_mut(page_id).map_or(0, |page| {
            let page = Arc::make_mut(page);
            page.requests += 1;
            page.requests
        });
        format!("{}-request-{number}", page_id.0)
    }

    /// Keeps `body`, the response body of the served page `page_id`'s
    /// request `request_id`, for `response.body`.
    fn keep_body(&self, page_id: &PageId, request_id: &str, body: BodyHandle) {
        if let Some(page) = lock(&self.served.pages).open.get_mut(page_id) {
            Arc::make_mut(page)
                .bodies
                .insert(request_id.to_owned(), body);
        }
    }

    /// Puts the document `opened`, which a navigation of the served page
    /// `page_id` reached, in its old one's place, where `at` says in the
    /// page's history.
    fn navigated(&self, page_id: &PageId, opened: Opened, at: History) {
        let mut pages = lock(&self.served.pages);
        let Some(page) = pages.open.get_mut(page_id) else {
            return;
        };
        let page = Arc::make_mut(page);
        match at {
            History::Pushed => {
                page.history.truncate(page.at + 1);
                page.history.push(opened.document.url.clone());
                page.at = page.history.len() - 1;
            }
            History::At(entry) => {
                page.at = entry;
                if let Some(url) = page.history.get_mut(entry) {
                    url.clone_from(&opened.document.url);
                }
            }
        }
        page.document = Arc::clone(&opened.document);
        page.not_document.clone_from(&opened.not_document);
        page.read = opened.read;
        page.untouched = Arc::clone(&opened.untouched);
        page.classified = Classified::NotRead;
        pages.opened(page_id, opened);
    }

    /// The error of `call` on the served page `page_id`, which its tier
    /// cannot serve exactly: under `auto` the page is escalated, by its
    /// first such call; under a forced tier `browser_required`. `waits`
    /// when the call waits for content the document lacks.
    fn unserved(&self, page_id: &PageId, call: &'static str, waits: bool) -> CallError {
        let mut pages = lock(&self.served.pages);
        let Some(page) = pages.open.get_mut(page_id) else {
            return CallError::new(
                &goodfetch_protocol::connection::CLOSED,
                format!("the page `{}` is closed", page_id.0),
                &serde_json::json!({ "page_id": page_id }),
            );
        };
        let first = page.escalated.unwrap_or(call);
        match unserved(&page.choice, first, waits) {
            Unserved::Escalated(error) => {
                if page.escalated.is_none() {
                    Arc::make_mut(page).escalated = Some(first);
                }
                error
            }
            Unserved::Refused(error) => error,
        }
    }
}

impl HandlerPages for FetchTiers {
    /// The value of the attempt opening its own browser page, else of the
    /// one that handed its Run's pages over last, while its handler runs
    /// ([`FetchTiers::handed_over`]); else a program's page's, empty.
    fn value(&self, run: &RunId, session: &SessionId) -> PageValue {
        match self.handed_over(run) {
            Some(hand_over) => {
                let (page_check, extractor) = (hand_over.page_check.clone(), hand_over.extractor);
                PageValue::new(Gate::new(Arc::clone(&hand_over)), Lease::new(hand_over))
                    .with_page_check(page_check)
                    .with_extractor(extractor)
            }
            None => ProgramPages.value(run, session),
        }
    }

    /// The capture of the attempt that handed its Run's pages over last,
    /// while its handler runs, when it is a sample's browser run (the
    /// `discovered_api` module); else a program's page's, none.
    fn observer(&self, run: &RunId, session: &SessionId) -> Option<Observer> {
        match self
            .handed_over(run)
            .and_then(|hand_over| hand_over.capture.clone())
        {
            Some(capture) => Some(Observer::new(capture)),
            None => ProgramPages.observer(run, session),
        }
    }

    /// The browser tier's document reader of the attempt that handed its
    /// Run's pages over last, while its handler runs; else a program's
    /// page's, none.
    fn reader(&self, run: &RunId, session: &SessionId) -> Option<Reader> {
        let Some(hand_over) = self.handed_over(run) else {
            return ProgramPages.reader(run, session);
        };
        let handled = hand_over
            .run()
            .settings()
            .map(|settings| Following::of(settings.resolved()).handled)
            .unwrap_or_default();
        Some(Reader::new(reading::BrowserReader {
            served: Arc::clone(&self.served),
            hand_over,
            handled,
        }))
    }

    fn document(&self, page_id: &PageId) -> Option<Arc<ServedDocument>> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .map(|page| Arc::clone(&page.document))
    }

    fn call(
        &self,
        page_id: &PageId,
        operation: &'static str,
        params: ReadParams<'_>,
        context: &CallContext,
    ) -> Option<GuardAnswer> {
        let served = calls::on_browser(self, page_id, operation, params)
            .or_else(|| calls::answer(self, page_id, operation, params))?;
        let context = context.clone();
        Some(Box::pin(async move {
            Answered::encode(&context, &served.await?)
        }))
    }

    fn opened(&self, pages: Arc<dyn BrowserPages>) {
        // A second composition of the same value keeps the first's pages.
        let _ = self.served.browser.set(pages);
    }
}

/// Hands the Run's report the warning that the installed browser's version
/// was not read, so its Sessions' own transport presents no browser
/// identity (ADR 0021).
async fn warn_version_unread(run: &Run, unread: &Unread) {
    let warning = RunWarning {
        code: BROWSER_VERSION_UNREAD.to_owned(),
        message: unread.to_string(),
        sessions: Vec::new(),
        folder: None,
    };
    if let Err(error) = run.warn(warning).await {
        tracing::warn!(%error, run_id = %run.id(), "the warning that the installed browser's version was not read was not kept");
    }
}

/// The HTTP tier of one Run: its transport, whose connections its hops
/// share, and the engine's timeouts its fetches wait under.
#[derive(Clone)]
pub struct HttpTier {
    /// reqwest, or browser-fetch in a Session that may move to the browser.
    transport: Arc<dyn Transport>,
    /// The Run's egress policy, which every transport judges each URL by.
    egress: Policy,
    /// The address check of the tier's reqwest connections, run ahead of a
    /// request whatever transport then sends it.
    addresses: AddressCheck,
    /// The timeouts its fetches wait under and the limits they read under.
    bounds: FetchBounds,
    /// Its Run's hedges and requests in flight, which no other Run shares.
    redundant: Arc<Redundant>,
    /// What its requests' redirect chains follow.
    following: Following,
    /// The reqwest transports of the Run's Sessions that hold their own
    /// local addresses, which every clone shares.
    sessions: Arc<SessionTransports>,
    /// The cookie jars of the Run's Sessions that send their HTTP
    /// themselves, which every clone shares.
    jars: Arc<Jars>,
    /// The proxies of the Run's Sessions, when it sets one, which every
    /// clone shares (#1640).
    proxies: Option<Arc<SessionProxies>>,
}

/// The reqwest transport of each local source a Session of the Run sends
/// from (ADR 0044), built on its first use, so the hops of Sessions that
/// share one share its connections. It keeps those of the sources used
/// last, as many as the pool keeps Sessions open (`session.pool.max_size`),
/// so a Run whose Sessions each take their own address, as a prefix's
/// positions give them, holds no transport of a Session long closed
/// (STYLE.md, bounded memory); a source used again past them is built
/// again.
struct SessionTransports {
    /// The Run's transport configuration, its `local` replaced for each.
    config: TransportConfig,
    /// The document request each presents: its Sessions' browser's.
    presents: Option<DocumentRequest>,
    /// The most transports kept.
    most: usize,
    built: Mutex<Built>,
}

/// The transports a Run's sources built, each with when it was last used.
#[derive(Default)]
struct Built {
    transports: BTreeMap<LocalSource, ((Arc<Reqwest>, AddressCheck), u64)>,
    /// How many times a transport was taken, which orders them by use.
    uses: u64,
}

impl SessionTransports {
    fn new(config: TransportConfig, presents: Option<DocumentRequest>, most: usize) -> Self {
        Self {
            config,
            presents,
            most: most.max(1),
            built: Mutex::new(Built::default()),
        }
    }

    /// The transport of `local`, built when no transport of it is kept,
    /// the one used longest ago dropped when the kept ones are as many as
    /// the pool's Sessions.
    ///
    /// # Errors
    ///
    /// A message when its transport cannot be built.
    fn of(&self, local: &LocalSource) -> Result<(Arc<Reqwest>, AddressCheck), String> {
        let mut built = self
            .built
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        built.uses += 1;
        let now = built.uses;
        if let Some((transport, used)) = built.transports.get_mut(local) {
            *used = now;
            return Ok(transport.clone());
        }
        let transport = Reqwest::new(TransportConfig {
            local: Some(local.clone()),
            ..self.config.clone()
        })
        .map_err(|error| error.to_string())?
        .presenting(self.presents.clone());
        let addresses = transport.address_check();
        let transport = (Arc::new(transport), addresses);
        if built.transports.len() >= self.most
            && let Some(oldest) = built
                .transports
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(source, _)| source.clone())
        {
            built.transports.remove(&oldest);
        }
        built
            .transports
            .insert(local.clone(), (transport.clone(), now));
        Ok(transport)
    }
}

/// How many redirects a robots.txt fetch follows: RFC 9309 §2.3.1.2's
/// least.
const ROBOTS_TXT_REDIRECTS: usize = 5;

impl HttpTier {
    /// The tier as the pool Session `session`, holding the local addresses
    /// `local`, sends on it: its connections and DNS queries leave from
    /// them (ADR 0044), from the Run's for `None`, and, when the Run sets
    /// a proxy, its hops go through the proxy the Session took on its first
    /// hop, which it holds until it retires (#1640).
    ///
    /// # Errors
    ///
    /// A message when its transport cannot be built.
    pub fn of_session(
        &self,
        session: &SessionId,
        local: Option<&LocalSource>,
    ) -> Result<HttpTier, String> {
        let own = local.filter(|local| self.sessions.config.local.as_ref() != Some(*local));
        let (direct, addresses) = match own {
            Some(local) => {
                let (transport, addresses) = self.sessions.of(local)?;
                (Some(transport as Arc<dyn Transport>), addresses)
            }
            None => (None, self.addresses.clone()),
        };
        let transport = match (&self.proxies, direct) {
            (Some(proxies), direct) => {
                proxies.of(session, own.cloned(), direct) as Arc<dyn Transport>
            }
            (None, Some(direct)) => direct,
            (None, None) => Arc::clone(&self.transport),
        };
        Ok(HttpTier {
            transport,
            addresses,
            ..self.clone()
        })
    }

    /// The tier as the pool Session `session` sends on it itself: its
    /// hops carry the cookies the answers to its earlier hops set, a
    /// redirect's among them, and keep those their own answers set (RFC
    /// 6265), in a jar no other Session reads (ADR 0021).
    #[must_use]
    pub fn keeping_cookies_of(&self, session: &SessionId) -> HttpTier {
        HttpTier {
            transport: Arc::new(WithJar {
                transport: Arc::clone(&self.transport),
                jar: self.jars.of(session),
            }),
            ..self.clone()
        }
    }

    /// The tier Provenance names for a document this tier fetched under
    /// the tier choice `chosen` (ADR 0023): the tier that sent its hops,
    /// `browser_fetch` from inside the Session's browser and `http` on a
    /// transport of its own, whoever chose it; `chosen` itself where it
    /// names more than who sends, as `discovered_api` and `light_browser`
    /// do.
    #[must_use]
    pub fn sent_on(&self, chosen: FetchTier) -> FetchTier {
        match chosen {
            FetchTier::Http | FetchTier::BrowserFetch => self.transport.tier(),
            FetchTier::DiscoveredApi | FetchTier::LightBrowser | FetchTier::Browser => chosen,
        }
    }

    /// Forgets the pool Session `session`, retired: its cookie jar and
    /// its proxy end with it, the Run warned when it outlived its
    /// provider's sticky lifetime (#1640).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the warning unwritten; the jar and
    /// the proxy are forgotten before its first `.await`.
    pub async fn forget_session(&self, session: &SessionId) {
        self.jars.forget(session);
        let proxy = self
            .proxies
            .as_ref()
            .and_then(|proxies| proxies.forget(session));
        if let Some(proxy) = proxy {
            proxy.warn_if_outlived().await;
        }
    }

    /// The address check of the tier's connections, which judges a crawl
    /// request's URL, and the addresses its name resolves to, before
    /// anything is sent for it (ADR 0017).
    pub fn address_check(&self) -> AddressCheck {
        self.addresses.clone()
    }

    /// [`HttpTier::robots_txt`] as the navigation gate fetches robots.txt.
    pub fn robots_txt_fetch(&self) -> RobotsTxtFetch {
        let tier = self.clone();
        Arc::new(move |run, location| {
            let tier = tier.clone();
            Box::pin(async move { tier.robots_txt(&run, &location).await })
        })
    }

    /// Fetches the AI-use signal file at `url` for `run`, `TDMRep`'s
    /// well-known file or an RSL license, as [`HttpTier::robots_txt`]
    /// fetches robots.txt: its body, its first 500 KiB, when it answers
    /// `2xx`; `None` otherwise.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the fetch's connection.
    pub async fn signal_file(&self, run: &Run, url: &Url) -> Option<Vec<u8>> {
        match self.robots_txt(run, url).await {
            Answer::Response { status, body } if (200..300).contains(&status) => Some(body),
            _ => None,
        }
    }

    /// Fetches the robots.txt at `location` for `run` (ADR 0086), following
    /// up to five redirects as RFC 9309 §2.3.1.2 asks: a `3xx` past them,
    /// or one naming no location, answers as itself, which disallows
    /// everything until a later fetch; a failed fetch answers why. Only its
    /// first 500 KiB are read and decoded, all RFC 9309 has a crawler
    /// parse, under the Run's limits (ADR 0067).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the fetch's connection.
    pub async fn robots_txt(&self, run: &Run, location: &Url) -> Answer {
        let mut url = location.clone();
        let mut redirects = 0;
        loop {
            let fetched = match fetch_start(
                self.transport.as_ref(),
                &Hop::get(url.clone()),
                &self.bounds,
                &run.clock(),
                MAX_SIZE,
            )
            .await
            {
                Ok(fetched) => fetched,
                Err(failure) => {
                    return Answer::Unreachable(failure.error().message().to_owned());
                }
            };
            let next = (300..400)
                .contains(&fetched.status)
                .then(|| {
                    fetched
                        .headers
                        .iter()
                        .find(|(name, _)| name == "location")
                        .and_then(|(_, to)| url.join(to).ok())
                })
                .flatten();
            match next {
                Some(next) if redirects < ROBOTS_TXT_REDIRECTS => {
                    redirects += 1;
                    url = next;
                }
                _ => {
                    return Answer::Response {
                        status: fetched.status,
                        body: fetched.body.to_vec(),
                    };
                }
            }
        }
    }
}

/// Provenance's action for the tier a crawl request's attempt ran on.
const FETCH_TIER_ACTION: &str = "fetch_tier";

/// How an attempt's run came to be on its tier, for its Provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// As its [`TierChoice`] says: forced, or the comparison's choice.
    AsChosen,
    /// One run of a sample of the crawl's sample comparison; on the
    /// browser, the call its cheaper run escalated at, if it did.
    Sample(Option<&'static str>),
    /// An escalation to the browser at this call, which the cheaper tier
    /// could not serve.
    Escalated(&'static str),
    /// The winning run of a race of browser-fetch against a navigation
    /// (`fetch.tier_race`, ADR 0040).
    Raced,
}

/// How `choice` came to fetch an attempt's document, as `how` says (ADR
/// 0023): `forced`, with who forced it, the Run or the Label; `compared`
/// under `auto`, the sample comparison's choice; `sample`, one run of a
/// sample; `escalation` to the browser, naming the call the cheaper tier
/// could not serve; or `race`, the winner of a tier race. It names no
/// client for replayed calls: the attempt that replays them does.
pub fn tier_provenance(choice: &TierChoice, how: Chosen) -> TierProvenance {
    let (chosen, call) = match how {
        Chosen::AsChosen if choice.forced.is_some() => (TierChosen::Forced, None),
        Chosen::AsChosen => (TierChosen::Compared, None),
        Chosen::Sample(call) => (TierChosen::Sample, call),
        Chosen::Escalated(call) => (TierChosen::Escalation, Some(call)),
        Chosen::Raced => (TierChosen::Race, None),
    };
    TierProvenance {
        tier: choice.tier,
        chosen,
        forced_by: choice.forced.as_ref().map(|forced| {
            if forced.label.is_some() {
                ForcedBy::Label
            } else {
                ForcedBy::Run
            }
        }),
        call: call.map(str::to_owned),
        calls_sent_on: None,
    }
}

/// Records in `run`'s Provenance the tier `tier` for an attempt of the
/// crawl request `request_id` of `label`, as [`tier_provenance`] gives it,
/// and the `encoding` its document was read in, with where it came from
/// (ADR 0119), when it read one; a document the browser read only by its
/// guess, after an escalation at `fetch.encoding`, is recorded as
/// `detected`. Answers that encoding as recorded.
///
/// # Cancel safety
///
/// Dropping the future never stops the line being written once it began.
pub async fn record_tier(
    run: &Run,
    request_id: &str,
    label: &str,
    tier: &TierProvenance,
    encoding: Option<EncodingProvenance>,
) -> Option<EncodingProvenance> {
    // The browser reads a document only a guess decodes with its own
    // detector (ADR 0119).
    let encoding = if tier.call.as_deref() == Some(ENCODING_CALL) {
        encoding.map(|encoding| EncodingProvenance {
            source: Some(EncodingSource::Detected),
            ..encoding
        })
    } else {
        encoding
    };
    let mut entry = serde_json::json!({
        "action": FETCH_TIER_ACTION,
        "crawl_request_id": request_id,
        "label": label,
        "tier": tier.tier,
        "chosen": tier.chosen,
    });
    if let Some(forced_by) = tier.forced_by {
        entry["forced_by"] = serde_json::json!(forced_by);
    }
    if let Some(encoding) = &encoding {
        entry["encoding"] = serde_json::json!(encoding.encoding);
        if let Some(source) = encoding.source {
            entry["encoding_source"] = serde_json::json!(source);
        }
    }
    if let Some(call) = &tier.call {
        entry["call"] = serde_json::json!(call);
        if call == ENCODING_CALL && encoding.is_none() {
            entry["encoding_source"] = serde_json::json!(EncodingSource::Detected);
        }
    }
    if let Some(calls_sent_on) = tier.calls_sent_on {
        entry["calls_sent_on"] = serde_json::json!(calls_sent_on);
    }
    goodfetch_page::provenance(run, entry).await;
    encoding
}

/// Provenance's action for a race of a crawl request's tiers.
const TIER_RACE_ACTION: &str = "tier_race";

/// How one run of a tier race ended (ADR 0040).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceEnd {
    /// Its result passed the Page check and its Label's comparison first.
    Won,
    /// It answered no result that could win.
    Failed,
    /// It was sent, then cancelled once the other won.
    Cancelled,
    /// It was never sent: the other won first, or Usage had none left.
    NotSent,
}

impl RaceEnd {
    /// Its name in Provenance.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Won => "won",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NotSent => "not_sent",
        }
    }
}

/// How a tier race of one crawl request ran, for its Provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Race<'a> {
    /// The crawl request's ID.
    pub request_id: &'a str,
    /// Its Label.
    pub label: &'a str,
    /// How long after its first request started the navigation was to
    /// start.
    pub after: Duration,
    /// The tier that sent its first request: `browser_fetch` from inside
    /// a Session's browser, `http` from a Session's own transport (#2421).
    pub first_sent_on: FetchTier,
    /// How its first request ended.
    pub first: RaceEnd,
    /// How the navigation ended.
    pub navigation: RaceEnd,
}

/// Records in `run`'s Provenance how a tier race ran (ADR 0040): its
/// delay, how each run ended, the first request under the name of the tier
/// that sent it, `browser_fetch` or `http`, and which won, that tier,
/// `browser` or none.
///
/// # Cancel safety
///
/// Dropping the future never stops the line being written once it began.
pub async fn record_race(run: &Run, race: &Race<'_>) {
    let first = race.first_sent_on.as_str();
    let winner = if race.first == RaceEnd::Won {
        Some(first)
    } else if race.navigation == RaceEnd::Won {
        Some(FetchTier::Browser.as_str())
    } else {
        None
    };
    let mut entry = serde_json::json!({
        "action": TIER_RACE_ACTION,
        "crawl_request_id": race.request_id,
        "label": race.label,
        "after_ms": u64::try_from(race.after.as_millis()).unwrap_or(u64::MAX),
        "navigation": race.navigation.as_str(),
        "winner": winner,
    });
    entry[first] = serde_json::Value::from(race.first.as_str());
    goodfetch_page::provenance(run, entry).await;
}

/// Refuses a `fetch.redirect.max` outside 1 to 19, as many redirects as
/// the browser follows (ADR 0118), with `setting_invalid`.
fn refuse_redirect_max(settings: &Resolved) -> Result<(), CallError> {
    let Some(effective) = settings.effective(&FETCH_REDIRECT_MAX) else {
        return Ok(());
    };
    let max = effective.value.as_u64().unwrap_or_default();
    if (1..=REDIRECT_MAX_LIMIT).contains(&max) {
        return Ok(());
    }
    Err(CallError::new(
        &SETTING_INVALID,
        format!(
            "`fetch.redirect.max` is {max} in {}, but a fetch follows from 1 to {REDIRECT_MAX_LIMIT} redirects, as many as the browser follows: list a redirect status in `crawl.handle_statuses` to hand it to the handler unfollowed.",
            effective.layer.as_str()
        ),
        &SettingInvalidDetails {
            key: FETCH_REDIRECT_MAX.name.to_owned(),
            layer: Some(effective.layer),
            reason: format!("expected a count from 1 to {REDIRECT_MAX_LIMIT}, found `{max}`"),
        },
    ))
}

/// The tier `fetch.tier` forces and the layer that set it; `None` under
/// `auto`.
fn forced(settings: &Resolved) -> Option<(FetchTier, SettingsLayer)> {
    let effective = settings.effective(&FETCH_TIER)?;
    let tier = FetchTier::named(effective.value.as_str()?)?;
    Some((tier, effective.layer))
}

/// The first hop of `request`'s fetch.
///
/// # Errors
///
/// `navigation_failed` for a URL that does not parse.
fn first_hop(request: &CrawlRequest) -> Result<Hop, Stop> {
    let url = Url::parse(&request.url).map_err(|error| {
        Stop::failed(CallError::new(
            &goodfetch_protocol::page::NAVIGATION_FAILED,
            format!(
                "The crawl request's URL `{}` does not parse: {error}.",
                request.url
            ),
            &goodfetch_protocol::page::NavigationFailedDetails {
                url: Some(request.url.clone()),
                error: None,
            },
        ))
    })?;
    Ok(Hop {
        url,
        method: http::Method::from_bytes(request.method.as_str().as_bytes())
            .unwrap_or(http::Method::GET),
        headers: Vec::new(),
        body: request
            .body
            .as_ref()
            .map(|body| Bytes::copy_from_slice(body.bytes())),
        // A crawl request is a read whatever its method (ADR 0106).
        read: true,
    })
}

/// The Provenance mark of the exception `used`.
fn mark(used: &ExceptionUsed) -> CertificateExceptionMark {
    CertificateExceptionMark {
        kind: match used.kind {
            ExceptionKind::Any => CertificateExceptionKind::Any,
            _ => CertificateExceptionKind::TrustOnFirstUse,
        },
        spki_sha256: used.certificate.to_string(),
    }
}

/// The invalid-certificate exceptions of `settings`' `tls.accept_invalid_certs`
/// for `run`, pinning in the person's known-certificates store; none,
/// logged, when an entry names no host.
fn exceptions_of(run: &Run, settings: &Resolved) -> CertificateExceptions {
    let book = PinBook {
        store: run.known_certificates(),
        person: run.person().to_owned(),
        run_id: Some(run.id().clone()),
        audit: Arc::new(run.audit_log()),
        clock: run.clock(),
    };
    CertificateExceptions::listed(
        settings.value(&TLS_ACCEPT_INVALID_CERTS),
        book,
        run.secrets().exception_hosts(),
    )
    .unwrap_or_else(|error| {
        tracing::warn!(%error, "`tls.accept_invalid_certs` names no host, so it makes no exception");
        CertificateExceptions::default()
    })
}

/// The `blocked` a crawl request whose document `response` answered,
/// received at `received`, ends with for `block`: its cause, URL, status
/// and the time the site's `Retry-After` asks for (ADR 0079).
/// Whether the Run `run` reads its documents for a Block cause, its
/// `fetch.block_classification` (ADR 0138).
fn classifies_blocks(run: &Run) -> bool {
    get_or_default(
        run.settings().map(RunSettings::resolved),
        &FETCH_BLOCK_CLASSIFICATION,
    )
}

fn blocked(block: Block, response: &PageResponse, received: std::time::SystemTime) -> CallError {
    let retry_after = response
        .headers
        .get("retry-after")
        .map(|value| {
            retry_after::read(
                value,
                response.headers.get("date").map(String::as_str),
                received,
            )
        })
        .and_then(|asked| match asked {
            RetryAfter::Until(until) => jiff::Timestamp::try_from(until).ok(),
            RetryAfter::NoWait | RetryAfter::Unparsable => None,
        });
    CallError::new(
        &BLOCKED,
        format!(
            "The document of `{}`, answered with {}, is a block, `{}`: {} (rule `{}`). Its fetch tier is forced, so the request is not moved to the browser.",
            response.url,
            response.status,
            block.cause.as_str(),
            block.cause.description(),
            block.rule,
        ),
        &BlockedDetails {
            cause: block.cause,
            url: response.url.clone(),
            status: Some(response.status),
            retry_after,
        },
    )
}

/// The main-content step a page of `run` is signed with, the one its
/// formats read its main content with: the page crate's one resolver, given
/// its Label's `content.extractor`, `label` (ADR 0138; spec #2164).
fn chosen_extractor(run: &Run, label: Option<BuiltIn>) -> goodfetch_extractor::Chosen {
    goodfetch_page::extractor::served(run.settings().map(RunSettings::resolved), label)
}
