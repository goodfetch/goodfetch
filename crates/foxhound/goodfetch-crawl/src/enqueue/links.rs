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
//! the job that holds the attempt, where the Run's state is: its lease,
//! the call's `limit`, and the Frontier's part that reads the visited set
//! and the Bounds, and adds. An attempt's commit runs the second part
//! again and the first never.

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_fetch_tier::{FetchTiers, LinksUnread, ServedLinks};
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::trap_links::{self, Mark, Sight, Trap, Verdict};
use goodfetch_frontier::{
    Entering, Keyed, Narrowing, Patterns, Url, document_base, key_link, resolve_link,
};
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
/// its Payloads fitted and checked, its patterns compiled and each link
/// keyed, on the task that serves the call, with no run file (#2244); one
/// batch's transaction counted.
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
    work.count(Row::BatchTransactions);
    let batch = OwnedBatch {
        request: Request::of(params),
        invariants,
        links,
        refused_payload,
    };
    Ok((run, staging, batch))
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

/// A batch of links as the task that serves its call read them, each keyed,
/// and all it admits against, owned by the write that admits it, which may
/// run it again at the commit.
struct OwnedBatch {
    request: Request,
    invariants: Invariants,
    links: Vec<Link>,
    /// The refusal of the first of its Payloads that does not fit the
    /// Label's schema or the cap.
    refused_payload: Option<CallError>,
}

impl OwnedBatch {
    /// Admits it at `positions`, as [`add`] does.
    fn add(&self, connection: &Connection, positions: &mut Positions) -> Answered<Admitted> {
        add(connection, positions, self)
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

/// One matched element, as the task that serves its call read it.
#[expect(
    clippy::large_enum_variant,
    reason = "nearly every link of a page is keyed, so the large variant is the common one, and a Box would add an allocation a link"
)]
enum Link {
    /// It has no `href`.
    NoHref,
    /// Its browser page's layout hides it (ADR 0035): it is not added.
    Trap { href: String, trap: Trap },
    /// Its Payload function skipped it.
    Skipped { href: String },
    /// One the Frontier judges, keyed.
    Keyed {
        href: String,
        keyed: Keyed,
        /// Its Payload, if any.
        payload: Option<Arc<Payload>>,
        /// Why it is suspected a trap; none for one a person sees.
        trap: Option<Suspected>,
    },
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

/// Why a link that is added is suspected a trap.
#[derive(Clone, Copy, Debug)]
enum Suspected {
    /// Its served document's markup hides it, which no layout confirms: its
    /// request is marked as one a browser tier never follows.
    Hidden(Trap),
    /// A menu, a tab or a disclosure a person opens hides it, on either
    /// tier: its request is marked as one every tier follows.
    Revealable,
}

impl Suspected {
    /// The mark of the request a link so suspected added.
    fn mark(self) -> Mark {
        match self {
            Self::Hidden(_) => Mark::Hidden,
            Self::Revealable => Mark::Revealable,
        }
    }

    /// Why a person may not see the link, for traces.
    fn name(self) -> &'static str {
        match self {
            Self::Hidden(trap) => trap.name(),
            Self::Revealable => "revealable",
        }
    }

    /// How a link so suspected, or one a person sees (`None`), answered
    /// `duplicate`, reaches the request it names again: as one a person
    /// sees (`Some(false)`), which clears its mark, as one a person reveals
    /// (`Some(true)`), which leaves it followed on every tier, or as a
    /// suspected trap (`None`), which leaves its mark as it is.
    fn reaching_again(suspected: Option<Self>) -> Option<bool> {
        match suspected {
            None => Some(false),
            Some(Self::Revealable) => Some(true),
            Some(Self::Hidden(_)) => None,
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

/// Admits each of `batch`'s links through the Frontier, each as its call's
/// task keyed it and added at its position from `positions`, while the
/// call's attempt holds its lease, once every Payload fits its Label, else
/// `payload_invalid` and none admitted; `None` once the attempt ended. The
/// part of the call's admission that needs the Run's state (#2244): the
/// lease, the count against the call's `limit`, and the Frontier's.
fn add(
    connection: &Connection,
    positions: &mut Positions,
    batch: &OwnedBatch,
) -> Answered<Admitted> {
    let params = &batch.request;
    let Some((url, depth)) =
        work_items::attempt(connection, &params.crawl_request_id, params.lease_epoch)?
    else {
        return Ok(None);
    };
    let Some(from) = document_base(&url, None) else {
        return Ok(None);
    };
    if let Some(refused) = &batch.refused_payload {
        return Ok(Some(Err(refused.clone())));
    }
    let method = HttpMethod::get();
    let mut answers = Vec::with_capacity(batch.links.len());
    let mut added: u64 = 0;
    let mut suspected = Vec::new();
    for link in &batch.links {
        let answer = match link {
            Link::NoHref => EnqueuedLink::NoLink,
            Link::Trap { href, trap } => {
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
            Link::Skipped { href } => EnqueuedLink::SkippedByPayload { url: href.clone() },
            Link::Keyed { href, .. } if params.limit.is_some_and(|limit| added >= limit) => {
                EnqueuedLink::NotAdded {
                    reason: NotAddedReason::LinkLimit,
                    url: href.clone(),
                }
            }
            Link::Keyed {
                href,
                keyed,
                payload,
                trap,
            } => {
                let adding = Adding {
                    method: &method,
                    body: None,
                    label: &params.label,
                    payload: payload.as_deref(),
                    priority: params.priority,
                    forefront: false,
                    depth: depth + 1,
                };
                let enqueued = work_items::admit_keyed(
                    connection,
                    positions,
                    batch.invariants.frontier(),
                    (href, keyed),
                    Entering {
                        from: Some(&from),
                        depth: depth + 1,
                    },
                    adding,
                )?;
                match enqueued {
                    Enqueued::Added { id, url } => {
                        added += 1;
                        if let Some(trap) = trap {
                            tracing::debug!(%url, trap = trap.name(), "a link a person may not see was added, marked as a suspected trap");
                            trap_links::suspect(connection, &id, trap.mark())?;
                            suspected.push((id.clone(), url.clone()));
                        }
                        EnqueuedLink::Added { id, url }
                    }
                    Enqueued::NotAdded { reason, url } => {
                        if reason == NotAddedReason::Duplicate
                            && let Some(revealable) = Suspected::reaching_again(*trap)
                        {
                            trap_links::seen_again(connection, keyed, revealable)?;
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
