//! The tiers that serve a crawl request (spec #579; ADRs 0020, 0087 and
//! 0093), and the crate's one composition value, [`FetchTiers`]: the hook
//! Foxhound's composition sets on Collie's pages, and the tiers its crawl
//! serves requests with. Under `fetch.tier: http`, [`HttpTier`] fetches a
//! request's document over reqwest through the fetch driver, and
//! [`FetchTiers::serve`] opens a page served from it, which the handler
//! reads through `page.formats`; every other call on that page answers
//! `closed` until #767 serves or escalates it. Under `auto` the engine
//! fetches nothing yet and the handler is given its request alone (#767,
//! area 6); a forced tier not built yet refuses the crawl's start.
//!
//! A served page's document is read as UTF-8, a byte that is none replaced,
//! until #764 builds ADR 0119's charset order.
#![expect(
    clippy::disallowed_types,
    reason = "the served pages' table is read and written in one step, before any `.await` and never under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use goodfetch_engine::CallError;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tls::Trust;
use goodfetch_page::page_value::{
    HandlerPages, PageValue, ProgramPages, ResponseObserver, ServedDocument,
};
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::fetch::{FETCH_HEDGE_ENABLED, FETCH_TIER, FetchTier};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, NETWORK_RESOLVE, SettingsLayer,
};
use url::Url;

use crate::driver::{Deadlines, Hop};
use crate::redundant::{Redundant, Requester};
use crate::transports::{Reqwest, TransportConfig};

/// Foxhound's fetch tiers: the pages they serve, which Collie's page
/// crate reads through the hook [`FetchTiers::handler_pages`] gives, and
/// the tier each Run's crawl requests take. Cheap to clone; every clone
/// shares the pages.
#[derive(Clone, Default)]
pub struct FetchTiers {
    served: Arc<Served>,
}

#[derive(Default)]
struct Served {
    pages: Mutex<Pages>,
}

#[derive(Default)]
struct Pages {
    /// The number of the next page served.
    next: u64,
    open: BTreeMap<PageId, ServedDocument>,
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

    /// The hook Foxhound's composition gives Collie's page crate (ADR
    /// 0087): the pages the tiers serve, with the empty value and no
    /// response observer of a program's pages, which Foxhound's gate fills
    /// later (area 5).
    pub fn handler_pages(&self) -> Arc<dyn HandlerPages> {
        Arc::new(self.clone())
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
        let egress = Policy::of(settings).map_err(|error| error.to_string())?;
        let transport = Reqwest::new(TransportConfig {
            trust: Trust::default_roots(),
            proxy: None,
            resolve: settings.hosts(NETWORK_RESOLVE.name),
            egress,
            clock: run.clock(),
        })
        .map_err(|error| error.to_string())?;
        Ok(Some(HttpTier {
            transport: Arc::new(transport),
            deadlines: Deadlines::of(settings),
            redundant: Arc::new(Redundant::of(settings)),
        }))
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
        let html = String::from_utf8_lossy(&fetched.body).into_owned();
        let base_url = goodfetch_frontier::document_base(&url, base_href(&html).as_deref())
            .map_or_else(|| url.clone(), |base| base.to_string());
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
        Ok(page_id)
    }

    /// The URL and the page signature of the served page `page_id`, which
    /// the Frontier learns from (ADR 0005), read on its Run's blocking
    /// owner; `None` once the page closed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the signature; the page stays open.
    pub async fn signature(&self, page_id: &PageId) -> Option<(String, [u8; 32])> {
        let served = self.document(page_id)?;
        let run = served.run.clone();
        let url = served.url.clone();
        let signature = run
            .owner()
            .blocking(move || {
                goodfetch_extractor::page_signature(&goodfetch_extractor::Document {
                    html: &served.html,
                    url: &served.url,
                    base_url: &served.base_url,
                })
            })
            .await;
        Some((url, signature))
    }

    /// Closes the served page `page_id`: later calls on it answer
    /// `closed`.
    pub fn close(&self, page_id: &PageId) {
        lock(&self.served.pages).open.remove(page_id);
    }
}

impl HandlerPages for FetchTiers {
    fn value(&self, run: &RunId, session: &SessionId) -> PageValue {
        ProgramPages.value(run, session)
    }

    fn observer(&self, run: &RunId, session: &SessionId) -> Option<Arc<dyn ResponseObserver>> {
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
