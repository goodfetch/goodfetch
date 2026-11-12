//! What Foxhound sets on a handler's pages (spec #579's `gate`; ADRs 0086,
//! 0087, 0093, 0118 and 0121): Collie's `NavigationGate`, filled with the
//! per-domain Politeness and the robots.txt of the host each navigation
//! goes to, and Collie's `LeaseCheck`, the attempt's own.
//!
//! While an attempt's handler runs, the attempt hands its Run's pages over
//! ([`FetchTiers::hand_over`](crate::FetchTiers::hand_over)): every page
//! that opens in the Run's Sessions meanwhile takes the value of the
//! attempt handed over last, and every page opened from it takes its
//! opener's. The gate then admits each navigation, redirect hop,
//! `page.request` and service worker's navigation request of such a page:
//!
//! - **Its permit.** A request to the attempt's own domain key goes under
//!   the Permit the attempt's admission holds, its document having arrived
//!   before the handler ran, so the domain keeps one request in flight; any
//!   other domain key takes a Permit of its own beside the Run's requests,
//!   as a hedge's second copy does (ADR 0040), waiting while the domain is
//!   full or its delay has not passed, and given back once the permit is
//!   released.
//! - **robots.txt.** The host's robots.txt is read through the copy the
//!   user's runs share (ADR 0086), fetched under that permit when the copy
//!   has none to use, and judged, as the use signals below, by the one
//!   judge of a site's preferences (`site_preferences`); the groups for
//!   `goodfetch-foxhound` and for
//!   `site_preferences.robots_txt_user_agent` apply, merged, else `*`'s
//!   (ADR 0090). A URL they disallow is refused with `robots_txt`, and a
//!   host whose Crawl-delay is above `site_preferences.max_crawl_delay`
//!   with `crawl_delay`. With `site_preferences.respect_robots_txt` off,
//!   its rules are not applied, and it is read only for its use signals.
//! - **Its host's declared use signals.** While
//!   `site_preferences.respect_use_signals` is on, a request whose host
//!   declared, before it is sent, that the Run's `site_preferences.purpose`
//!   is refused is refused with `use_signals` and never leaves: the
//!   `Content-Signal` lines of its robots.txt's groups that apply, and
//!   `TDMRep`'s well-known file, fetched once per origin for the Run, as a
//!   crawl request is judged before it is sent (ADR 0093).
//! - **Its document's use signals.** While
//!   `site_preferences.respect_use_signals` is on, a main-frame document
//!   whose response's AI-use signals refuse the Run's
//!   `site_preferences.purpose` is refused with `use_signals`, and the page
//!   is left on a blank document: `TDMRep`'s `tdm-reservation` header or
//!   `<meta>`, aipref's `Content-Usage` header, and the RSL license its
//!   `Link` header or `<link>` element names, fetched once for the Run
//!   through its shared signal files, as the crawl request's own document
//!   is judged. A page a document tier serves is judged the same way, its
//!   navigations' documents and its `page.request`s' responses alike.
//! - **The delay between starts.** The request waits until the domain's
//!   last start, by any sharing run, is its Crawl-delay ago, and on its
//!   own domain key, or after a robots.txt it fetched, also
//!   `politeness.domain.delay`. There, its start is taken in the
//!   Politeness file at once, so two handlers' requests never read the
//!   same last start and go together; on another key, its Permit's rule
//!   already spaced it, and its start is recorded for every run sharing the
//!   domain.
//!
//! The page holds the lease through every wait (`LeaseCheck::permit_wait`),
//! which the crawl renews and names in the Run's live status. A request of
//! no HTTP(S) URL goes ungated.
//!
//! The same gate admits each hop after the first of a crawl request's
//! redirect chain on a document tier ([`ChainGate`]; ADR 0118), after the
//! Frontier's checks of the hop, in this order: a hop leaving the crawl's
//! scope ends the request `skipped` with `out_of_scope`, one to a host the
//! Run's domain policy refuses with `domain_policy`, and one to a key the
//! Frontier admitted for another request with `duplicate` (ADRs 0093, 0118
//! and 0120). A hop whose URL names an address a page may not reach in
//! place of a host, or whose name resolves to one, ends it `skipped` with
//! `domain_policy` too, before the gate reads its robots.txt (ADR 0017); a
//! refusal of the gate's ends it `skipped` with its reason, but for a
//! robots.txt that could not be read, with no earlier copy: the crawl
//! request is then retried as one whose own robots.txt could not be read
//! ([`Stop::RobotsTxtUnreachable`]).
//!
//! Size: one concern, the gate of an attempt's pages and of its document's
//! redirect hops, with what it judges them by: the value the attempt hands
//! over, the robots.txt groups that apply and the AI-use signals declared.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::Instant;

