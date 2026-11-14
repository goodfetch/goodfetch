//! A handler's `enqueueLinks` (ADRs 0093 and 0120): `crawl_request.links`
//! reads each element a locator matches on the page a fetch tier served
//! the attempt, as the program's Payload function reads it, and
//! `crawl_request.enqueue_links` admits each element's link as an `href`
//! candidate, resolved against the document's base URL with its query in
//! the document's encoding, under the call's own `include` and `exclude`,
//! and answers each element: added, not added with its reason, `no_link`
//! or `skipped_by_payload`. Once the call's `limit` links are added, each
//! later link the Frontier would judge is not added, with `link_limit`. Every Payload is checked before any link is
//! admitted, so a `payload_invalid` adds none; the links are staged with
//! the attempt together, and commit or are discarded with it, as one
//! enqueue is (ADR 0093). On the browser tier, a link a person cannot see
//! is not added, with `trap_link` (ADR 0035; the Frontier's `trap_links`);
//! a served document has no layout, so a link its markup hides is added,
//! and marked as a suspected trap in its work item and, once its attempt
//! commits, in Provenance, `trap_link_suspected`: a browser tier never
//! follows it. A link hidden in a menu, a tab or a disclosure a person
//! opens, and not `nofollow`, is added on either tier, marked alike as
//! `revealable`, which every tier follows (owner decision O26). A link that
//! is not suspected and reaches a marked request again clears its mark; a
//! revealable one leaves it marked as revealable.
//!
//! A call's work has two parts (#2244; ADR 0142). On the task that serves
//! it, the page's own for a code-free page, with no run file: each link is
//! read, judged as a trap, given its Payload, which is checked, and keyed
//! by the Frontier under the Run's invariants and its domain policy. In
//! the crawl's Run state, which its delivery loop owns and the call asks
//! (`run_state::admission`): its lease, the call's `limit`, and the
//! Frontier's part that reads the visited set and the Bounds, and adds. A
//! program's call is answered so in a write of its own, counted as
//! `F3.transactions`, and its attempt's commit applies what that write
//! decided, running neither part again (#2245). A code-free page's links
//! are answered to nobody: they ride its attempt's outcome, whose one job
//! runs the second part the first time.

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_fetch_tier::{FetchTiers, LinksUnread, ServedLinks};
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::trap_links::{self, Mark, Sight, Trap, Verdict};
use goodfetch_frontier::{Narrowing, Patterns, Url, document_base, key_link, resolve_link};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE_LINKS, CRAWL_REQUEST_LINKS, CrawlRequestEnqueueLinksParams,
    CrawlRequestLinks, CrawlRequestLinksParams, EnqueuedLinks, MatchedLink,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::Locator;
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails, RunId};
use serde_json::Value;

use crate::invariants::Invariants;
use crate::run_state::admission::{Link, LinksBatch, LinksRequest, Suspected};
use crate::run_state::{Asking, StateError};
use crate::serving::{Crawls, LiveCrawl};
use crate::staging::{Marked, Staging};

/// The handlers of `crawl_request.links` and `crawl_request.enqueue_links`
/// on the live crawls of `crawls`, whose pages `tiers` serve.
pub(crate) fn handlers(crawls: &Crawls, tiers: &FetchTiers) -> [Handler; 2] {
    let (reading, enqueuing) = (crawls.clone(), crawls.clone());
    let (read_on, enqueue_on) = (tiers.clone(), tiers.clone());
    [
        Handler::new(&CRAWL_REQUEST_LINKS, move |params, context| {
            links(params, context, reading.clone(), read_on.clone())
        }),
        Handler::new(&CRAWL_REQUEST_ENQUEUE_LINKS, move |params, context| {
            enqueue_links(params, context, enqueuing.clone(), enqueue_on.clone())
        }),
    ]
}

/// The attempt a call names.
struct Attempt<'a> {
    run_id: &'a RunId,
    crawl_request_id: &'a str,
    lease_epoch: u64,
}

