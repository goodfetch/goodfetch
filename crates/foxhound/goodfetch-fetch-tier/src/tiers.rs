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
//! A served page's document is read in its BOM's encoding, else its
//! `Content-Type` charset's, else as UTF-8 (the `document` module), until
//! #764 builds the rest of ADR 0119's charset order; the links a locator
//! matches in it are read for `enqueueLinks` with that encoding, in which
//! their queries are percent-encoded (ADR 0120).
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

use bytes::Bytes;
use encoding_rs::Encoding;
use goodfetch_engine::dns::ResolverConfiguration;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::robots_txt::{Answer, MAX_SIZE};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tls::Trust;
use goodfetch_engine::{CallContext, CallError, GuardAnswer};
use goodfetch_page::page_value::{
    BrowserPages, HandlerPages, Observer, PageValue, PooledSession, ProgramPages, ServedDocument,
};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::fetch::{
    FETCH_ENCODING, FETCH_HEDGE_ENABLED, FETCH_REDIRECT_MAX, FETCH_TIER, FETCH_TIER_RACE,
    FetchTier, REDIRECT_MAX_LIMIT,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{BodyHandle, Navigated, PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, NETWORK_RESOLVE, SETTING_INVALID,
    SettingInvalidDetails, SettingsLayer,
};
use url::Url;

use crate::canonical::canonical;
use crate::chain::{Following, Stop, follow};
use crate::charset::{self, Kind, Read, content_type};
use crate::choice::{
    ENCODING_CALL, LabelSettings, SetTier, TierChoice, Unserved, undeclared, unserved,
};
use crate::document::{Matched, SelectorInvalid, matched};
use crate::driver::{Deadlines, Hop, Transport, fetch};
use crate::expected_page::{self, Verdict};
use crate::gate::{Admitted, ChainGate, Gate, HandOver, Lease, RobotsTxtFetch};
use crate::redundant::{Redundant, Requester};
use crate::sniffing::sniff;
use crate::transports::{AddressCheck, BrowserFetch, Reqwest, TransportConfig};
use crate::use_signals::UseSignals;

mod calls;
mod sitemap;

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
    document: ServedDocument,
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
}

/// A document a fetch answered, read as a served page holds it: its text,
/// its encoding, its `Link` header fields and its AI-use signals.
struct Opened {
    document: ServedDocument,
    encoding: &'static Encoding,
    /// The encoding ADR 0119's order found, and where; `None` when only a
    /// guess could read the document, its text then read as UTF-8 and
    /// never served.
    read: Option<Read>,
    links: Vec<(String, String)>,
    use_signals: UseSignals,
}