use bytes::Bytes;
use goodfetch_counted_work::Row;
use goodfetch_engine::CallError;
use goodfetch_engine::admission::{Room, SecondCopies, SecondCopy};
use goodfetch_engine::egress::{Policy, Refused};
use goodfetch_engine::governor::Permit as HttpAdmission;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_engine::politeness::{Politeness, PolitenessError};
use goodfetch_engine::robots_txt::{self, Fetch};
use goodfetch_engine::runs::Run;
use goodfetch_frontier::DomainKeys;
use goodfetch_page::page_value::{
    Admission, DocumentJudgment, DocumentResponse, Navigation, NavigationGate, Permit, Refusal,
    Sending,
};
use goodfetch_protocol::foxhound::crawl::RESPONSE_UNAVAILABLE;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::settings::page_check::PageCheck;
use serde_json::json;
use url::Url;

use crate::chain::{Gating, HopGate, Skip, Stop};
use crate::content_types::ContentTypes;
use crate::discovered_api::Capture;
use crate::driver::{Hop, judged};
use crate::site_preferences::{self, AfterRobotsTxt, Reading, RefusedBy, Request, SitePreferences};
use crate::transports::AddressCheck;
use crate::use_signals::UseSignals;

/// Fetches the robots.txt at a location for a Run, as the crawl's HTTP
/// tier does (`HttpTier::robots_txt_fetch`).
pub type RobotsTxtFetch = Arc<dyn Fn(Run, Url) -> Fetch<'static> + Send + Sync>;

/// How long a request beside the Run's own whose domain is full waits
/// before it asks for room again: the Politeness file wakes only the
/// admission's own waiters.
const FULL_RETRY: Duration = Duration::from_millis(250);

/// The second copy `room` gave a request beside the Run's own, or when it
/// asks for room again: at the moment its key named, or [`FULL_RETRY`]
/// after `now` while its domain is full. A handler's navigation to another
/// domain key and a tier race's navigation wait for room alike.
///
/// # Errors
///
/// When to ask again, for a `room` that holds no copy.
pub fn room_taken(room: Room, now: Instant) -> Result<SecondCopy, Instant> {
    match room {
        Room::Taken(copy) => Ok(copy),
        Room::Until(moment) => Err(moment),
        Room::Full => Err(now + FULL_RETRY),
    }
}

/// What an attempt hands over with its Run's pages while its handler runs:
/// what every attempt of its Run hands over alike, built once as the Run's
/// crawl starts and shared (ADR 0138), and what is its own. The crawl
/// builds it once per attempt, before the attempt starts, and each of its
/// runs on a tier takes it whole.
#[derive(Clone)]
pub struct HandOver {
    /// What every attempt of its Run hands over alike.
    pub of_run: Arc<RunHandOver>,
    /// The domain key of its crawl request, whose Permit its admission
    /// holds.
    pub domain_key: String,
    /// Room for requests beside those the Run admitted.
    pub second_copies: SecondCopies,
    /// The attempt's lease.
    pub lease: Arc<dyn LeaseCheck>,
    /// The HTTP admission the Resource governor gave the attempt, given
    /// back once its crawl request's own document is fetched.
    pub fetch_admission: FetchAdmission,
    /// The Page check its crawl request's Label declares of its own, which
    /// judges the browser pages the attempt opens in place of their
    /// Session's (ADR 0093).
    pub page_check: Option<PageCheck>,
    /// The main-content step its crawl request's Label chooses with
    /// `content.extractor`, which every page the attempt serves or opens
    /// reads its formats with over the Run's; none when the Label leaves
    /// it to the Run.
    pub extractor: Option<goodfetch_extractor::BuiltIn>,
    /// The encoding `fetch.encoding` forces on the crawl request's own
    /// document, its Label's else the Run's (ADR 0119); none of the
    /// handler's own navigations takes it.
    pub encoding: Option<&'static encoding_rs::Encoding>,
    /// The types `crawl.content_types` lets the crawl request's own
    /// document be, its Label's else the Run's (ADR 0119); none of the
    /// handler's own navigations is judged by them.
    pub content_types: ContentTypes,
    /// The readers of the untouched page its crawl request's Label reads
    /// each document it is served with, built once per Run (ADR 0138).
    pub untouched: Arc<crate::untouched::UntouchedReaders>,
    /// What a sample's browser run captures of the JSON calls its page
    /// makes, for the `discovered_api` tier's plan (ADR 0020); none on
    /// any other run ([`HandOver::capturing`]).
    pub capture: Option<Capture>,
}

