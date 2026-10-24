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
//! A served page's document is read as UTF-8, a byte that is none replaced,
//! until #764 builds ADR 0119's charset order.
#![expect(
    clippy::disallowed_types,
    reason = "the served pages' table is read and written in one step, before any `.await` and never under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use bytes::Bytes;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::robots_txt::{Answer, MAX_SIZE};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tls::Trust;
use goodfetch_engine::{CallContext, CallError, GuardAnswer};
use goodfetch_page::page_value::{
    BrowserPages, HandlerPages, Observer, PageValue, ProgramPages, ServedDocument,
};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::fetch::{FETCH_HEDGE_ENABLED, FETCH_TIER, FetchTier};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{BodyHandle, Navigated, PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, NETWORK_RESOLVE, SettingsLayer,
};
use url::Url;

use crate::canonical::canonical;
use crate::choice::{LabelTiers, TierChoice, Unserved, unserved};
use crate::driver::{Deadlines, Hop, fetch};
use crate::gate::{Gate, HandOver, Lease, RobotsTxtFetch};
use crate::redundant::{Redundant, Requester};
use crate::transports::{Reqwest, TransportConfig};

mod calls;

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
}

/// A page served from a document: the document, the tier that fetched it
/// and how that tier was chosen, the call that escalated it, if one did,
/// and the bodies of the responses its `page.request`s got.
#[derive(Clone)]
struct ServedPage {
    document: ServedDocument,
    http: HttpTier,
    choice: TierChoice,
    /// The first call its tier could not serve, under `auto`.
    escalated: Option<&'static str>,
    /// Its `page.request`s so far.
    requests: u64,
    bodies: BTreeMap<String, BodyHandle>,
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

    /// Refuses a Run whose `fetch.tier`, its own or one of its Labels' in
    /// `labels`, forces a tier this engine does not build yet, with
    /// `capability_unavailable` naming the setting, and one that hedges
    /// where a hedge could never start.
    ///
    /// # Errors
    ///
    /// `capability_unavailable` for `discovered_api`, `browser_fetch` and
    /// `light_browser`; `setting_not_applicable` for hedges switched on
    /// while a domain admits one request at a time (ADR 0086).
    pub fn check(settings: &Resolved, labels: &LabelTiers) -> Result<(), CallError> {
        goodfetch_frontier::refuse_second_copies(settings, &FETCH_HEDGE_ENABLED)?;
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

    /// The tier the Run's own `fetch.tier` forces, if it forces one.
    pub fn run_forced(settings: &Resolved) -> Option<FetchTier> {
        forced(settings).map(|(tier, _)| tier)
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
    /// is given, its calls served as `choice` says. An identical request in
    /// flight from its Session answers it instead, and a slow one is
    /// hedged (ADR 0040; the `redundant` module).
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
        choice: &TierChoice,
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
        let html = String::from_utf8_lossy(&fetched.body).into_owned();
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
        pages.open.insert(
            page_id.clone(),
            ServedPage {
                document,
                http: tier.clone(),
                choice: choice.clone(),
                escalated: None,
                requests: 0,
                bodies: BTreeMap::new(),
            },
        );
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

    /// Closes the served page `page_id`: later calls on it answer
    /// `closed`.
    pub fn close(&self, page_id: &PageId) {
        let mut pages = lock(&self.served.pages);
        pages.open.remove(page_id);
        pages.links.remove(page_id);
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
    /// Run `run_id`, in its default Session, for the call `context`, whose
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
        url: &str,
        context: &CallContext,
    ) -> Result<(PageId, Navigated), CallError> {
        let Some(pages) = self.served.browser.get() else {
            return Err(CallError::protocol(
                ProtocolFailure::InternalError,
                "the engine's composition handed the fetch tiers no browser pages",
            ));
        };
        pages.open(run_id, url, context).await
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
                PageValue::new(Gate::new(Arc::clone(&hand_over)), Lease::new(hand_over))
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

/// Provenance's action for the tier a crawl request's attempt ran on.
const FETCH_TIER_ACTION: &str = "fetch_tier";

/// Records in `run`'s Provenance the tier `choice` names for an attempt of
/// the crawl request `request_id` of `label`, and how it was chosen (ADR
/// 0023): `forced`, with who forced it, the Run or the Label; `engine`
/// under `auto`; or `escalation` to the browser, naming the call
/// `escalated` that the cheaper tier could not serve.
///
/// # Cancel safety
///
/// Dropping the future never stops the line being written once it began.
pub async fn record_tier(
    run: &Run,
    request_id: &str,
    label: &str,
    choice: &TierChoice,
    escalated: Option<&'static str>,
) {
    let mut entry = serde_json::json!({
        "action": FETCH_TIER_ACTION,
        "crawl_request_id": request_id,
        "label": label,
        "tier": choice.tier.as_str(),
        "chosen": if escalated.is_some() { "escalation" } else { choice.chosen() },
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
    }
    goodfetch_page::provenance(run, &entry).await;
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