impl Attempt<'_> {
    fn ended(&self) -> CallError {
        CallError::new(
            &ATTEMPT_ENDED,
            format!(
                "Attempt {} of crawl request `{}` of the crawl `{}` already ended, so no link was read or enqueued.",
                self.lease_epoch, self.crawl_request_id, self.run_id
            ),
            &(),
        )
    }

    fn unavailable(&self, error: impl std::fmt::Display) -> CallError {
        CallError::new(
            &RUN_STATE_UNAVAILABLE,
            format!(
                "The state of the Run `{}` could not take the links: {error}",
                self.run_id
            ),
            &RunDetails {
                run_id: self.run_id.clone(),
                path: None,
            },
        )
    }
}

/// The elements `locator` matches on the page a tier served, with the
/// base URL their links resolve against.
async fn served(
    tiers: &FetchTiers,
    locator: &Locator,
    work: &CountedWork,
) -> Result<(ServedLinks, Url), CallError> {
    let served = tiers
        .links(&locator.page_id, &locator.selector, work)
        .await
        .ok_or_else(|| {
            CallError::new(
                &CLOSED,
                format!(
                    "No fetch tier serves the page `{}` now, so its links cannot be read: read the page its attempt was given while the attempt runs.",
                    locator.page_id.0
                ),
                &(),
            )
        })?
        .map_err(|unread| match unread {
            LinksUnread::Selector(invalid) => {
                CallError::protocol(ProtocolFailure::InvalidParams, invalid.to_string())
            }
            LinksUnread::Page(error) => error,
        })?;
    let base = document_base(&served.base_url, None).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "The page's base URL `{}` is no absolute URL.",
                served.base_url
            ),
        )
    })?;
    Ok((served, base))
}

async fn links(
    params: CrawlRequestLinksParams,
    _context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
) -> Result<CrawlRequestLinks, CallError> {
    let attempt = Attempt {
        run_id: &params.run_id,
        crawl_request_id: &params.crawl_request_id,
        lease_epoch: params.lease_epoch,
    };
    let Some(LiveCrawl { run, staging, .. }) = crawls.live(&params.run_id).await else {
        return Err(attempt.ended());
    };
    // The Lease is held in memory: the run file may not hold its claim yet,
    // in a batch its readers do not see (ADR 0142).
    if !staging.holds(&params.crawl_request_id, params.lease_epoch) {
        return Err(attempt.ended());
    }
    let (served, base) = served(&tiers, &params.locator, run.metrics().counted_work()).await?;
    Ok(CrawlRequestLinks {
        links: served
            .elements
            .into_iter()
            .map(|element| MatchedLink {
                url: element
                    .href
                    .as_deref()
                    .and_then(|href| resolve_link(href, &base, served.encoding))
                    .map(String::from),
                href: element.href,
                text: element.text,
                attributes: element.attributes.into_iter().collect(),
            })
            .collect(),
    })
}

pub(crate) async fn enqueue_links(
    params: CrawlRequestEnqueueLinksParams,
    _context: CallContext,
    crawls: Crawls,
    tiers: FetchTiers,
) -> Result<EnqueuedLinks, CallError> {
    let attempt = Attempt {
        run_id: &params.run_id,
        crawl_request_id: &params.crawl_request_id,
        lease_epoch: params.lease_epoch,
    };
    let (asking, _, batch) = prepared(&params, &crawls, &tiers).await?;
    match asking.links(batch).await {
        Ok(Some(answered)) => answered,
        // The crawl ended with its attempts.
        Ok(None) | Err(StateError::Closed) => Err(attempt.ended()),
        Err(error) => Err(attempt.unavailable(error)),
    }
}