/// What every attempt of a Run hands over alike with its pages: the Run,
/// its settings as its navigation gate reads them, the Politeness it
/// shares and the files it fetched once. Built once as the Run's crawl
/// starts, never per attempt (ADR 0138), and shared by each attempt's
/// [`HandOver`].
pub struct RunHandOver {
    /// The Run.
    pub run: Run,
    /// How the Run keys its requests' domains.
    pub keys: DomainKeys,
    /// The Politeness the Run shares, once it opened; none while the Run's
    /// Politeness paces nothing, which consults no store (ADR 0138).
    pub politeness: Option<Politeness>,
    /// The Politeness the Run shares, once it opened, whatever the Run's
    /// own pacing: a robots.txt's Crawl-delay is recorded there for every
    /// run sharing it (ADR 0086).
    pub shared_politeness: Option<Politeness>,
    /// How robots.txt and the AI-use signal files are fetched when the
    /// copy the Run reads has none.
    pub robots: Option<RobotsTxtFetch>,
    /// `politeness.domain.delay`.
    pub domain_delay: Duration,
    /// `politeness.domain.adaptive_delay`: on, each hop's latency is
    /// recorded for its domain key, for every sharing run.
    pub adaptive_delay: bool,
    /// The Run's delay between starts and its rate: the crawl request's
    /// own send waits for its delay after its domain's last send and for
    /// a token of its domain's sends, every sharing run's together.
    pub pace: goodfetch_frontier::Pace,
    /// What the site's preferences say of each request, the crawl
    /// request's own document, its redirect hops and its handler's pages
    /// alike, and the AI-use signal files the Run fetched for them.
    pub site_preferences: SitePreferences,
    /// The Run's domain policy, which judges each redirect hop.
    pub egress: Arc<Policy>,
    /// The address check of the Run's HTTP tier, which judges each
    /// redirect hop's URL and the addresses its name resolves to before
    /// its robots.txt is read; `None` without the tier, the URL alone then
    /// judged.
    pub addresses: Option<AddressCheck>,
}

/// The HTTP admission the Resource governor gave an attempt (ADR 0040),
/// which its crawl request's own document is fetched under and which is
/// given back once that fetch ended, so a page being analysed holds no
/// place among the requests in flight (ADRs 0004 and 0138). Its clones
/// share it: the first [`FetchAdmission::give_back`] gives it back, and
/// the attempt's ending gives back one its fetch never reached.
#[derive(Clone, Debug, Default)]
pub struct FetchAdmission(Arc<AdmissionCell>);

#[expect(
    clippy::disallowed_types,
    reason = "the admission is taken out in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]
type AdmissionCell = std::sync::Mutex<Option<HttpAdmission>>;

impl FetchAdmission {
    /// One holding `admission` until it is given back.
    pub fn holding(admission: HttpAdmission) -> Self {
        Self(Arc::new(AdmissionCell::new(Some(admission))))
    }

    /// Gives the admission back, if no clone did yet.
    pub fn give_back(&self) {
        let admission = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(admission);
    }
}

impl HandOver {
    /// The attempt's Run.
    pub(crate) fn run(&self) -> &Run {
        &self.of_run.run
    }

    /// This hand-over for a sample's browser run, whose page's JSON calls
    /// are captured into `capture` (ADR 0020).
    #[must_use]
    pub fn capturing(&self, capture: Capture) -> Self {
        Self {
            capture: Some(capture),
            ..self.clone()
        }
    }

