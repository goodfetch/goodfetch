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
use encoding_rs::Encoding;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::dns::ResolverConfiguration;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::ExceptionKind;
use goodfetch_engine::retry_after::{self, RetryAfter};
use goodfetch_engine::robots_txt::{Answer, MAX_SIZE};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tls::{
    CertificateExceptions, ClientCertificates, ExceptionUsed, PinBook, trusted_cas,
};
use goodfetch_engine::{CallContext, CallError, GuardAnswer};
use goodfetch_extractor::BuiltIn;
use goodfetch_page::blocks::Block;
use goodfetch_page::page_value::{
    BrowserPages, HandlerPages, Observer, PageValue, PooledSession, ProgramPages, Reader,
    ServedDocument,
};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_CONTENT_TYPES, CRAWL_START, CrawlRequest, FieldLocator,
};
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
use goodfetch_protocol::foxhound::session_pool::SESSION_POOL_MAX_SIZE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::proxy::{PROXY_KIND, PROXY_PROVIDER, PROXY_SERVER};
use goodfetch_protocol::page::{
    BLOCKED, BlockedDetails, BodyHandle, FormatsRequest, Navigated, PageFormats, PageId,
    PageResponse, SessionId,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, NETWORK_RESOLVE, SETTING_INVALID,
    SettingInvalidDetails, SettingsLayer,
};
use goodfetch_protocol::tls::{TLS_ACCEPT_INVALID_CERTS, TLS_CLIENT_CERTIFICATES, TLS_TRUSTED_CAS};
use url::Url;

use crate::canonical::canonical;
use crate::chain::{Following, Skip, Stop, follow};
use crate::charset::{self, Kind, Read};
use crate::choice::{
    ENCODING_CALL, LabelSettings, SetTier, TierChoice, Unserved, undeclared, unserved,
};
use crate::content_types::{Accepts, ContentTypes, is_document, typed};
use crate::discovered_api::Plan;
use crate::document::{self, FieldElement, Matched, SelectorInvalid, matched};
use crate::driver::{FetchBounds, Hop, Transport, fetch_accepting, fetch_start};
use crate::expected_page::{self, Verdict};
use crate::gate::{Admitted, ChainGate, Gate, HandOver, Lease, RobotsTxtFetch};
use crate::light_browser::LightBrowser;
use crate::redundant::{Redundant, Requester};
use crate::transports::{AddressCheck, BrowserFetch, LocalSource, Reqwest, TransportConfig};
use crate::use_signals::UseSignals;

mod calls;
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

#[derive(Default)]
struct Pages {
    /// The number of the next page served.
    next: u64,
    open: BTreeMap<PageId, ServedPage>,
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
    answer: tokio::sync::oneshot::Sender<Option<goodfetch_protocol::page::events::PageExchange>>,
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
    /// The response of its crawl request's own document and its body,
    /// stored in the Run's folder only once `response.body` asks for it.
    own: Option<(PageResponse, Bytes)>,
    /// The invalid-certificate exception its document's host was reached
    /// through (ADR 0042).
    certificate_exception: Option<CertificateExceptionMark>,
    /// What the attempt it belongs to handed over: the navigation gate and
    /// the lease its navigations and `page.request`s go through.
    hand_over: Arc<HandOver>,
}

/// A document a fetch answered, read as a served page holds it: its text,
/// its encoding, its `Link` header fields and its AI-use signals.
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
}

