//! The tiers that serve a crawl request (spec #579; ADRs 0020, 0087 and
//! 0093), and the crate's one composition value, [`FetchTiers`]: the seam
//! Foxhound's composition sets on Collie's pages, and the tiers its crawl
//! serves requests with. Under `fetch.tier: http`, [`HttpTier`] fetches a
//! request's document over reqwest through the fetch driver, and
//! [`FetchTiers::serve`] opens a page served from it, which the handler
//! reads through `page.formats`; every other call on that page answers
//! `closed` until #767 serves or escalates it. Under `auto` the engine
//! fetches nothing yet and the handler is given its request alone (#767,
//! area 6); a forced tier not built yet refuses the crawl's start.
//!
//! A served page's document is read in its BOM's encoding, else its
//! `Content-Type` charset's, else as UTF-8 (the `document` module), until
//! #764 builds the rest of ADR 0119's charset order; the links a locator
//! matches in it are read for `enqueueLinks` with that encoding, in which
//! their queries are percent-encoded (ADR 0120).
#![expect(
    clippy::disallowed_types,
    reason = "the served pages' table is read and written in one step, before any `.await` and never under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use encoding_rs::Encoding;
use goodfetch_engine::CallError;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::robots_txt::{Answer, MAX_SIZE};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tls::Trust;
use goodfetch_page::page_value::{HandlerPages, Observer, PageValue, ProgramPages, ServedDocument};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::fetch::{FETCH_HEDGE_ENABLED, FETCH_TIER, FetchTier};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, NETWORK_RESOLVE, SettingsLayer,
};
use url::Url;