    /// Records `hop`'s send as the last start of its domain key for every
    /// run sharing it (ADR 0086), when the Run's Politeness opened. The
    /// `lead` send, its crawl request's own, first waits for its delay
    /// after the key's last send and, in a rated run, for a token of the
    /// key's sends, renewing the lease meanwhile.
    ///
    /// # Errors
    ///
    /// [`Stop::Failed`] with `response_unavailable` when the send could not
    /// be recorded: the hop is then not sent (ADR 0086).
    pub(crate) async fn sending(&self, hop: &Hop, lead: bool) -> Result<(), Stop> {
        self.sent(hop.url.as_str(), lead)
            .await
            .map_err(Stop::Failed)
    }

    /// Records the send of a request to `url` as the last start of its
    /// domain key for every run sharing it (ADR 0086), at the moment the
    /// Politeness file records it, when the Run's Politeness opened: the
    /// later of it and the start already recorded is kept. A `lead` send
    /// first waits for its delay after the key's last send and, in a rated
    /// run, for a token of the key's sends, renewing the lease meanwhile.
    ///
    /// # Errors
    ///
    /// `response_unavailable`, with no `status`, when the Politeness file's
    /// transaction failed, whatever its error: the request's caller then
    /// sends nothing, so no request reaches a site with its start unrecorded
    /// and its delay unchecked (#2321).
    async fn sent(&self, url: &str, lead: bool) -> Result<(), CallError> {
        let of_run = &self.of_run;
        let Some(politeness) = &of_run.politeness else {
            return Ok(());
        };
        let run = &of_run.run;
        let key = of_run.keys.key(url);
        let pace = lead.then_some(of_run.pace);
        loop {
            match goodfetch_frontier::started_at_send(politeness, &key, run.clock(), pace).await {
                Ok(Some(until)) => {
                    let _waiting = self.lease.permit_wait(url);
                    if let Ok(wait) = until.duration_since(run.clock().wall()) {
                        run.clock().sleep(wait).await;
                    }
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    tracing::warn!(%error, run_id = %run.id(), "a request's start at its send was not recorded: it is not sent");
                    return Err(unrecorded(url, &error));
                }
            }
        }
    }
}

/// `response_unavailable` for the request to `url`, held unsent because its
/// start could not be recorded for its domain's Politeness: `error`.
fn unrecorded(url: &str, error: &PolitenessError) -> CallError {
    CallError::new(
        &RESPONSE_UNAVAILABLE,
        format!(
            "The request to {url} was not sent: its start could not be recorded for its domain's Politeness ({error}), so its delay could not be kept."
        ),
        &json!({ "url": url, "status": null, "fix": UNRECORDED_FIX }),
    )
}

/// What `response_unavailable` advises for a request held unsent.
const UNRECORDED_FIX: &str = "Free space on the storage the Politeness file is on, or repair it; `run.retry_failed` then admits the request again.";

impl RunHandOver {
    /// Judges `url` by the Run's egress policy as its HTTP tier's
    /// connection to it would be judged: the URL, then the addresses its
    /// name resolves to (ADR 0017), the URL alone without the tier.
    ///
    /// # Errors
    ///
    /// The policy's refusal, whose `egress_refused` names the URL.
    pub(crate) async fn address_check(&self, url: &Url) -> Result<(), Refused> {
        match &self.addresses {
            Some(addresses) => addresses.check(url).await,
            None => judged(url, &self.egress),
        }
    }

    /// Fetches `location`, a robots.txt or an AI-use signal file, with
    /// [`RunHandOver::robots`]; unreachable without it.
    async fn site_file(&self, location: Url) -> robots_txt::Answer {
        match &self.robots {
            Some(fetch) => fetch(self.run.clone(), location).await,
            None => robots_txt::Answer::Unreachable(
                "the crawl has no transport to fetch robots.txt with".to_owned(),
            ),
        }
    }

    /// Records `hop`'s `latency` for its domain key, for every run sharing
    /// it, while the Run's adaptive delay is on.
    pub(crate) async fn answered(&self, hop: &Hop, latency: Duration) {
        let Some(politeness) = self.politeness.as_ref().filter(|_| self.adaptive_delay) else {
            return;
        };
        let key = self.keys.key(hop.url.as_str());
        if let Err(error) = goodfetch_frontier::latency_measured(politeness, &key, latency).await {
            tracing::warn!(%error, run_id = %self.run.id(), "a response's latency was not recorded for its domain");
        }
    }
}

/// The navigation gate of the pages one attempt handed over.
pub(crate) struct Gate(Arc<HandOver>);

