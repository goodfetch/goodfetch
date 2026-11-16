//! A redirect chain's end judged on a reader beside a batching writer: its
//! identities are held as pending visits until the writer's job visits
//! them, so no admission or other chain takes them meanwhile.

use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rusqlite::Connection;
use url::Url;

use super::{Declared, Identities, Key, Seen, identities, seen, visit_all};
use crate::unwritten::{self, UnwrittenVisits};

/// What a redirect chain's end found, judged on a reader.
#[derive(Debug)]
pub enum Reached {
    /// Another request was admitted for, or reached, its key: the request
    /// ends `skipped` with `duplicate`.
    Duplicate,
    /// The request goes on: its key is held as pending, until the visit
    /// job made of it ([`visit_pending`]) runs.
    Follow(PendingVisit),
}

/// A chain's end, held as visited beside the run file until its visit job
/// runs or is dropped unrun.
pub struct PendingVisit {
    pending: unwritten::Pending,
    crawl_request: String,
    identities: Identities<'static>,
}

impl std::fmt::Debug for PendingVisit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingVisit")
            .field("crawl_request", &self.crawl_request)
            .finish_non_exhaustive()
    }
}

/// Where the redirect chain of the crawl request `crawl_request` ends,
/// at `url`, sent with `method` and `body`, judged on the reader
/// `connection` beside `unwritten`, the visits a batching writer has not
/// committed, as [`redirect_reached`](super::redirect_reached) judges it: on `Follow`, its
/// identities are held as pending, so no admission or other chain takes
/// them before its visit job runs.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn hold_reached(
    unwritten: &UnwrittenVisits,
    connection: &Connection,
    crawl_request: &str,
    (url, method, body): (&Url, &HttpMethod, Option<&[u8]>),
) -> rusqlite::Result<Reached> {
    let mut holding = unwritten.lock();
    let reached = reached_identities(connection, url, method, body)?;
    if duplicate(
        &seen(connection, &reached, None, Some(&holding))?,
        crawl_request,
    ) {
        return Ok(Reached::Duplicate);
    }
    let pending = holding.hold(reached.plain(), reached.learned(), crawl_request);
    Ok(Reached::Follow(PendingVisit {
        pending,
        crawl_request: crawl_request.to_owned(),
        identities: reached,
    }))
}

/// Visits `pending`, a chain's end held as pending, on the writer's
/// `connection`, as [`redirect_reached`](super::redirect_reached) visits it,
/// beside `unwritten`, the visits a batching writer has not committed,
/// when the Run holds them.
///
/// # Errors
///
/// When the run file cannot be written.
pub fn visit_pending(
    connection: &Connection,
    unwritten: Option<&UnwrittenVisits>,
    pending: PendingVisit,
) -> rusqlite::Result<()> {
    let PendingVisit {
        pending,
        crawl_request,
        identities,
    } = pending;
    let mut holding = unwritten.map(UnwrittenVisits::lock);
    visit_all(connection, &identities, &crawl_request)?;
    if let Some(holding) = &mut holding {
        holding.written(identities.plain(), identities.learned(), &crawl_request);
    }
    pending.done();
    Ok(())
}

/// The identities of a redirect chain's end at `url`, sent with `method`
/// and `body`.
pub(super) fn reached_identities(
    connection: &Connection,
    url: &Url,
    method: &HttpMethod,
    body: Option<&[u8]>,
) -> rusqlite::Result<Identities<'static>> {
    identities(
        connection,
        &Key::of(url, false),
        Declared {
            method,
            body,
            key: None,
            keeps_fragment: false,
        },
    )
}

/// Whether `seen` makes a hop of `crawl_request` a `duplicate`: another
/// request was admitted for, or reached, its key.
pub(super) fn duplicate(seen: &Seen, crawl_request: &str) -> bool {
    match seen {
        Seen::Visited(visitor)
        | Seen::Merged {
            crawl_request: visitor,
            ..
        } => visitor != crawl_request,
        Seen::New => false,
    }
}