impl Opened {
    /// The document `fetched` answered for `run`, decoded in the encoding
    /// ADR 0119's order finds, `forced` being `fetch.encoding`'s: a
    /// response with no `Content-Type` is typed as Chromium's MIME sniffer
    /// types it.
    fn of(run: &Run, fetched: crate::driver::Fetched, forced: Option<&'static Encoding>) -> Self {
        let url = fetched.url.to_string();
        let (essence, charset) = fetched
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map_or_else(
                || (sniff(&fetched.body, fetched.url.path()).to_owned(), None),
                |(_, value)| content_type(value),
            );
        let read = charset::read(&fetched.body, Kind::of(&essence), charset, forced);
        let (html, encoding) = match read {
            Some(read) => {
                tracing::debug!(run_id = %run.id(), url, encoding = read.encoding.name(), source = read.source.as_str(), "a document's encoding was found");
                (charset::decode(&fetched.body, read), read.encoding)
            }
            None => (
                String::from_utf8_lossy(&fetched.body).into_owned(),
                encoding_rs::UTF_8,
            ),
        };
        let base_url = goodfetch_frontier::document_base(&url, base_href(&html).as_deref())
            .map_or_else(|| url.clone(), |base| base.to_string());
        let use_signals = UseSignals::of(&fetched.headers);
        let links = fetched
            .headers
            .into_iter()
            .filter(|(name, _)| name == "link")
            .collect();
        Self {
            document: ServedDocument {
                run: run.clone(),
                url,
                base_url,
                html,
            },
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
    /// `capability_unavailable` naming the setting, one that hedges
    /// where a hedge could never start, and one that sets `fetch.tier_race`
    /// or `fetch.encoding`, neither built yet.
    ///
    /// # Errors
    ///
    /// `capability_unavailable` for `discovered_api`, `browser_fetch` and
    /// `light_browser`, for `fetch.tier_race` switched on and for a
    /// `fetch.encoding` set; `setting_invalid` for a Run's `fetch.encoding`
    /// that is no WHATWG Encoding label and for a `fetch.redirect.max`
    /// outside 1 to 19 (ADR 0118); `setting_not_applicable` for hedges
    /// switched on while a domain admits one request at a time (ADR 0086).
    pub fn check(settings: &Resolved, labels: &LabelSettings) -> Result<(), CallError> {
        goodfetch_frontier::refuse_second_copies(settings, &FETCH_HEDGE_ENABLED)?;
        refuse_redirect_max(settings)?;
        if let Some(effective) = settings
            .get(FETCH_TIER_RACE.name)
            .filter(|effective| effective.value == serde_json::Value::Bool(true))
        {
            return Err(unbuilt(
                FETCH_TIER_RACE.name,
                effective.layer,
                "tier_race",
                "This engine does not race a crawl request's tiers yet, which `fetch.tier_race` asks for in {set}: switch `fetch.tier_race` off.",
                "`fetch.tier_race: false`",
                None,
            ));
        }
        if let Some(effective) = settings.get(FETCH_ENCODING.name) {
            crate::choice::encoding("the Run", &effective.value)?;
        }
        let run = forced(settings).map(|(tier, layer)| (tier, layer, None));
        let by_label = labels
            .forced()
            .map(|(label, tier)| (tier, SettingsLayer::Run, Some(label)));
        for (tier, layer, label) in run.into_iter().chain(by_label) {
            if matches!(tier, FetchTier::Http | FetchTier::Browser) {
                continue;
            }
            let name = tier.as_str();
            let set = label.map_or_else(
                || layer.as_str().to_owned(),
                |label| format!("the Label `{label}`"),
            );
            return Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "This engine fetches no crawl request on the `{name}` tier, which `fetch.tier` forces in {set}: set `fetch.tier` to `auto`, `http` or `browser`."
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
    /// take, and its navigation gate fetches robots.txt with.
    ///
    /// # Errors
    ///
    /// A message when the transport cannot be built: the system's resolver
    /// configuration or the Session's egress policy cannot be read.
    pub fn built(run: &Run, settings: &Resolved) -> Result<HttpTier, String> {
        let egress = Policy::of(settings).map_err(|error| error.to_string())?;
        let transport = Reqwest::new(TransportConfig {
            trust: Trust::default_roots(),
            proxy: None,
            resolve: settings.hosts(NETWORK_RESOLVE.name),
            egress: egress.clone(),
            clock: run.clock(),
            resolver: ResolverConfiguration::system().map_err(|error| error.to_string())?,
        })
        .map_err(|error| error.to_string())?;
        Ok(HttpTier {
            addresses: transport.address_check(),
            transport: Arc::new(transport),
            egress,
            deadlines: Deadlines::of(settings),
            redundant: Arc::new(Redundant::of(settings)),
            following: Following::of(settings),
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
        let hop = Hop {
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
        };
        let gate = ChainGate {
            hand_over: Arc::clone(hand_over),
            crawl_request: request.id.clone(),
        };
        let clock = run.clock();
        let mut first = true;
        let fetched = follow(hop, &tier.following, &gate, &clock, |hop| {
            let lead = std::mem::replace(&mut first, false);
            async move {
                if lead {
                    tier.redundant
                        .fetch(
                            tier.transport.as_ref(),
                            &tier.deadlines,
                            run,
                            &hop,
                            requester,
                        )
                        .await
                } else {
                    fetch(tier.transport.as_ref(), &hop, &tier.deadlines, &clock).await
                }
            }
        })
        .await?;
        let status = fetched.status;
        let opened = Opened::of(run, fetched, hand_over.encoding);
        // A document only a guess could read is never served: under a
        // forced tier it fails, and under `auto` its page escalates before
        // its handler runs (ADR 0119).
        let escalated = match (&opened.read, &choice.forced) {
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
        pages.open.insert(
            page_id.clone(),
            ServedPage {
                history: vec![opened.document.url.clone()],
                at: 0,
                document: opened.document.clone(),
                status,
                http: tier.clone(),
                choice: choice.clone(),
                escalated,
                requests: 0,
                bodies: BTreeMap::new(),
            },
        );
        pages.opened(&page_id, opened);
        Ok(page_id)
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

    /// The URL, the page signature and the `rel=canonical` of the served
    /// page `page_id`, which the Frontier learns from (ADRs 0005 and
    /// 0120), read on its Run's blocking owner; `None` once the page
    /// closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn signature(&self, page_id: &PageId) -> Option<Signed> {
        let served = self.document(page_id)?;
        let links = lock(&self.served.pages)
            .links
            .get(page_id)
            .cloned()
            .unwrap_or_default();
        let run = served.run.clone();
        let signed = run
            .owner()
            .blocking(move || {
                let signature =
                    goodfetch_extractor::page_signature(&goodfetch_extractor::Document {
                        html: &served.html,
                        url: &served.url,
                        base_url: &served.base_url,
                    });
                let canonical = match (Url::parse(&served.base_url), Url::parse(&served.url)) {
                    (Ok(base), Ok(url)) => canonical(&served.html, &links, &base, &url),
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

    /// The elements the CSS selector `selector` matches in the served
    /// page `page_id`'s document, read on its Run's blocking owner;
    /// `None` when no tier serves that page, as a closed one.
    ///
    /// # Errors
    ///
    /// When `selector` is no CSS selector.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only what it reads; the page stays open.
    pub async fn links(
        &self,
        page_id: &PageId,
        selector: &str,
    ) -> Option<Result<ServedLinks, SelectorInvalid>> {
        let (served, encoding) = {
            let pages = lock(&self.served.pages);
            let served = pages.open.get(page_id)?.document.clone();
            let encoding = pages
                .encodings
                .get(page_id)
                .copied()
                .unwrap_or(encoding_rs::UTF_8);
            (served, encoding)
        };
        let selector = selector.to_owned();
        let run = served.run.clone();
        let read = run
            .owner()
            .blocking(move || {
                matched(&served.html, &selector).map(|elements| ServedLinks {
                    url: served.url,
                    base_url: served.base_url,
                    encoding,
                    elements,
                })
            })
            .await;
        Some(read)
    }

    /// Judges the served page `page_id`'s document against `check`, its
    /// Label's Page check, before its handler is given it (ADRs 0006 and
    /// 0093), read on its Run's blocking owner: a page failing its expected
    /// page, or one its markup cannot judge, escalates under `auto` and
    /// fails under a forced tier, with `page_check_failed` or, unjudged,
    /// `browser_required`; a login page fails with `logged_out` on every
    /// tier. `None` once the page closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the verdict; the page stays open.
    pub async fn judge(&self, page_id: &PageId, check: &PageCheck) -> Option<Judged> {
        let page = self.served_page(page_id)?;
        let ServedPage {
            document,
            status,
            choice,
            ..
        } = page;
        let run = document.run.clone();
        let check = check.clone();
        let url = document.url.clone();
        let verdict = run
            .owner()
            .blocking(move || expected_page::judge(&check, &document.url, &document.html))
            .await;
        Some(match verdict {
            Verdict::Expected => Judged::Passed,
            Verdict::LoginPage => Judged::Failed(expected_page::logged_out(&url, status)),
            Verdict::NotExpected | Verdict::Unjudged if choice.forced.is_none() => {
                Judged::Escalated
            }
            Verdict::NotExpected => Judged::Failed(expected_page::failed(&url, status)),
            Verdict::Unjudged => match unserved(&choice, PAGE_CHECK_CALL, false) {
                Unserved::Escalated(_) => Judged::Escalated,
                Unserved::Refused(error) => Judged::Failed(error),
            },
        })
    }

    /// Closes the served page `page_id`: later calls on it answer
    /// `closed`.
    pub fn close(&self, page_id: &PageId) {
        let mut pages = lock(&self.served.pages);
        pages.open.remove(page_id);
        pages.links.remove(page_id);
        pages.encodings.remove(page_id);
        pages.use_signals.remove(page_id);
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
    /// Dropping the future may leave the page open; [`FetchTiers::close_browser`]
    /// closes it.
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

    /// Closes the browser page `page_id` [`FetchTiers::browser`] opened.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the page open, until its Run ends.
    pub async fn close_browser(&self, page_id: &PageId) {
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
        page.document = opened.document.clone();
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
                let page_check = hand_over.page_check.clone();
                PageValue::new(Gate::new(Arc::clone(&hand_over)), Lease::new(hand_over))
                    .with_page_check(page_check)
            }
            None => ProgramPages.value(run, session),
        }
    }

    fn observer(&self, run: &RunId, session: &SessionId) -> Option<Observer> {
        ProgramPages.observer(run, session)
    }

    fn document(&self, page_id: &PageId) -> Option<ServedDocument> {
        lock(&self.served.pages)
            .open
            .get(page_id)
            .map(|page| page.document.clone())
    }

    fn call(
        &self,
        page_id: &PageId,
        operation: &'static str,
        params: &serde_json::Value,
        _context: &CallContext,
    ) -> Option<GuardAnswer> {
        calls::answer(self, page_id, operation, params)
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
    deadlines: Deadlines,
    /// Its Run's hedges and requests in flight, which no other Run shares.
    redundant: Arc<Redundant>,
    /// What its requests' redirect chains follow.
    following: Following,
}

/// How many redirects a robots.txt fetch follows: RFC 9309 §2.3.1.2's
/// least.
const ROBOTS_TXT_REDIRECTS: usize = 5;

impl HttpTier {
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

    /// Fetches the robots.txt at `location` for `run` (ADR 0086), following
    /// up to five redirects as RFC 9309 §2.3.1.2 asks: a `3xx` past them,
    /// or one naming no location, answers as itself, which disallows
    /// everything until a later fetch; a failed fetch answers why.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the fetch's connection.
    pub async fn robots_txt(&self, run: &Run, location: &Url) -> Answer {
        let mut url = location.clone();
        let mut redirects = 0;
        loop {
            let fetched = match fetch(
                self.transport.as_ref(),
                &Hop::get(url.clone()),
                &self.deadlines,
                &run.clock(),
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
                        body: fetched.body.into_iter().take(MAX_SIZE).collect(),
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
}

/// Records in `run`'s Provenance the tier `choice` names for an attempt of
/// the crawl request `request_id` of `label`, and how it was chosen (ADR
/// 0023): `forced`, with who forced it, the Run or the Label; `compared`
/// under `auto`, the sample comparison's choice; `sample`, one run of a
/// sample; or `escalation` to the browser, naming the call the cheaper
/// tier could not serve.
///
/// # Cancel safety
///
/// Dropping the future never stops the line being written once it began.
pub async fn record_tier(
    run: &Run,
    request_id: &str,
    label: &str,
    choice: &TierChoice,
    how: Chosen,
) {
    let (chosen, escalated) = match how {
        Chosen::AsChosen => (choice.chosen(), None),
        Chosen::Sample(call) => ("sample", call),
        Chosen::Escalated(call) => ("escalation", Some(call)),
    };
    let mut entry = serde_json::json!({
        "action": FETCH_TIER_ACTION,
        "crawl_request_id": request_id,
        "label": label,
        "tier": choice.tier.as_str(),
        "chosen": chosen,
    });
    if let Some(forced) = &choice.forced {
        entry["forced_by"] = serde_json::json!(if forced.label.is_some() {
            "label"
        } else {
            "run"
        });
    }
    if let Some(call) = escalated {
        entry["call"] = serde_json::json!(call);
        // The browser reads a document only a guess decodes with its own
        // detector (ADR 0119).
        if call == ENCODING_CALL {
            entry["encoding_source"] = serde_json::json!("detected");
        }
    }
    goodfetch_page::provenance(run, &entry).await;
}

/// `capability_unavailable` for `key`, set in `layer` or by the Label
/// `label`, asking for `kind`, which this engine does not build: `message`
/// with `{set}` naming where it was set, and `fix`.
fn unbuilt(
    key: &str,
    layer: SettingsLayer,
    kind: &str,
    message: &str,
    fix: &str,
    label: Option<&str>,
) -> CallError {
    let set = label.map_or_else(
        || layer.as_str().to_owned(),
        |label| format!("the Label `{label}`"),
    );
    CallError::new(
        &CAPABILITY_UNAVAILABLE,
        message.replace("{set}", &set),
        &CapabilityUnavailableDetails {
            key: key.to_owned(),
            layer,
            kind: kind.to_owned(),
            fix: fix.to_owned(),
        },
    )
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

/// The `href` of the document's first `<base>` element that has one.
fn base_href(html: &str) -> Option<String> {
    let document = dom_query::Document::from(html);
    let base = document.select("base[href]");
    base.attr("href").map(|href| href.to_string())
}