impl Opened {
    /// The document `fetched` answered for `run`, its formats read with
    /// `extractor`, its Label's `content.extractor`, decoded in the
    /// encoding ADR 0119's order finds, `forced` being `fetch.encoding`'s: a
    /// response with no `Content-Type` is typed as Chromium's MIME sniffer
    /// types it, and one of a type that is no HTML or XHTML holds no
    /// document.
    fn of(
        run: &Run,
        fetched: crate::driver::Fetched,
        (forced, extractor): (Option<&'static Encoding>, Option<BuiltIn>),
    ) -> Self {
        let url = fetched.url.to_string();
        let header = fetched
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str());
        let (essence, charset) = typed(header, &fetched.body, fetched.url.path());
        let not_document = (!is_document(&essence)).then_some(essence.clone());
        let read = not_document
            .is_none()
            .then(|| {
                charset::read(
                    &fetched.body,
                    Kind::of(&essence),
                    charset.as_deref(),
                    forced,
                )
            })
            .flatten();
        let (html, encoding) = match read {
            _ if not_document.is_some() => (String::new(), encoding_rs::UTF_8),
            Some(read) => {
                tracing::debug!(run_id = %run.id(), url, encoding = read.encoding.name(), source = read.source.as_str(), "a document's encoding was found");
                (charset::decode(&fetched.body, read), read.encoding)
            }
            None => (
                String::from_utf8_lossy(&fetched.body).into_owned(),
                encoding_rs::UTF_8,
            ),
        };
        let base_url = goodfetch_frontier::document_base(
            &url,
            base_href(&html, run.metrics().counted_work()).as_deref(),
        )
        .map_or_else(|| url.clone(), |base| base.to_string());
        let markup = if not_document.is_none() {
            html.as_str()
        } else {
            ""
        };
        let use_signals = UseSignals::of(&fetched.headers).with_markup(markup, &fetched.url);
        let links = fetched
            .headers
            .into_iter()
            .filter(|(name, _)| name == "link")
            .collect();
        Self {
            document: Arc::new(ServedDocument {
                run: run.clone(),
                url,
                base_url,
                html,
                extractor,
            }),
            not_document,
            encoding,
            read,
            links,
            use_signals,
        }
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

/// The first element each of a code-free crawl's field locators found in
/// a page, in order, with the document's base URL (ADR 0123).
#[derive(Clone, Debug)]
pub struct FieldElements {
    /// The document's base URL.
    pub base_url: String,
    /// The first element each locator found; `None` for one that found none.
    pub elements: Vec<Option<FieldElement>>,
}

/// What the Frontier learns from a served page (ADRs 0005 and 0120).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    /// The URL it was fetched at.
    pub url: String,
    /// Its page signature.
    pub signature: [u8; 32],
    /// The URL its `rel=canonical` names, resolved, when it names one.
    pub canonical: Option<Url>,
}

/// `capability_unavailable` for `fetch.tier: light_browser` forced in
/// `layer`, or by `label`, where `settings` do not allow the tier: naming
/// `fetch.light_browser` while it is off, else this platform, for which
/// Lightpanda builds nothing (ADRs 0020 and 0093).
/// The `proxy.*` keys that send a Run's Sessions through a proxy.
const PROXY_KEYS: [&str; 3] = [PROXY_SERVER.name, PROXY_PROVIDER.name, PROXY_KIND.name];

/// Refuses a Run that allows the light-browser tier beside a proxy: the
/// light-browser bridge connects to sites directly, so Lightpanda's
/// requests would show them the machine's own address the proxy hides,
/// until #1640 chains the bridge through the Session's proxy.
///
/// # Errors
///
/// `capability_unavailable` naming `fetch.light_browser`.
fn refuse_light_beside_a_proxy(settings: &Resolved) -> Result<(), CallError> {
    let Some(allowed) = settings
        .get(FETCH_LIGHT_BROWSER.name)
        .filter(|effective| effective.value == serde_json::Value::Bool(true))
    else {
        return Ok(());
    };
    let Some((key, proxied)) = PROXY_KEYS.iter().find_map(|key| {
        settings
            .get(key)
            .filter(|effective| !effective.value.is_null())
            .map(|effective| (*key, effective))
    }) else {
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

fn light_unavailable(settings: &Resolved, layer: SettingsLayer, label: Option<&str>) -> CallError {
    let set = label.map_or_else(
        || layer.as_str().to_owned(),
        |label| format!("the Label `{label}`"),
    );
    let switched_on = settings.boolean(FETCH_LIGHT_BROWSER.name) == Some(true);
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
    /// addresses (`HttpTier::from_session`) still sends on reqwest.
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
        let run = hand_over.run.id().clone();
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
    /// Lightpanda's requests would leave past (#1640); `setting_invalid`
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
        if let Some(effective) = settings.get(FETCH_ENCODING.name) {
            crate::choice::encoding("the Run", &effective.value)?;
        }
        if let Some(effective) = settings.get(CRAWL_CONTENT_TYPES.name) {
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

    /// The Run's own `fetch.tier`: the tier it forces, `auto` set by any
    /// layer above the built-in default, or the default.
    pub fn run_tier(settings: &Resolved) -> SetTier {
        if let Some((tier, _)) = forced(settings) {
            return SetTier::Forced(tier);
        }
        match settings.get(FETCH_TIER.name) {
            Some(effective) if effective.layer != SettingsLayer::Default => SetTier::Auto,
            _ => SetTier::Unset,
        }
    }

    /// An HTTP tier for `run` under its settings, one per Run so its hops
    /// share connections: the one its crawl requests on the `http` tier
    /// take, and its navigation gate fetches robots.txt with, sending from
    /// `local` when the Run sets `egress.local_addresses` (ADR 0044). Each
    /// Session's own local addresses take [`HttpTier::from_session`].
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
        let trust = trusted_cas(settings.value(TLS_TRUSTED_CAS.name), run.owner()).await;
        let client_certificates = ClientCertificates::listed(
            settings.value(TLS_CLIENT_CERTIFICATES.name),
            run.secrets().clone(),
            settings
                .get(TLS_CLIENT_CERTIFICATES.name)
                .map(|effective| effective.layer),
            Some(run.id().clone()),
        )
        .map_err(|error| error.to_string())?;
        let config = TransportConfig {
            trust,
            exceptions: exceptions_of(run, settings),
            client_certificates: Some(client_certificates),
            proxy: None,
            resolve: settings.hosts(NETWORK_RESOLVE.name),
            egress: egress.clone(),
            clock: run.clock(),
            resolver: ResolverConfiguration::system().map_err(|error| error.to_string())?,
            local,
        };
        let reqwest = Reqwest::new(config.clone()).map_err(|error| error.to_string())?;
        let addresses = reqwest.address_check();
        let transport: Arc<dyn Transport> = match &self.served.transport {
            Some(transport) => Arc::clone(transport),
            None => Arc::new(reqwest),
        };
        Ok(HttpTier {
            addresses,
            transport,
            egress,
            bounds: FetchBounds::of(settings),
            redundant: Arc::new(Redundant::of(settings)),
            following: Following::of(settings),
            sessions: Arc::new(SessionTransports::new(
                config,
                settings
                    .count(SESSION_POOL_MAX_SIZE.name)
                    .and_then(|size| usize::try_from(size).ok())
                    .unwrap_or(usize::MAX),
            )),
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
    /// egress check refuses.
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
    ) -> Result<PageId, Stop> {
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
        let on = (&clock, run.owner(), run.metrics().counted_work());
        let fetched = follow(hop, &tier.following, &gate, on, |hop| {
            let lead = std::mem::replace(&mut first, false);
            let (accepts, clock) = (&accepts, &clock);
            async move {
                if lead {
                    tier.redundant
                        .fetch(
                            tier.transport.as_ref(),
                            &tier.bounds,
                            run,
                            (&hop, Some(accepts)),
                            requester,
                        )
                        .await
                } else {
                    fetch_accepting(
                        tier.transport.as_ref(),
                        &hop,
                        &tier.bounds,
                        clock,
                        Some(accepts),
                    )
                    .await
                }
            }
        })
        .await?;
        if let Some(refused) = &fetched.unaccepted {
            return Err(Stop::Skipped(Skip {
                reason: SkipReason::ContentType,
                message: format!(
                    "`{}` answered `{refused}`, which its Label's `crawl.content_types` does not list: its body was not read.",
                    fetched.url
                ),
            }));
        }
        self.open_served(tier, run, choice, fetched, (hand_over, hand_over.encoding))
            .await
    }

    /// Opens a page served from `fetched` on `tier` for `run`, its calls
    /// served as `choice` says and gated by `hand_over`, the attempt's, its
    /// document read with `forced` from `fetch.encoding` and its formats
    /// with the hand-over's extractor, its Label's `content.extractor`.
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
    /// blocking pool, runs to its end.
    async fn open_served(
        &self,
        tier: &HttpTier,
        run: &Run,
        choice: &TierChoice,
        fetched: crate::driver::Fetched,
        (hand_over, forced): (&Arc<HandOver>, Option<&'static Encoding>),
    ) -> Result<PageId, Stop> {
        let status = fetched.status;
        let (url, headers, body) = (
            fetched.url.clone(),
            fetched.headers.clone(),
            fetched.body.clone(),
        );
        let certificate_exception = fetched.certificate_exception.as_ref().map(mark);
        // The document is decoded and parsed on the blocking pool, never on
        // an async worker (STYLE.md).
        let opened = {
            let (reading, extractor) = (run.clone(), hand_over.extractor);
            run.owner()
                .blocking(move || Opened::of(&reading, fetched, (forced, extractor)))
                .await
        };
        // A document only a guess could read is never served: under a
        // forced tier it fails, and under `auto` its page escalates before
        // its handler runs (ADR 0119). A page holding no document is never
        // decoded.
        let escalated = match (&opened.read, &choice.forced) {
            _ if opened.not_document.is_some() => None,
            (Some(_), _) => None,
            (None, Some(forced)) => {
                return Err(Stop::Failed(undeclared(
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
            ServedPage {
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
                own: Some((response, body)),
                certificate_exception,
                hand_over: Arc::clone(hand_over),
            },
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
            .map_err(Stop::Failed)?;
        let rendered = crate::light_browser::render(
            run,
            &prepared.executable,
            &request.url,
            (tier.address_check(), &tier.bounds.limits),
        )
        .await
        .map_err(Stop::Failed)?;
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
        };
        self.open_served(tier, run, choice, fetched, (hand_over, None))
            .await
    }

    /// Whether a Run's resolved `settings` let its crawl take the
    /// `light_browser` tier: `fetch.light_browser` is on, and this platform
    /// has a pinned Lightpanda build (ADR 0020).
    pub fn light_allowed(settings: &Resolved) -> bool {
        settings.boolean(FETCH_LIGHT_BROWSER.name) == Some(true)
            && crate::light_browser::pinned_here().is_some()
    }

    /// The AI-use signals the response of the served page `page_id`
    /// carried; none once the page closed.
    pub fn use_signals(&self, page_id: &PageId) -> UseSignals {
        lock(&self.served.pages)
            .use_signals
            .get(page_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The response of the served page `page_id`'s crawl request's own
    /// document, whose body `response.body` reads under its `request_id`;
    /// `None` once the page closed.
    pub fn response(&self, page_id: &PageId) -> Option<PageResponse> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .and_then(|page| page.own.as_ref())
            .map(|(response, _)| response.clone())
    }

    /// The type of the served page `page_id`'s response when that is no
    /// HTML document, the page then holding none (ADR 0119); `None` for a
    /// document, or once the page closed.
    pub fn not_document(&self, page_id: &PageId) -> Option<String> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .and_then(|page| page.not_document.clone())
    }

    /// The encoding the served page `page_id`'s document was read in and
    /// where it came from, as Provenance records them (ADR 0119); `None`
    /// for a page holding no document, or once it closed.
    pub fn encoding(&self, page_id: &PageId) -> Option<EncodingProvenance> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            // Lightpanda decoded a light page's document itself, in an
            // encoding the engine does not learn.
            .filter(|page| page.choice.tier != FetchTier::LightBrowser)
            .and_then(|page| page.read)
            .map(|read| EncodingProvenance {
                encoding: read.encoding.name().to_owned(),
                source: Some(read.source.provenance()),
            })
    }

    /// The invalid-certificate exception the host of the served page
    /// `page_id`'s document was reached through, as Provenance marks it
    /// (ADR 0042, pitfall H03); `None` when its certificate verified, or
    /// once it closed.
    pub fn certificate_exception(&self, page_id: &PageId) -> Option<CertificateExceptionMark> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .and_then(|page| page.certificate_exception.clone())
    }

    /// The URL, the page signature and the `rel=canonical` of the served
    /// page `page_id`, which the Frontier learns from (ADRs 0005 and
    /// 0120), read on its Run's blocking owner; `None` once the page
    /// closed, or for a page holding no document, which teaches nothing.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn signature(&self, page_id: &PageId) -> Option<Signed> {
        if self.served_page(page_id)?.not_document.is_some() {
            return None;
        }
        let served = self.document(page_id)?;
        let links = lock(&self.served.pages)
            .links
            .get(page_id)
            .cloned()
            .unwrap_or_default();
        let run = served.run.clone();
        let work = run.metrics().counted_work().clone();
        let signed = run
            .owner()
            .blocking(move || {
                let signature = goodfetch_extractor::page_signature(
                    &goodfetch_extractor::Document {
                        html: &served.html,
                        url: &served.url,
                        base_url: &served.base_url,
                        serialization: goodfetch_extractor::Serialization::Served,
                    },
                    &work,
                );
                let canonical = match (Url::parse(&served.base_url), Url::parse(&served.url)) {
                    (Ok(base), Ok(url)) => canonical(&served.html, &links, &base, &url, &work),
                    _ => None,
                };
                Signed {
                    url: served.url,
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
    ) -> Option<Result<ServedLinks, LinksUnread>> {
        let opened = {
            let pages = lock(&self.served.pages);
            pages.open.get(page_id).map(|page| {
                let encoding = pages
                    .encodings
                    .get(page_id)
                    .copied()
                    .unwrap_or(encoding_rs::UTF_8);
                (Arc::clone(&page.document), encoding)
            })
        };
        let Some((served, encoding)) = opened else {
            return self.browser_links(page_id, selector).await;
        };
        let selector = selector.to_owned();
        let run = served.run.clone();
        let work = run.metrics().counted_work().clone();
        let read = run
            .owner()
            .blocking(move || {
                matched(&served.html, &selector, &work).map(|elements| ServedLinks {
                    url: served.url.clone(),
                    base_url: served.base_url.clone(),
                    encoding,
                    elements,
                })
            })
            .await;
        Some(read.map_err(LinksUnread::Selector))
    }

    /// The first element each of `locators` finds in the page `page_id`'s
    /// document: a served page's, read on its Run's blocking owner, by CSS
    /// selectors and `XPath` 1.0 expressions alike, as a browser reads them;
    /// a browser page's, read by the page itself (ADR 0123). `None` when no
    /// tier serves that page, as a closed one.
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
        locators: &[FieldLocator],
    ) -> Option<Result<FieldElements, CallError>> {
        let served = {
            let pages = lock(&self.served.pages);
            pages
                .open
                .get(page_id)
                .map(|page| Arc::clone(&page.document))
        };
        let Some(served) = served else {
            return self.browser_field_elements(page_id, locators).await;
        };
        let locators = locators.to_vec();
        let run = served.run.clone();
        let work = run.metrics().counted_work().clone();
        let read = run
            .owner()
            .blocking(move || {
                document::first_elements(&served.html, &locators, &work).map(|elements| {
                    FieldElements {
                        base_url: served.base_url.clone(),
                        elements,
                    }
                })
            })
            .await;
        Some(read.map_err(|invalid| {
            CallError::protocol(ProtocolFailure::InvalidParams, invalid.to_string())
        }))
    }

    /// The first element each of `locators` finds in the browser page
    /// `page_id`'s document, as [`FetchTiers::field_elements`] reads them;
    /// `None` for a page no browser holds.
    async fn browser_field_elements(
        &self,
        page_id: &PageId,
        locators: &[FieldLocator],
    ) -> Option<Result<FieldElements, CallError>> {
        let pages = self.served.browser.get()?;
        let answer = match pages
            .read(
                page_id,
                document::FIELDS_READ,
                serde_json::json!({ "locators": locators }),
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
            document::browser_fields(&answer)
                .map(|(base_url, elements)| FieldElements { base_url, elements })
                .ok_or_else(|| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        "the browser page's read of a code-free crawl's fields answered no list of elements",
                    )
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
    ) -> Option<Result<ServedLinks, LinksUnread>> {
        let pages = self.served.browser.get()?;
        if let Err(invalid) = document::check_selector(selector) {
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
            ..
        } = page;
        let run = document.run.clone();
        let check = check.clone();
        let url = document.url.clone();
        let headers = own
            .map(|(response, _)| response.headers)
            .unwrap_or_default();
        let work = run.metrics().counted_work().clone();
        let (verdict, noindex) = run
            .owner()
            .blocking(move || {
                let verdict = expected_page::judge(&check, &document.url, &document.html, &work);
                let noindex = verdict == Verdict::NotExpected
                    && expected_page::noindex(&headers, &document.html);
                (verdict, noindex)
            })
            .await;
        Some(match verdict {
            Verdict::Expected => Judged::Passed,
            Verdict::LoginPage => Judged::Failed(expected_page::logged_out(&url, status)),
            Verdict::NotExpected | Verdict::Unjudged if choice.forced.is_none() => {
                Judged::Escalated
            }
            Verdict::NotExpected => Judged::Failed(expected_page::failed(&url, status, noindex)),
            Verdict::Unjudged => match unserved(&choice, PAGE_CHECK_CALL, false) {
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
        let response = page.own.map(|(response, _)| response)?;
        let run = page.document.run.clone();
        self.classified(&run, response, page.document).await
    }

    /// The Block cause of `document`, which `response` answered for
    /// `run`, as the Product's Block classification tells it (ADR 0035),
    /// with the `blocked` its crawl request ends with when nothing moves
    /// it, read on the blocking pool. `None` for a document no rule
    /// classifies, or before the engine's pages were handed over.
    async fn classified(
        &self,
        run: &Run,
        response: PageResponse,
        document: Arc<ServedDocument>,
    ) -> Option<(Block, CallError)> {
        let pages = Arc::clone(self.served.browser.get()?);
        let received = run.clock().wall();
        let (block, response) = run
            .owner()
            .blocking(move || {
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
    /// `closed`.
    pub fn close(&self, page_id: &PageId) {
        let mut pages = lock(&self.served.pages);
        pages.open.remove(page_id);
        pages.links.remove(page_id);
        pages.encodings.remove(page_id);
        pages.use_signals.remove(page_id);
        pages.replays.remove(page_id);
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

    /// The call that escalated the served page `page_id` to the browser,
    /// the first its tier could not serve exactly under `auto`; `None`
    /// while none did, or once the page closed.
    pub fn escalated(&self, page_id: &PageId) -> Option<&'static str> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .and_then(|page| page.escalated)
    }

    /// Opens Collie's page on the browser tier for a handler of the live
    /// Run `run_id`, in the Session of the crawl's Session pool that
    /// serves the handler, `pooled`, its own browser context, for the call
    /// `context`, whose
    /// Connection's Client then reaches it, and navigates it to `url`
    /// through its navigation gate: the attempt hands its Run's pages over
    /// first, so the page takes its value. Answers its ID and the
    /// navigation's answer.
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
        run_id: &RunId,
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
        pages.open(run_id, pooled, url, context).await
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
    fn served_page(&self, page_id: &PageId) -> Option<ServedPage> {
        lock(&self.served.pages).open.get(page_id).cloned()
    }

    /// The hand-over of the attempt that handed `run`'s pages over last.
    fn handed_over(&self, run: &RunId) -> Option<Arc<HandOver>> {
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
            page.requests += 1;
            page.requests
        });
        format!("{}-request-{number}", page_id.0)
    }

    /// Keeps `body`, the response body of the served page `page_id`'s
    /// request `request_id`, for `response.body`.
    fn keep_body(&self, page_id: &PageId, request_id: &str, body: BodyHandle) {
        if let Some(page) = lock(&self.served.pages).open.get_mut(page_id) {
            page.bodies.insert(request_id.to_owned(), body);
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
                page.escalated = Some(first);
                error
            }
            Unserved::Refused(error) => error,
        }
    }
}

impl HandlerPages for FetchTiers {
    /// The value of the attempt that handed its Run's pages over last,
    /// while its handler runs; else a program's page's, empty.
    fn value(&self, run: &RunId, session: &SessionId) -> PageValue {
        let handed = lock(&self.served.handed)
            .by_run
            .get(run)
            .and_then(|attempts| attempts.last())
            .map(|(_, hand_over)| Arc::clone(hand_over));
        match handed {
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
            .run
            .settings()
            .map(|settings| Following::of(settings.resolved()).handled)
            .unwrap_or_default();
        Some(Reader::new(reading::BrowserReader {
            served: Arc::clone(&self.served),
            hand_over,
            handled,
        }))
    }

    fn document(&self, page_id: &PageId) -> Option<ServedDocument> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .map(|page| ServedDocument::clone(&page.document))
    }

    fn call(
        &self,
        page_id: &PageId,
        operation: &'static str,
        params: &serde_json::Value,
        _context: &CallContext,
    ) -> Option<GuardAnswer> {
        calls::on_browser(self, page_id, operation, params)
            .or_else(|| calls::answer(self, page_id, operation, params))
    }

    fn opened(&self, pages: Arc<dyn BrowserPages>) {
        // A second composition of the same value keeps the first's pages.
        let _ = self.served.browser.set(pages);
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
    fn new(config: TransportConfig, most: usize) -> Self {
        Self {
            config,
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
        .map_err(|error| error.to_string())?;
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
    /// The tier as a Session holding the local addresses `local` sends on
    /// it: its connections and DNS queries leave from them (ADR 0044);
    /// the tier itself for `None`, a Session sending from the Run's.
    ///
    /// # Errors
    ///
    /// A message when its transport cannot be built.
    pub fn from_session(&self, local: Option<&LocalSource>) -> Result<HttpTier, String> {
        let Some(local) = local else {
            return Ok(self.clone());
        };
        if self.sessions.config.local.as_ref() == Some(local) {
            return Ok(self.clone());
        }
        let (transport, addresses) = self.sessions.of(local)?;
        Ok(HttpTier {
            transport,
            addresses,
            ..self.clone()
        })
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
                Err(error) => return Answer::Unreachable(error.message().to_owned()),
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
/// could not serve; or `race`, the winner of a tier race.
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
    goodfetch_page::provenance(run, &entry).await;
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
    /// How long after browser-fetch started the navigation was to start.
    pub after: Duration,
    /// How browser-fetch ended.
    pub browser_fetch: RaceEnd,
    /// How the navigation ended.
    pub navigation: RaceEnd,
}

/// Records in `run`'s Provenance how a tier race ran (ADR 0040): its
/// delay, how each run ended, and which won, `browser_fetch`, `browser`
/// or none.
///
/// # Cancel safety
///
/// Dropping the future never stops the line being written once it began.
pub async fn record_race(run: &Run, race: &Race<'_>) {
    let winner = if race.browser_fetch == RaceEnd::Won {
        Some(FetchTier::BrowserFetch.as_str())
    } else if race.navigation == RaceEnd::Won {
        Some(FetchTier::Browser.as_str())
    } else {
        None
    };
    goodfetch_page::provenance(
        run,
        &serde_json::json!({
            "action": TIER_RACE_ACTION,
            "crawl_request_id": race.request_id,
            "label": race.label,
            "after_ms": u64::try_from(race.after.as_millis()).unwrap_or(u64::MAX),
            "browser_fetch": race.browser_fetch.as_str(),
            "navigation": race.navigation.as_str(),
            "winner": winner,
        }),
    )
    .await;
}

/// Refuses a `fetch.redirect.max` outside 1 to 19, as many redirects as
/// the browser follows (ADR 0118), with `setting_invalid`.
fn refuse_redirect_max(settings: &Resolved) -> Result<(), CallError> {
    let Some(effective) = settings.get(FETCH_REDIRECT_MAX.name) else {
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
    let effective = settings.get(FETCH_TIER.name)?;
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
        Stop::Failed(CallError::new(
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

/// The `href` of the document's first `<base>` element that has one.
fn base_href(html: &str, work: &CountedWork) -> Option<String> {
    let document = work.parse_html(Row::PageParses, html);
    let base = document.select("base[href]");
    base.attr("href").map(|href| href.to_string())
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
        owner: Arc::new(run.owner().child()),
        clock: run.clock(),
    };
    CertificateExceptions::listed(
        settings.value(TLS_ACCEPT_INVALID_CERTS.name),
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
