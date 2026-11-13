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

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_fetch_tier::{FetchTiers, LinksUnread, ServedLinks};
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::trap_links::{self, Mark, Sight, Trap, Verdict};
use goodfetch_frontier::{Candidate, Narrowing, Patterns, Url, document_base, resolve_link};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_REQUEST_ENQUEUE_LINKS, CRAWL_REQUEST_LINKS, CrawlRequestEnqueueLinksParams,
    CrawlRequestLinks, CrawlRequestLinksParams, Enqueued, EnqueuedLink, EnqueuedLinks, HttpMethod,
    MatchedLink, NotAddedReason,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::Locator;
use goodfetch_protocol::run::{RUN_STATE_UNAVAILABLE, RunDetails, RunId};
use rusqlite::Connection;
use serde_json::Value;

use crate::invariants::Invariants;
use crate::serving::{Crawls, LiveCrawl};
use crate::staging::{Answered, Marked, Staging};
use crate::work_items::{self, Adding, Positions};

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
    let Some(LiveCrawl { run, .. }) = crawls.live(&params.run_id).await else {
        return Err(attempt.ended());
    };
    let (id, epoch) = (params.crawl_request_id.clone(), params.lease_epoch);
    let holds = run
        .state()
        .read(move |connection| work_items::attempt(connection, &id, epoch))
        .await
        .map_err(|error| attempt.unavailable(error))?;
    if holds.is_none() {
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
    let (run, staging, batch) = prepared(&params, &crawls, &tiers).await?;
    // An enqueue's answers promise nothing (ADR 0138's note of #1926): the
    // attempt's outcome syncs them with it.
    let answered = run
        .state()
        .write_deferred(move |connection| {
            let (crawl_request_id, lease_epoch) = (
                batch.request.crawl_request_id.clone(),
                batch.request.lease_epoch,
            );
            let answered = staging.admit(
                connection,
                &crawl_request_id,
                lease_epoch,
                move |connection, positions| batch.add(connection, positions),
            )?;
            note_suspected(
                &staging,
                (&crawl_request_id, lease_epoch),
                answered.as_ref(),
            );
            Ok(answered)
        })
        .await;
    match answered {
        Ok(Some(Ok(admitted))) => Ok(admitted.links),
        Ok(Some(Err(refused))) => Err(refused),
        Ok(None) => Err(attempt.ended()),
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
    let deferred = staging.defer(
        &params.crawl_request_id,
        params.lease_epoch,
        move |connection, positions| {
            Ok(match batch.add(connection, positions)? {
                Some(Ok(admitted)) => admitted.suspected,
                Some(Err(_)) | None => Vec::new(),
            })
        },
    );
    match deferred {
        Some(deferred) => deferred,
        None => Err(attempt.ended()),
    }
}

/// The live crawl of `params`'s Run, its staging, and the batch of links
/// its locator matches on the served page, owned for a write to admit,
/// its Payloads fitted and its patterns compiled; one batch's
/// transaction counted.
async fn prepared(
    params: &CrawlRequestEnqueueLinksParams,
    crawls: &Crawls,
    tiers: &FetchTiers,
) -> Result<(Run, Staging, OwnedBatch), CallError> {
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
    let links: Vec<Link> = served
        .elements
        .into_iter()
        .enumerate()
        .map(|(at, element)| Link {
            trap: Judged::of(element.sight),
            href: element.href,
            payload: match &params.payloads {
                Some(payloads) => match &payloads[at] {
                    Value::Null => Chosen::Skipped,
                    payload => Chosen::Payload(Some(Arc::new(Payload::of(payload.clone(), &work)))),
                },
                None => Chosen::Payload(every.clone()),
            },
        })
        .collect();
    work.count(Row::BatchTransactions);
    let batch = OwnedBatch {
        request: Request::of(params),
        egress,
        invariants,
        links,
        base,
        encoding: served.encoding,
        include,
        exclude,
        work,
    };
    Ok((run, staging, batch))
}

/// What a links call's admission reads of its params.
struct Request {
    crawl_request_id: String,
    lease_epoch: u64,
    label: String,
    limit: Option<u64>,
    priority: i64,
}

impl Request {
    fn of(params: &CrawlRequestEnqueueLinksParams) -> Self {
        Self {
            crawl_request_id: params.crawl_request_id.clone(),
            lease_epoch: params.lease_epoch,
            label: params.label.clone(),
            limit: params.limit,
            priority: params.priority,
        }
    }
}

/// A batch of links and all it admits against, owned by the write that
/// admits it, which may run it again at the commit.
struct OwnedBatch {
    request: Request,
    egress: Arc<Policy>,
    invariants: Invariants,
    links: Vec<Link>,
    base: Url,
    encoding: &'static goodfetch_frontier::Encoding,
    include: Patterns,
    exclude: Patterns,
    work: CountedWork,
}

impl OwnedBatch {
    /// Admits it at `positions`, as [`add`] does.
    fn add(&self, connection: &Connection, positions: &mut Positions) -> Answered<Admitted> {
        add(
            connection,
            positions,
            &self.egress,
            &self.request,
            &Batch {
                invariants: &self.invariants,
                links: &self.links,
                base: &self.base,
                encoding: self.encoding,
                narrowing: Narrowing {
                    include: &self.include,
                    exclude: &self.exclude,
                },
                work: &self.work,
            },
        )
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

/// One matched element: its `href`, if any, and what its link takes.
struct Link {
    href: Option<String>,
    /// Why it is a trap a person cannot see, or is suspected to be one
    /// (ADR 0035); none for one a person sees.
    trap: Option<Judged>,
    payload: Chosen,
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

    /// The mark of the request a link so judged added; `None` for a trap,
    /// which adds none.
    fn mark(self) -> Option<Mark> {
        match self {
            Self::Trap(_) => None,
            Self::Suspected(_) => Some(Mark::Hidden),
            Self::Revealable => Some(Mark::Revealable),
        }
    }

    /// Why a person may not see the link, for traces.
    fn name(self) -> &'static str {
        match self {
            Self::Trap(trap) | Self::Suspected(trap) => trap.name(),
            Self::Revealable => "revealable",
        }
    }
}

impl Link {
    /// How the link, answered `duplicate`, reaches the request it names
    /// again: as one a person sees (`Some(false)`), which clears its mark,
    /// as one a person reveals (`Some(true)`), which leaves it followed on
    /// every tier, or as a suspected trap (`None`), which leaves its mark
    /// as it is.
    fn reaching_again(&self) -> Option<bool> {
        match self.trap {
            None => Some(false),
            Some(Judged::Revealable) => Some(true),
            Some(Judged::Trap(_) | Judged::Suspected(_)) => None,
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

/// Notes with the staging of the attempt of `attempt`'s work item under its
/// lease epoch the requests `answered` added through links suspected traps:
/// their Provenance entries wait for the attempt's commit, which writes
/// those of the marks it committed (ADR 0035).
fn note_suspected(
    staging: &Staging,
    attempt: (&str, u64),
    answered: Option<&Result<Admitted, CallError>>,
) {
    if let Some(Ok(admitted)) = answered {
        staging.suspected(attempt.0, attempt.1, admitted.suspected.iter().cloned());
    }
}

/// What one call admitted: its answer, and the work items it added through
/// a link suspected a trap, each with its URL.
struct Admitted {
    links: EnqueuedLinks,
    suspected: Vec<(String, String)>,
}

/// The links of one call and what they resolve against, within the Run's
/// `invariants`.
struct Batch<'a> {
    invariants: &'a Invariants,
    links: &'a [Link],
    base: &'a Url,
    encoding: &'static goodfetch_frontier::Encoding,
    narrowing: Narrowing<'a>,
    work: &'a CountedWork,
}

impl Batch<'_> {
    /// The refusal of the first of its Payloads that does not fit `label`'s
    /// schema or the cap, each link that has one checked before any is admitted.
    fn refused_payload(&self, label: &str) -> Option<CallError> {
        let rules = self.invariants.payloads();
        self.links
            .iter()
            .find_map(|link| match (&link.href, &link.payload) {
                (Some(_), Chosen::Payload(payload)) => {
                    rules.check(label, payload.as_deref(), self.work).err()
                }
                _ => None,
            })
    }

    /// Clears the mark of the request that the link `href`, one that is not
    /// suspected, reaches again as a `duplicate`: a request stays marked
    /// only while every link that reached it was suspected; a link a person
    /// reveals (`revealable`) leaves it marked as one every tier follows
    /// (ADR 0035).
    fn seen_again(
        &self,
        connection: &Connection,
        href: &str,
        keeps_fragment: bool,
        revealable: bool,
    ) -> rusqlite::Result<()> {
        trap_links::seen_again(
            connection,
            href,
            self.base,
            self.encoding,
            keeps_fragment,
            revealable,
        )
    }
}

/// Admits each of `batch`'s links through the Frontier, each added at its
/// position from `positions`, within the Run's invariants and under its
/// domain policy `egress`, while `params`'s attempt holds its lease, once
/// every Payload fits its Label, else `payload_invalid` and none admitted;
/// `None` once the attempt ended.
fn add(
    connection: &Connection,
    positions: &mut Positions,
    egress: &Policy,
    params: &Request,
    batch: &Batch<'_>,
) -> Answered<Admitted> {
    let Some((url, depth)) =
        work_items::attempt(connection, &params.crawl_request_id, params.lease_epoch)?
    else {
        return Ok(None);
    };
    let Some(from) = document_base(&url, None) else {
        return Ok(None);
    };
    if let Some(refused) = batch.refused_payload(&params.label) {
        return Ok(Some(Err(refused)));
    }
    let keeps_fragment = batch.invariants.keeps_fragment(&params.label);
    let method = HttpMethod::get();
    let mut answers = Vec::with_capacity(batch.links.len());
    let mut added: u64 = 0;
    let mut suspected = Vec::new();
    for link in batch.links {
        let answer = match (&link.href, &link.payload) {
            (None, _) => EnqueuedLink::NoLink,
            (Some(href), _) if let Some(Judged::Trap(trap)) = link.trap => {
                tracing::debug!(
                    url = %href,
                    trap = trap.name(),
                    "a link a person cannot see was not added"
                );
                EnqueuedLink::NotAdded {
                    reason: NotAddedReason::TrapLink,
                    url: href.clone(),
                }
            }
            (Some(href), Chosen::Skipped) => EnqueuedLink::SkippedByPayload { url: href.clone() },
            (Some(href), Chosen::Payload(_))
                if params.limit.is_some_and(|limit| added >= limit) =>
            {
                EnqueuedLink::NotAdded {
                    reason: NotAddedReason::LinkLimit,
                    url: href.clone(),
                }
            }
            (Some(href), Chosen::Payload(payload)) => {
                let candidate = Candidate::Href {
                    href,
                    base: batch.base,
                    encoding: batch.encoding,
                    keeps_fragment,
                    from: &from,
                    narrowing: batch.narrowing,
                    depth: depth + 1,
                };
                let adding = Adding {
                    method: &method,
                    body: None,
                    label: &params.label,
                    payload: payload.as_deref(),
                    priority: params.priority,
                    forefront: false,
                    depth: depth + 1,
                };
                let enqueued = work_items::admit(
                    connection,
                    positions,
                    batch.invariants.frontier(),
                    egress,
                    candidate,
                    adding,
                )?;
                match enqueued {
                    Enqueued::Added { id, url } => {
                        added += 1;
                        if let Some((judged, mark)) =
                            link.trap.and_then(|judged| Some((judged, judged.mark()?)))
                        {
                            tracing::debug!(%url, trap = judged.name(), "a link a person may not see was added, marked as a suspected trap");
                            trap_links::suspect(connection, &id, mark)?;
                            suspected.push((id.clone(), url.clone()));
                        }
                        EnqueuedLink::Added { id, url }
                    }
                    Enqueued::NotAdded { reason, url } => {
                        if reason == NotAddedReason::Duplicate
                            && let Some(revealable) = link.reaching_again()
                        {
                            batch.seen_again(connection, href, keeps_fragment, revealable)?;
                        }
                        EnqueuedLink::NotAdded { reason, url }
                    }
                }
            }
        };
        answers.push(answer);
    }
    Ok(Some(Ok(Admitted {
        links: EnqueuedLinks { links: answers },
        suspected,
    })))
}