impl Gate {
    pub(crate) fn new(hand_over: Arc<HandOver>) -> Self {
        Self(hand_over)
    }
}

impl fmt::Debug for Gate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Gate")
            .field("run", self.0.run().id())
            .field("domain_key", &self.0.domain_key)
            .finish_non_exhaustive()
    }
}

/// The navigation gate of the pages browser-fetch sends a crawl request's
/// hops from: it admits each at once, since the redirect chain already
/// took each hop through [`ChainGate`] (ADR 0118).
#[derive(Debug)]
pub(crate) struct Admitted;

impl NavigationGate for Admitted {
    fn admit<'a>(&'a self, _navigation: &'a Navigation) -> Admission<'a> {
        Box::pin(async { Ok(Permit::unlimited()) })
    }

    /// Hears nothing: [`ChainGate::sending`] recorded the hop's send.
    fn sending<'a>(&'a self, _url: &'a str) -> Sending<'a> {
        Box::pin(async {})
    }

    fn judge_document<'a>(&'a self, _response: &'a DocumentResponse<'a>) -> DocumentJudgment<'a> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn reads_document_body(&self) -> bool {
        false
    }
}

/// The lease check of the pages one attempt handed over: the attempt's.
pub(crate) struct Lease(Arc<HandOver>);

impl Lease {
    pub(crate) fn new(hand_over: Arc<HandOver>) -> Self {
        Self(hand_over)
    }
}

impl fmt::Debug for Lease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.lease.fmt(formatter)
    }
}

impl LeaseCheck for Lease {
    fn check(&self) -> Result<(), LeaseEnded> {
        self.0.lease.check()
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        self.0.lease.withheld(ended, method)
    }

    fn permit_wait(&self, url: &str) -> PermitWait {
        self.0.lease.permit_wait(url)
    }
}

impl NavigationGate for Gate {
    fn admit<'a>(&'a self, navigation: &'a Navigation) -> Admission<'a> {
        Box::pin(async move {
            match Url::parse(&navigation.url) {
                Ok(url) if matches!(url.scheme(), "http" | "https") => {
                    self.admitted(&url).await.map_err(refusal)
                }
                _ => Ok(Permit::unlimited()),
            }
        })
    }

    /// Records the request's send as the last start of its domain key for
    /// every run sharing it, when later than the start its admission took:
    /// the browser sends it once the page sent it on, and a Session's first
    /// request through its egress relay only after the relay's `407`, so
    /// the next request would otherwise reach the site that much closer
    /// than the delay (ADR 0086).
    ///
    /// The browser sent the request before the page heard of it, so a send
    /// that cannot be recorded is only warned of here: nothing is left to
    /// hold back.
    fn sending<'a>(&'a self, url: &'a str) -> Sending<'a> {
        Box::pin(async move {
            if Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
                // Warned of where it failed.
                let _ = self.0.sent(url, false).await;
            }
        })
    }

    /// Refuses with `use_signals` a document whose response's AI-use
    /// signals refuse the Run's Purpose, while the Run honours them
    /// (ADRs 0016, 0045, 0086 and 0093): its headers, its markup, and the
    /// RSL license either names, fetched once for the Run.
    fn judge_document<'a>(&'a self, response: &'a DocumentResponse<'a>) -> DocumentJudgment<'a> {
        Box::pin(async move {
            let of_run = &self.0.of_run;
            if !of_run.site_preferences.honours_use_signals() {
                return Ok(());
            }
            let signals = UseSignals::of(response.headers);
            let signals = match Url::parse(response.url) {
                Ok(url) => {
                    // The markup's signals are ASCII: a document in another
                    // encoding reads the same for them.
                    let html = String::from_utf8_lossy(response.body).into_owned();
                    let readers = Arc::clone(&self.0.untouched);
                    let work = self.0.run().metrics().counted_work().clone();
                    self.0
                        .run()
                        .owner()
                        .analysis(move || readers.markup_use_signals(signals, &html, &url, &work))
                        .await
                }
                Err(_) => signals,
            };
            of_run
                .site_preferences
                .document_judged(response.url, &signals, |location| {
                    of_run.site_file(location)
                })
                .await
                .map_err(refusal)
        })
    }

    /// Only while the Run honours AI-use signals, whose `<meta>` it reads.
    fn reads_document_body(&self) -> bool {
        self.0.of_run.site_preferences.honours_use_signals()
    }
}

