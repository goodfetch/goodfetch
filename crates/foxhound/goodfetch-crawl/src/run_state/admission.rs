//! The part of each admission that needs the Run's state (ADR 0142;
//! #2244): the attempt's lease, the visited set, the Bounds, a call's
//! `limit` and the add. The task that serves a call does the rest with no
//! run file, the Payload's check, the keying and the Scope, and asks the
//! Run state's actor for this part ([`super::asks`]), which is the only
//! one to run it: a seed added to the running crawl, a handler's enqueue,
//! a program's links batch and a code-free page's, which rides its
//! attempt's outcome.
//!
//! The visited set and the pending work items are still the run file's
//! (#2167), so each part here reads and writes it, in the job the actor
//! queues for it; none of them is called from outside the Run state's own
//! jobs, the commit of an attempt that deferred one, the Run's start, which
//! admits its seeds as it makes the run file, and tests.

use std::sync::Arc;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::CallError;
use goodfetch_engine::egress::Policy;
use goodfetch_frontier::payload::Payload;
use goodfetch_frontier::trap_links::{Mark, Trap};
use goodfetch_frontier::{Candidate, Declared, Entering, Keyed, Narrowing, Url, document_base};
use goodfetch_protocol::foxhound::crawl::Seed;
use goodfetch_protocol::foxhound::frontier::{
    CrawlRequestEnqueueLinksParams, CrawlRequestEnqueueParams, Enqueued, EnqueuedLink,
    EnqueuedLinks, HttpBody, HttpMethod, NotAddedReason, SeedAttempt,
};
use rusqlite::Connection;

use crate::invariants::Invariants;
use crate::staging::{Answered, Staging};
use crate::work_items::{self, Adding, CheckedSeed, Positions};

/// A handler's enqueue as its call's task read it: its params, its Payload
/// serialized once, and the base URL of the document it names, if any.
pub(crate) struct Enqueue {
    pub(crate) request: CrawlRequestEnqueueParams,
    pub(crate) payload: Option<Arc<Payload>>,
    pub(crate) document: Option<Url>,
}

/// Adds `params`'s request, carrying `payload`, through the Frontier, at
/// its position from `positions`, within the Run's `invariants` and under
/// its domain policy `egress`, while its attempt holds its lease, against
/// `document`'s base URL, else the attempt's own URL, once its Payload fits
/// its Label's schema and the cap, else `payload_invalid`; `None` once the
/// attempt ended.
pub(crate) fn enqueue(
    connection: &Connection,
    positions: &mut Positions,
    (egress, invariants): (&Policy, &Invariants),
    (params, payload): (&CrawlRequestEnqueueParams, Option<&Arc<Payload>>),
    document: Option<&Url>,
    work: &CountedWork,
) -> Answered<Enqueued> {
    let Some((url, depth)) =
        work_items::attempt(connection, &params.crawl_request_id, params.lease_epoch)?
    else {
        return Ok(None);
    };
    let Some(from) = document_base(&url, None) else {
        return Ok(None);
    };
    let checked = payload.map(|payload| &**payload);
    if let Err(refused) = invariants.payloads().check(&params.label, checked, work) {
        return Ok(Some(Err(refused)));
    }
    let base = document.cloned().unwrap_or_else(|| from.clone());
    let candidate = Candidate::Enqueue {
        url: &params.url,
        base: &base,
        from: &from,
        declared: Declared {
            method: &params.method,
            body: params.body.as_ref().map(HttpBody::bytes),
            key: params.key.as_deref(),
            keeps_fragment: invariants.keeps_fragment(&params.label),
        },
        narrowing: Narrowing::none(),
        depth: depth + 1,
    };
    let adding = Adding {
        method: &params.method,
        body: params.body.as_ref(),
        label: &params.label,
        payload,
        priority: params.priority,
        forefront: params.forefront,
        depth: depth + 1,
    };
    work_items::admit(connection, positions, invariants, egress, candidate, adding)
        .map(|enqueued| Some(Ok(enqueued)))
}

/// Admits `seed` within the Run's `invariants` and under its domain policy
/// `egress`, at its position from `positions`, while `attempt`, when
/// given, holds its lease, once its Payload fits its Label's schema and
/// the cap, else `payload_invalid`; `None` once that attempt ended. It
/// runs once: an attempt's commit applies what its staging decided
/// (#2245).
pub(crate) fn seed(
    connection: &Connection,
    positions: &mut Positions,
    (egress, invariants, work): (&Policy, &Invariants, &CountedWork),
    seed: &Seed,
    attempt: Option<&SeedAttempt>,
) -> Answered<Enqueued> {
    if let Some(attempt) = attempt
        && work_items::attempt(connection, &attempt.crawl_request_id, attempt.lease_epoch)?
            .is_none()
    {
        return Ok(None);
    }
    let checked = match work_items::check_seed(invariants, seed.clone(), work) {
        Ok(checked) => checked,
        Err(refused) => return Ok(Some(Err(refused))),
    };
    let mut answers = admit_seeds(
        connection,
        positions,
        (egress, invariants),
        std::slice::from_ref(&checked),
    )?;
    Ok(Some(Ok(answers
        .pop()
        .expect("invariant: the Frontier answers each seed"))))
}