/// Stages the links `params` names with its attempt as
/// [`enqueue_links`] does, unanswered and unrun: its attempt's commit
/// admits them, the first time, in its outcome's one job (ADR 0138), so a
/// code-free attempt, which reads no answer, writes nothing for them
/// before. A link added through a link suspected a trap is marked in
/// Provenance once that commit kept it.
///
/// # Errors
///
/// As [`enqueue_links`]'s before its write, `attempt_ended` once the
/// attempt ended, and [`Staging::defer`]'s refusals.
pub(crate) async fn defer_links(
    params: &CrawlRequestEnqueueLinksParams,
    crawls: &Crawls,
    tiers: &FetchTiers,
) -> Result<(), CallError> {
    let attempt = Attempt {
        run_id: &params.run_id,
        crawl_request_id: &params.crawl_request_id,
        lease_epoch: params.lease_epoch,
    };
    let (_, staging, batch) = prepared(params, crawls, tiers).await?;
    match batch.defer(&staging) {
        Some(deferred) => deferred,
        None => Err(attempt.ended()),
    }
}

/// How the live crawl of `params`'s Run is asked, its staging, and the
/// batch of links its locator matches on the served page, owned for the
/// job that admits it, its Payloads fitted and checked, its patterns
/// compiled and each link keyed, on the task that serves the call, with no
/// run file (#2244).
async fn prepared(
    params: &CrawlRequestEnqueueLinksParams,
    crawls: &Crawls,
    tiers: &FetchTiers,
) -> Result<(Asking, Staging, LinksBatch), CallError> {
    let attempt = Attempt {
        run_id: &params.run_id,
        crawl_request_id: &params.crawl_request_id,
        lease_epoch: params.lease_epoch,
    };
    if params.payload.is_some() && params.payloads.is_some() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "An `enqueueLinks` call takes `payload`, one Payload for every link, or `payloads`, one per link, never both.".to_owned(),
        ));
    }
    let Some(LiveCrawl {
        run,
        egress,
        invariants,
        asking,
        staging,
        ..
    }) = crawls.live(&params.run_id).await
    else {
        return Err(attempt.ended());
    };
    let work = run.metrics().counted_work().clone();
    // Each glob compiles once for the engine, from its cache (#1896).
    let include = Patterns::cached(&work, Row::BatchCompilations, &params.include)
        .map_err(CallError::from)?;
    let exclude = Patterns::cached(&work, Row::BatchCompilations, &params.exclude)
        .map_err(CallError::from)?;
    let (served, base) = served(tiers, &params.locator, &work).await?;
    payloads_fit(params, served.elements.len())?;
    // Each Payload is serialized once, the call's one for every link
    // shared by them all (#1896).
    let every = params
        .payload
        .as_ref()
        .map(|payload| Arc::new(Payload::of(payload.clone(), &work)));
    let reading = Reading {
        invariants: &invariants,
        egress: &egress,
        label: &params.label,
        keeps_fragment: invariants.keeps_fragment(&params.label),
        base: &base,
        encoding: served.encoding,
        narrowing: Narrowing {
            include: &include,
            exclude: &exclude,
        },
        work: &work,
    };
    let mut refused_payload = None;
    let links: Vec<Link> = served
        .elements
        .into_iter()
        .enumerate()
        .map(|(at, element)| {
            let payload = match &params.payloads {
                Some(payloads) => match &payloads[at] {
                    Value::Null => Chosen::Skipped,
                    payload => Chosen::Payload(Some(Arc::new(Payload::of(payload.clone(), &work)))),
                },
                None => Chosen::Payload(every.clone()),
            };
            reading.link(element.href, element.sight, payload, &mut refused_payload)
        })
        .collect();
    let batch = LinksBatch {
        request: LinksRequest::of(params),
        invariants,
        links,
        refused_payload,
    };
    Ok((asking, staging, batch))
}

/// What the task that serves a links call reads each of its links with:
/// the Run's invariants and its domain policy, the call's Label and
/// patterns, and the document's base URL and encoding.
struct Reading<'a> {
    invariants: &'a Invariants,
    egress: &'a Policy,
    label: &'a str,
    /// Whether a request of the call's Label keeps its URL's fragment.
    keeps_fragment: bool,
    base: &'a Url,
    encoding: &'static goodfetch_frontier::Encoding,
    narrowing: Narrowing<'a>,
    work: &'a CountedWork,
}