/// A handler page's refusal by a site preference, an unreachable robots.txt
/// refusing it as robots.txt (RFC 9309 §2.3.1.4), or by a limit a file of the
/// preferences passed.
pub(crate) fn refusal(refused: impl Into<site_preferences::NotAllowed>) -> Refusal {
    match refused.into() {
        site_preferences::NotAllowed::Refused(refused) => Refusal::Preference {
            reason: refused.by.site_preference(),
            message: refused.message,
        },
        site_preferences::NotAllowed::PastALimit(limit) => Refusal::PastALimit(limit),
    }
}

impl Gate {
    /// Admits a request to `url`, or refuses it by the site's preferences
    /// ([`SitePreferences::judged`]).
    pub(crate) async fn admitted(&self, url: &Url) -> Result<Permit, site_preferences::NotAllowed> {
        let hand_over = &self.0.of_run;
        let run = &hand_over.run;
        let preferences = &hand_over.site_preferences;
        let key = hand_over.keys.key(url.as_str());
        let own = key == self.0.domain_key;
        let before = self.last_start(&key).await;
        let permit = if own {
            Permit::unlimited()
        } else {
            self.beside(&key).await
        };
        let reading = if preferences.reads_robots_txt(Request::Navigation) {
            self.robots_txt(url, &key).await
        } else {
            None
        };
        let allowed = preferences
            .judged(Request::Navigation, url, reading.as_ref(), |location| {
                hand_over.site_file(location)
            })
            .await?;
        let crawl_delay = allowed.crawl_delay.unwrap_or_default();
        let start = self
            .start(&key, own, before, reading.as_ref(), crawl_delay)
            .await;
        if let Some(wait) = start.and_then(|start| start.duration_since(run.clock().wall()).ok()) {
            run.clock().sleep(wait).await;
        }
        // Stamped before the request leaves, so their ways to the site take
        // no part in a gap here. A gap can still fall short of the delay by
        // how late the earlier request woke, since each start is taken before
        // its wait: under 1 ms measured, at 1999.16 ms for a 2 s delay.
        tracing::trace!(target: "goodfetch::navigation_gate", domain_key = %key, "a handler's request was admitted");
        Ok(permit)
    }

    /// When a request of `key` admitted now starts, taken for every run
    /// sharing the domain at once, so that two handlers' requests never
    /// read the same last start and go together (ADR 0086): after the
    /// robots.txt it `read`, while the fetch of it is less than the delay
    /// before, the delay between starts and the Crawl-delay after that
    /// fetch's response arrived and after the last start
    /// ([`Reading::start_after`]); else, on the attempt's own key, the
    /// same after the last start; on another key, whose Permit its
    /// domain's rule already spaced, the Crawl-delay after the start
    /// `before` it.
    async fn start(
        &self,
        key: &str,
        own: bool,
        before: Option<SystemTime>,
        read: Option<&Reading>,
        crawl_delay: Duration,
    ) -> Option<SystemTime> {
        let hand_over = &self.0.of_run;
        let run = &hand_over.run;
        let spacing = hand_over.domain_delay.max(crawl_delay);
        if let Some(read) = read
            && let AfterRobotsTxt::At(start) = read
                .start_after(hand_over.politeness.as_ref(), run, key, spacing)
                .await
        {
            return Some(start);
        }
        // No run shares the domain, and no last start is known: the request
        // goes once its robots.txt's spacing passed.
        let politeness = hand_over.politeness.as_ref()?;
        let wall = run.clock().wall();
        // The start its spacing runs from, when it takes its start at once.
        let after = own.then(|| wall.checked_sub(spacing).unwrap_or(wall));
        if let Some(after) = after {
            return goodfetch_frontier::start_after(politeness, key, after, spacing)
                .await
                .inspect_err(|error| {
                    tracing::warn!(%error, run_id = %run.id(), "a handler's navigation start was not taken");
                })
                .ok();
        }
        let start = before
            .map(|before| before + crawl_delay)
            .filter(|start| *start > wall);
        if let Err(error) =
            goodfetch_frontier::started_again(politeness, key, start.unwrap_or(wall)).await
        {
            tracing::warn!(%error, run_id = %run.id(), "a handler's navigation start was not recorded");
        }
        start
    }

