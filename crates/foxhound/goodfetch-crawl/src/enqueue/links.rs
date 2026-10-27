//! A handler's `enqueueLinks` (ADRs 0093 and 0120): `crawl_request.links`
//! reads each element a locator matches on the page a fetch tier served
//! the attempt, as the program's Payload function reads it, and
//! `crawl_request.enqueue_links` admits each element's link as an `href`
//! candidate, resolved against the document's base URL with its query in
//! the document's encoding, under the call's own `include` and `exclude`,
//! and answers each element: added, not added with its reason, `no_link`
//! or `skipped_by_payload`. Every Payload is checked before any link is
//! admitted, so a `payload_invalid` adds none; the links are staged with
//! the attempt together, and commit or are discarded with it, as one
//! enqueue is (ADR 0093). On the browser tier, a link a person cannot see
//! is not added, with `trap_link` (ADR 0035; the Frontier's `trap_links`);
//! a served document has no layout, so a link its markup hides is added,
//! and marked as a suspected trap in its work item and in Provenance,
//! `trap_link_suspected`: a browser tier never follows it.

use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_trace;
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_fetch_tier::{FetchTiers, LinksUnread, ServedLinks};
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_frontier::trap_links::{self, Sight, Trap};
use goodfetch_frontier::{
    Candidate, Narrowing, Patterns, Scope, Url, admit, document_base, resolve_link,
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

use crate::labels::Fragments;
use crate::serving::{Crawls, LiveCrawl};
use crate::staging::Answered;
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
async fn served(tiers: &FetchTiers, locator: &Locator) -> Result<(ServedLinks, Url), CallError> {
    let served = tiers
        .links(&locator.page_id, &locator.selector)
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
    let (served, base) = served(&tiers, &params.locator).await?;
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
    if params.payload.is_some() && params.payloads.is_some() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "An `enqueueLinks` call takes `payload`, one Payload for every link, or `payloads`, one per link, never both.".to_owned(),
        ));
    }
    let include = Patterns::globs(&params.include).map_err(CallError::from)?;
    let exclude = Patterns::globs(&params.exclude).map_err(CallError::from)?;
    let Some(LiveCrawl {
        run,
        egress,
        staging,
        ..
    }) = crawls.live(&params.run_id).await
    else {
        return Err(attempt.ended());
    };
    let (served, base) = served(&tiers, &params.locator).await?;
    if let Some(payloads) = &params.payloads
        && payloads.len() != served.elements.len()
    {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`payloads` holds {} Payloads, but the locator `{}` matches {} elements: give one per element, in the order `crawl_request.links` read them.",
                payloads.len(),
                params.locator.selector,
                served.elements.len()
            ),
        ));
    }
    let links: Vec<Link> = served
        .elements
        .into_iter()
        .enumerate()
        .map(|(at, element)| Link {
            trap: match &element.sight {
                Sight::Layout(seen) => trap_links::judge(seen).map(Judged::Trap),
                Sight::Markup(seen) => trap_links::judge(seen).map(Judged::Suspected),
            },
            href: element.href,
            payload: match &params.payloads {
                Some(payloads) => match &payloads[at] {
                    Value::Null => Chosen::Skipped,
                    payload => Chosen::Payload(Some(payload.clone())),
                },
                None => Chosen::Payload(params.payload.clone()),
            },
        })
        .collect();
    let request = params.clone();
    let encoding = served.encoding;
    let answered = run
        .state()
        .write(move |connection| {
            let (crawl_request_id, lease_epoch) =
                (request.crawl_request_id.clone(), request.lease_epoch);
            staging.admit(
                connection,
                &crawl_request_id,
                lease_epoch,
                move |connection, positions| {
                    add(
                        connection,
                        positions,
                        &egress,
                        &request,
                        &Batch {
                            links: &links,
                            base: &base,
                            encoding,
                            narrowing: Narrowing {
                                include: &include,
                                exclude: &exclude,
                            },
                        },
                    )
                },
            )
        })
        .await;
    match answered {
        Ok(Some(Ok(admitted))) => {
            mark_suspected(&run, &params, &admitted.suspected).await;
            Ok(admitted.links)
        }
        Ok(Some(Err(refused))) => Err(refused),
        Ok(None) => Err(attempt.ended()),
        Err(error) => Err(attempt.unavailable(error)),
    }
}

/// What the program chose for one matched element's link.
enum Chosen {
    /// Its Payload, if any.
    Payload(Option<Value>),
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
    /// is added, marked as a suspected trap.
    Suspected(Trap),
}

/// Marks in the Run's Provenance, `trap_link_suspected`, each link of
/// `enqueued` that its served document's markup hides (ADR 0035), with
/// the request whose page held it.
async fn mark_suspected(
    run: &Run,
    params: &CrawlRequestEnqueueLinksParams,
    suspected: &[(String, String)],
) {
    for (id, url) in suspected {
        let entry = serde_json::json!({
            "action": trap_links::SUSPECTED_ACTION,
            "crawl_request_id": id,
            "url": url,
            "from": params.crawl_request_id,
        });
        let written =
            run_trace::provenance(run.owner(), run.folder(), run.secrets().mask(), &entry).await;
        if let Err(error) = written {
            tracing::warn!(%error, "a suspected trap link's Provenance entry cannot be written");
        }
    }
}

/// What one call admitted: its answer, and the work items it added through
/// a link suspected a trap, each with its URL.
struct Admitted {
    links: EnqueuedLinks,
    suspected: Vec<(String, String)>,
}

/// The links of one call and what they resolve against.
struct Batch<'a> {
    links: &'a [Link],
    base: &'a Url,
    encoding: &'static goodfetch_frontier::Encoding,
    narrowing: Narrowing<'a>,
}

/// Admits each of `batch`'s links through the Frontier, each added at its
/// position from `positions`, under the Run's
/// domain policy `egress`, while `params`'s attempt holds its lease, once
/// every Payload fits its Label, else `payload_invalid` and none admitted;
/// `None` once the attempt ended.
fn add(
    connection: &Connection,
    positions: &mut Positions,
    egress: &Policy,
    params: &CrawlRequestEnqueueLinksParams,
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
    let rules = PayloadRules::read(connection)?;
    for link in batch.links {
        if let (Some(_), Chosen::Payload(payload)) = (&link.href, &link.payload)
            && let Err(refused) = rules.check(&params.label, payload.as_ref())
        {
            return Ok(Some(Err(refused)));
        }
    }
    let scope = Scope::read(connection)?;
    let keeps_fragment = Fragments::read(connection)?.keeps(&params.label);
    let method = HttpMethod::get();
    let mut answers = Vec::with_capacity(batch.links.len());
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
                let enqueued = admit(connection, &scope, egress, candidate, |url| {
                    work_items::add(
                        connection,
                        positions,
                        url,
                        Adding {
                            method: &method,
                            body: None,
                            label: &params.label,
                            payload: payload.as_ref(),
                            priority: params.priority,
                            forefront: false,
                            depth: depth + 1,
                        },
                    )
                })?;
                match enqueued {
                    Enqueued::Added { id, url } => {
                        if let Some(Judged::Suspected(trap)) = link.trap {
                            tracing::debug!(%url, trap = trap.name(), "a link a served document's markup hides was added, marked as a suspected trap");
                            trap_links::suspect(connection, &id)?;
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
    Ok(Some(Ok(Admitted {
        links: EnqueuedLinks { links: answers },
        suspected,
    })))
}