/// Admits each of `seeds` within the Run's `invariants` and under its
/// domain policy `egress`, at its position from `positions`, and answers
/// each one's answer in order; none is admitted, with `payload_invalid`,
/// when one's Payload does not fit its Label's schema or the cap.
pub(crate) fn seeds(
    connection: &Connection,
    positions: &mut Positions,
    (egress, invariants, work): (&Policy, &Invariants, &CountedWork),
    seeds: Vec<Seed>,
) -> rusqlite::Result<Result<Vec<Enqueued>, CallError>> {
    let checked = match work_items::check_seeds(invariants, seeds, work) {
        Ok(checked) => checked,
        Err(refused) => return Ok(Err(refused)),
    };
    admit_seeds(connection, positions, (egress, invariants), &checked).map(Ok)
}

/// Admits each of `checked`, seeds whose Payloads were checked, within the
/// Run's `invariants` and under its domain policy `egress`, at its position
/// from `positions`, and answers each one's answer in order: the last step
/// of every seed's admission, a Run's start's among them. It is never
/// inlined, so the `admit_seeds` benchmark bounds its count to it (ADR
/// 0071).
#[inline(never)]
pub(crate) fn admit_seeds(
    connection: &Connection,
    positions: &mut Positions,
    (egress, invariants): (&Policy, &Invariants),
    checked: &[CheckedSeed],
) -> rusqlite::Result<Vec<Enqueued>> {
    work_items::admit_seeds(connection, positions, invariants, egress, checked)
}

/// What a links call's admission reads of its params.
pub(crate) struct LinksRequest {
    pub(crate) crawl_request_id: String,
    pub(crate) lease_epoch: u64,
    label: String,
    limit: Option<u64>,
    priority: i64,
}

impl LinksRequest {
    /// What `params`'s admission reads of them.
    pub(crate) fn of(params: &CrawlRequestEnqueueLinksParams) -> Self {
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
/// and all it admits against, owned by the job that admits it: its
/// call's own, or its attempt's outcome's for a code-free page.
pub(crate) struct LinksBatch {
    pub(crate) request: LinksRequest,
    pub(crate) invariants: Invariants,
    pub(crate) links: Vec<Link>,
    /// The refusal of the first of its Payloads that does not fit the
    /// Label's schema or the cap.
    pub(crate) refused_payload: Option<CallError>,
}

impl LinksBatch {
    /// Stages it with its attempt in `staging`, unanswered and unrun: its
    /// attempt's commit admits it, the first time, in its outcome's one
    /// job (ADR 0138). `None` once the attempt ended; refused as
    /// [`Staging::defer`] refuses.
    pub(crate) fn defer(self, staging: &Staging) -> Option<Result<(), CallError>> {
        let (work_item, lease_epoch) = (
            self.request.crawl_request_id.clone(),
            self.request.lease_epoch,
        );
        staging.defer(&work_item, lease_epoch, move |connection, positions| {
            Ok(match links(connection, positions, &self)? {
                Some(Ok(admitted)) => admitted.suspected,
                Some(Err(_)) | None => Vec::new(),
            })
        })
    }
}

/// One matched element, as the task that serves its call read it.
#[expect(
    clippy::large_enum_variant,
    reason = "nearly every link of a page is keyed, so the large variant is the common one, and a Box would add an allocation a link"
)]
pub(crate) enum Link {
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

/// Why a link that is added is suspected a trap.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Suspected {
    /// Its served document's markup hides it, which no layout confirms: its
    /// request is marked as one a browser tier never follows.
    Hidden(Trap),
    /// A menu, a tab or a disclosure a person opens hides it, on either
    /// tier: its request is marked as one every tier follows.
    Revealable,
}

impl Suspected {
    /// The mark of the request a link so suspected adds.
    const fn mark(self) -> Mark {
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
}

/// What one links call admitted: its answer, and the work items it added
/// through a link suspected a trap, each with its URL.
pub(crate) struct LinksAdmitted {
    pub(crate) links: EnqueuedLinks,
    pub(crate) suspected: Vec<(String, String)>,
}

/// Admits each of `batch`'s links through the Frontier, each as its call's
/// task keyed it and added at its position from `positions`, while the
/// call's attempt holds its lease, once every Payload fits its Label, else
/// `payload_invalid` and none admitted; `None` once the attempt ended. The
/// part of the call's admission that needs the Run's state (#2244): the
/// lease, the count against the call's `limit`, and the Frontier's, which
/// marks a request a link suspected a trap adds and tells one a link
/// reaches again how it was seen (ADR 0035). It runs once for a batch:
/// when its call stages it, or at its attempt's commit for a code-free
/// page's (#2245).
pub(crate) fn links(
    connection: &Connection,
    positions: &mut Positions,
    batch: &LinksBatch,
) -> Answered<LinksAdmitted> {
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
                    payload: payload.as_ref(),
                    priority: params.priority,
                    forefront: false,
                    depth: depth + 1,
                };
                let enqueued = work_items::admit_keyed(
                    connection,
                    positions,
                    &batch.invariants,
                    (href, keyed),
                    Entering {
                        from: Some(&from),
                        depth: depth + 1,
                    },
                    (adding, trap.map(Suspected::mark)),
                )?;
                match enqueued {
                    Enqueued::Added { id, url } => {
                        added += 1;
                        if let Some(trap) = trap {
                            tracing::debug!(%url, trap = trap.name(), "a link a person may not see was added, marked as a suspected trap");
                            suspected.push((id.clone(), url.clone()));
                        }
                        EnqueuedLink::Added { id, url }
                    }
                    Enqueued::NotAdded { reason, url } => EnqueuedLink::NotAdded { reason, url },
                }
            }
        };
        answers.push(answer);
    }
    Ok(Some(Ok(LinksAdmitted {
        links: EnqueuedLinks { links: answers },
        suspected,
    })))
}