    /// When the last request of `key` started, by any sharing run.
    async fn last_start(&self, key: &str) -> Option<SystemTime> {
        let politeness = self.0.of_run.politeness.as_ref()?;
        goodfetch_frontier::last_start(politeness, key)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, run_id = %self.0.run().id(), "a domain's last start could not be read");
                None
            })
    }

    /// A Permit of `key` beside the Run's own requests, once the domain
    /// has room, given back once released.
    async fn beside(&self, key: &str) -> Permit {
        let run = self.0.run();
        let clock = run.clock();
        loop {
            let room = self.0.second_copies.take(key, clock.now()).await;
            match room_taken(room, clock.now()) {
                Ok(copy) => {
                    let (run, second_copies) = (run.clone(), self.0.second_copies.clone());
                    return Permit::new(move || {
                        run.spawn(async move { second_copies.give_back(copy).await });
                    });
                }
                Err(until) => clock.sleep_until(until).await,
            }
        }
    }

    /// `url`'s robots.txt, read from the copy the Run shares
    /// ([`SitePreferences::read_robots_txt`]), its fetch, when this request
    /// made one, recorded as a start of `key` for every run sharing the
    /// domain from its response's arrival.
    async fn robots_txt(&self, url: &Url, key: &str) -> Option<Reading> {
        let hand_over = &self.0.of_run;
        let run = &hand_over.run;
        let reading = hand_over
            .site_preferences
            .read_robots_txt(run, url, |location| hand_over.site_file(location))
            .await?;
        if let (Some(at), Some(politeness)) = (reading.fetched(), &hand_over.politeness)
            && let Err(error) = goodfetch_frontier::started_again(politeness, key, at).await
        {
            tracing::warn!(%error, run_id = %run.id(), "a robots.txt fetch's start was not recorded");
        }
        Some(reading)
    }
}

/// The gate of a crawl request's redirect chain on a document tier: the
/// attempt's navigation gate, after the Frontier's checks of each hop.
pub(crate) struct ChainGate {
    pub(crate) hand_over: Arc<HandOver>,
    /// The crawl request's ID.
    pub(crate) crawl_request: String,
}

impl ChainGate {
    /// What the Frontier says of `hop`: whether it leaves the scope, goes
    /// to a host the domain policy refuses or reaches a key admitted for
    /// another request. A run file that cannot answer leaves the hop to the
    /// navigation gate, and the transport's own judgment of it.
    async fn frontier(&self, hop: &Hop) -> goodfetch_frontier::Hop {
        let run = self.hand_over.run();
        let (id, url, method, body) = frontier_hop(&self.crawl_request, hop);
        let egress = Arc::clone(&self.hand_over.of_run.egress);
        let work = run.metrics().counted_work().clone();
        work.count(Row::HopTransactions);
        // A hop's check-and-set answers the chain, not the outside: the
        // attempt's outcome syncs it (ADR 0138's note of #1926).
        run.state()
            .write_deferred(move |connection| {
                goodfetch_frontier::admit_hop(
                    connection,
                    &work,
                    &egress,
                    &id,
                    (&url, &method, body.as_deref()),
                )
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, run_id = %run.id(), "a redirect hop was not checked by the Frontier");
                goodfetch_frontier::Hop::Follow
            })
    }
}

/// What the Frontier takes of `hop` of `crawl_request`.
fn frontier_hop(crawl_request: &str, hop: &Hop) -> (String, Url, HttpMethod, Option<Bytes>) {
    (
        crawl_request.to_owned(),
        hop.url.clone(),
        HttpMethod::parse(hop.method.as_str()).unwrap_or_else(|_| HttpMethod::get()),
        hop.body.clone(),
    )
}