use crate::canonical::canonical;
use crate::document::{Matched, SelectorInvalid, decode, matched};
use crate::driver::{Deadlines, Hop, fetch};
use crate::gate::{Gate, HandOver, Lease, RobotsTxtFetch};
use crate::redundant::{Redundant, Requester};
use crate::transports::{Reqwest, TransportConfig};

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
    open: BTreeMap<PageId, ServedDocument>,
    /// The `Link` header fields of each open page that had any, where
    /// its `rel=canonical` may be.
    links: BTreeMap<PageId, Vec<(String, String)>>,
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
    pub fn hand_over(&self, hand_over: HandOver) -> HandedOver {
        let run = hand_over.run.id().clone();
        let mut handed = lock(&self.served.handed);
        handed.last += 1;
        let number = handed.last;
        handed
            .by_run
            .entry(run.clone())
            .or_default()
            .push((number, Arc::new(hand_over)));
        HandedOver {
            served: Arc::clone(&self.served),
            run,
            number,
        }
    }

    /// Refuses a Run whose `fetch.tier` forces a tier this engine does not
    /// build yet, with `capability_unavailable` naming the setting, and one
    /// that hedges where a hedge could never start.
    ///
    /// # Errors
    ///
    /// `capability_unavailable` for `discovered_api`, `browser_fetch`,
    /// `light_browser` and `browser`; `setting_not_applicable` for hedges
    /// switched on while a domain admits one request at a time (ADR 0086).
    pub fn check(settings: &Resolved) -> Result<(), CallError> {
        goodfetch_frontier::refuse_second_copies(settings, &FETCH_HEDGE_ENABLED)?;
        let Some((tier, layer)) = forced(settings) else {
            return Ok(());
        };
        if tier == FetchTier::Http {
            return Ok(());
        }
        let name = tier.as_str();
        Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            format!(
                "This engine fetches no crawl request on the `{name}` tier, which `fetch.tier` forces in {}: set `fetch.tier` to `auto` or `http`.",
                layer.as_str()
            ),
            &CapabilityUnavailableDetails {
                key: FETCH_TIER.name.to_owned(),
                layer,
                kind: name.to_owned(),
                fix: "`fetch.tier: auto`".to_owned(),
            },
        ))
    }

    /// The HTTP tier a Run's crawl requests take under its settings, one
    /// per Run so its hops share connections; `None` unless `fetch.tier`
    /// is `http`.
    ///
    /// # Errors
    ///
    /// A message when the transport cannot be built: the system's resolver
    /// configuration or the Session's egress policy cannot be read.
    pub fn http(run: &Run, settings: &Resolved) -> Result<Option<HttpTier>, String> {
        if forced(settings).map(|(tier, _)| tier) != Some(FetchTier::Http) {
            return Ok(None);
        }
        Self::built(run, settings).map(Some)
    }

    /// An HTTP tier for `run` under its settings, whatever `fetch.tier`
    /// says: the one a Run's navigation gate fetches robots.txt with when
    /// its crawl requests take no HTTP tier.
    ///
    /// # Errors
    ///
    /// As [`FetchTiers::http`].
    pub fn built(run: &Run, settings: &Resolved) -> Result<HttpTier, String> {
        let egress = Policy::of(settings).map_err(|error| error.to_string())?;
        let transport = Reqwest::new(TransportConfig {
            trust: Trust::default_roots(),
            proxy: None,
            resolve: settings.hosts(NETWORK_RESOLVE.name),
            egress,
            clock: run.clock(),
        })
        .map_err(|error| error.to_string())?;
        Ok(HttpTier {
            transport: Arc::new(transport),
            deadlines: Deadlines::of(settings),
            redundant: Arc::new(Redundant::of(settings)),
        })
    }

    /// Fetches `request`'s document on `tier` for `run`, sent for
    /// `requester`, and opens a page served from it, whose ID the handler
    /// is given. An identical request in flight from its Session answers it
    /// instead, and a slow one is hedged (ADR 0040; the `redundant`
    /// module).
    ///
    /// # Errors
    ///
    /// The fetch driver's: `deadline_exceeded`, `egress_refused` or
    /// `navigation_failed` with its network error.
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
    ) -> Result<PageId, CallError> {
        let url = Url::parse(&request.url).map_err(|error| {
            CallError::new(
                &goodfetch_protocol::page::NAVIGATION_FAILED,
                format!(
                    "The crawl request's URL `{}` does not parse: {error}.",
                    request.url
                ),
                &goodfetch_protocol::page::NavigationFailedDetails {
                    url: Some(request.url.clone()),
                    error: None,
                },
            )
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
        let fetched = tier
            .redundant
            .fetch(
                tier.transport.as_ref(),
                &tier.deadlines,
                run,
                &hop,
                requester,
            )
            .await?;
        let url = fetched.url.to_string();
        let content_type = fetched
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str());
        let (html, encoding) = decode(&fetched.body, content_type);
        let base_url = goodfetch_frontier::document_base(&url, base_href(&html).as_deref())
            .map_or_else(|| url.clone(), |base| base.to_string());
        let links: Vec<(String, String)> = fetched
            .headers
            .into_iter()
            .filter(|(name, _)| name == "link")
            .collect();
        let document = ServedDocument {
            run: run.clone(),
            url,
            base_url,
            html,
        };
        let mut pages = lock(&self.served.pages);
        pages.next += 1;
        let page_id = PageId(format!("document-{}", pages.next));
        pages.open.insert(page_id.clone(), document);
        pages.encodings.insert(page_id.clone(), encoding);
        if !links.is_empty() {
            pages.links.insert(page_id.clone(), links);
        }
        Ok(page_id)
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
            let served = pages.open.get(page_id)?.clone();
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

    /// Closes the served page `page_id`: later calls on it answer
    /// `closed`.
    pub fn close(&self, page_id: &PageId) {
        let mut pages = lock(&self.served.pages);
        pages.open.remove(page_id);
        pages.links.remove(page_id);
        pages.encodings.remove(page_id);
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
                PageValue::new(Gate::new(Arc::clone(&hand_over)), Lease::new(hand_over))
            }
            None => ProgramPages.value(run, session),
        }
    }

    fn observer(&self, run: &RunId, session: &SessionId) -> Option<Observer> {
        ProgramPages.observer(run, session)
    }

    fn document(&self, page_id: &PageId) -> Option<ServedDocument> {
        lock(&self.served.pages).open.get(page_id).cloned()
    }
}

/// The HTTP tier of one Run: its transport, whose connections its hops
/// share, and the engine's timeouts its fetches wait under.
#[derive(Clone)]
pub struct HttpTier {
    transport: Arc<Reqwest>,
    deadlines: Deadlines,
    /// Its Run's hedges and requests in flight, which no other Run shares.
    redundant: Arc<Redundant>,
}

/// How many redirects a robots.txt fetch follows: RFC 9309 §2.3.1.2's
/// least.
const ROBOTS_TXT_REDIRECTS: usize = 5;

impl HttpTier {
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