impl Reading<'_> {
    /// Reads one matched element, its `href`, how a person sees it and the
    /// Payload `chosen` for it: the part of its admission that takes no run
    /// file (#2244). The refusal of the first Payload that does not fit the
    /// Label's schema or the cap is kept in `refused`, each link that has
    /// one being checked.
    fn link(
        &self,
        href: Option<String>,
        sight: Sight,
        chosen: Chosen,
        refused: &mut Option<CallError>,
    ) -> Link {
        let Some(href) = href else {
            return Link::NoHref;
        };
        if let Chosen::Payload(payload) = &chosen
            && refused.is_none()
        {
            *refused = self
                .invariants
                .payloads()
                .check(self.label, payload.as_deref(), self.work)
                .err();
        }
        let trap = match Judged::of(sight) {
            Some(Judged::Trap(trap)) => return Link::Trap { href, trap },
            Some(Judged::Suspected(trap)) => Some(Suspected::Hidden(trap)),
            Some(Judged::Revealable) => Some(Suspected::Revealable),
            None => None,
        };
        let Chosen::Payload(payload) = chosen else {
            return Link::Skipped { href };
        };
        let keyed = key_link(
            self.invariants.frontier(),
            self.egress,
            goodfetch_frontier::Link {
                href: &href,
                base: self.base,
                encoding: self.encoding,
                keeps_fragment: self.keeps_fragment,
                narrowing: self.narrowing,
            },
        );
        Link::Keyed {
            href,
            keyed,
            payload,
            trap,
        }
    }
}

/// `invalid_params` when `params`'s `payloads` do not give one Payload for each
/// of the `elements` its locator matches.
fn payloads_fit(params: &CrawlRequestEnqueueLinksParams, elements: usize) -> Result<(), CallError> {
    match &params.payloads {
        Some(payloads) if payloads.len() != elements => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`payloads` holds {} Payloads, but the locator `{}` matches {elements} elements: give one per element, in the order `crawl_request.links` read them.",
                payloads.len(),
                params.locator.selector,
            ),
        )),
        _ => Ok(()),
    }
}

/// What the program chose for one matched element's link.
enum Chosen {
    /// Its Payload, if any, serialized once.
    Payload(Option<Arc<Payload>>),
    /// Its Payload function skipped it.
    Skipped,
}

/// What `trap_links` made of a link a person may not see.
#[derive(Clone, Copy, Debug)]
enum Judged {
    /// Its browser page's layout hides it: it is not added.
    Trap(Trap),
    /// Its served document's markup hides it, which no layout confirms: it
    /// is added, marked as a suspected trap a browser tier never follows.
    Suspected(Trap),
    /// A menu, a tab or a disclosure a person opens hides it, on either
    /// tier: it is added, marked as a suspected trap every tier follows.
    Revealable,
}

impl Judged {
    /// What `trap_links` makes of the link `sight` describes; `None` for
    /// one a person sees.
    fn of(sight: Sight) -> Option<Self> {
        let (seen, hidden): (_, fn(Trap) -> Self) = match sight {
            Sight::Layout(seen) => (seen, Self::Trap),
            Sight::Markup(seen) => (seen, Self::Suspected),
        };
        match trap_links::judge(&seen) {
            Verdict::Seen => None,
            Verdict::Revealable => Some(Self::Revealable),
            Verdict::Trap(trap) => Some(hidden(trap)),
        }
    }
}

/// Marks in the Run's Provenance, `trap_link_suspected`, each request of
/// `suspected`, with its URL, that the attempt of the request `from`
/// committed as added through a link its served document's markup hides,
/// or that a menu, a tab or a disclosure a person opens hides
/// (`revealable`, followed on every tier) (ADR 0035), once that attempt
/// committed: a discarded attempt marks nothing.
pub(crate) async fn mark_suspected(run: &Run, from: &str, suspected: &[Marked]) {
    for Marked { id, url, mark } in suspected {
        let entry = serde_json::json!({
            "action": trap_links::SUSPECTED_ACTION,
            "crawl_request_id": id,
            "url": url,
            "from": from,
            "revealable": *mark == Mark::Revealable,
        });
        if let Err(error) = run.provenance(entry).await {
            tracing::warn!(%error, "a suspected trap link's Provenance entry cannot be written");
        }
    }
}