impl HopGate for ChainGate {
    fn admit<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>> {
        Box::pin(async move {
            let skipped = |reason, message: String| Err(Stop::Skipped(Skip { reason, message }));
            match self.frontier(hop).await {
                goodfetch_frontier::Hop::OutOfScope => {
                    return skipped(
                        SkipReason::OutOfScope,
                        format!("A redirect to `{}` leaves the crawl's scope.", hop.url),
                    );
                }
                goodfetch_frontier::Hop::DomainPolicy => {
                    return skipped(
                        SkipReason::DomainPolicy,
                        format!(
                            "A redirect to `{}` goes to a host the Run's domain policy refuses.",
                            hop.url
                        ),
                    );
                }
                goodfetch_frontier::Hop::Duplicate => {
                    return skipped(
                        SkipReason::Duplicate,
                        format!(
                            "A redirect to `{}` reaches a page the crawl admitted for another request.",
                            hop.url
                        ),
                    );
                }
                goodfetch_frontier::Hop::Follow => {}
            }
            if let Err(refused) = self.hand_over.of_run.address_check(&hop.url).await {
                return skipped(
                    SkipReason::DomainPolicy,
                    format!(
                        "A redirect to `{}` is refused by the Run's egress policy: {refused}.",
                        hop.url
                    ),
                );
            }
            let _waiting = self.hand_over.lease.permit_wait(hop.url.as_str());
            match Gate::new(Arc::clone(&self.hand_over))
                .admitted(&hop.url)
                .await
            {
                Ok(permit) => Ok(permit),
                Err(site_preferences::NotAllowed::PastALimit(limit)) => {
                    Err(Stop::Failed(limit.error()))
                }
                // A crawl request retries a hop's unreachable robots.txt as
                // it retries its own (ADR 0086).
                Err(site_preferences::NotAllowed::Refused(refusal))
                    if refusal.by == RefusedBy::UnreachableRobotsTxt =>
                {
                    Err(Stop::RobotsTxtUnreachable {
                        message: refusal.message,
                        on_own_domain: self.hand_over.of_run.keys.key(hop.url.as_str())
                            == self.hand_over.domain_key,
                    })
                }
                Err(site_preferences::NotAllowed::Refused(refusal)) => {
                    skipped(refusal.by.skip_reason(), refusal.message)
                }
            }
        })
    }

    fn wait_until<'a>(&'a self, hop: &'a Hop, until: SystemTime) -> Gating<'a, ()> {
        Box::pin(async move {
            let _waiting = self.hand_over.lease.permit_wait(hop.url.as_str());
            let clock = self.hand_over.run().clock();
            if let Ok(wait) = until.duration_since(clock.wall()) {
                clock.sleep(wait).await;
            }
        })
    }

    /// Records the hop's send as the last start of its domain key for
    /// every run sharing it, when later than the start its admission took:
    /// what ran between them (robots.txt, the Frontier's checks, the
    /// hand-over) would otherwise bring the next request that much closer
    /// at the site than the delay (ADR 0086). It also moves the token the
    /// admission took from the domain's rate to the send, when later; the
    /// lead hop of a rated run first waits for a token of its domain's
    /// sends.
    fn sending<'a>(&'a self, hop: &'a Hop, lead: bool) -> Gating<'a, Result<(), Stop>> {
        Box::pin(self.hand_over.sending(hop, lead))
    }

    /// Records the hop's latency for its domain key, for every run sharing
    /// it, while the Run's adaptive delay is on (spec #582's Settings).
    fn answered<'a>(&'a self, hop: &'a Hop, latency: Duration) -> Gating<'a, ()> {
        Box::pin(self.hand_over.of_run.answered(hop, latency))
    }

    /// Records the chain's final URL under its key, unless another request
    /// was admitted for that key, or reached it, since the hop was checked:
    /// the request then ends `skipped` with `duplicate` (ADR 0120). A run
    /// file that cannot answer lets the chain end on its document.
    fn reached<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<(), Stop>> {
        Box::pin(async move {
            let run = self.hand_over.run();
            let (id, url, method, body) = frontier_hop(&self.crawl_request, hop);
            let recorded = run
                .state()
                .write_deferred(move |connection| {
                    goodfetch_frontier::redirect_reached(
                        connection,
                        &id,
                        &url,
                        &method,
                        body.as_deref(),
                    )
                })
                .await;
            match recorded {
                Ok(goodfetch_frontier::Hop::Duplicate) => Err(Stop::Skipped(Skip {
                    reason: SkipReason::Duplicate,
                    message: format!(
                        "A redirect to `{}` reaches a page the crawl reached for another request meanwhile.",
                        hop.url
                    ),
                })),
                Ok(_) => Ok(()),
                Err(error) => {
                    tracing::warn!(%error, run_id = %run.id(), "a redirect's final URL was not recorded");
                    Ok(())
                }
            }
        })
    }
}
