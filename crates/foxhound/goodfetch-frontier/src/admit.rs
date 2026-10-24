//! The Frontier's admission of a candidate (ADRs 0093, 0102 and 0120):
//! `admit(candidate, context)` answers every seed and every enqueue with an
//! `Enqueued` value, added with its ID or not added with its reason, so a
//! crawl that does nothing says why. A seed is checked as an enqueue is,
//! but takes only an absolute URL, since nothing gives it a base; an
//! enqueue resolves against its document's base URL. A candidate whose
//! identity the visited set holds is a `duplicate`; any other is added by
//! the context, where the crawl keeps its requests, and joins the set, in
//! the same transaction of the run file.
//!
//! An enqueue outside the crawl's scope, measured against every seed, is
//! `out_of_scope`; a seed is never, since seeds define the scope, and each
//! seed added widens it. A seed or an enqueue whose host the Run's domain
//! policy refuses (`egress.allowed_domains`, `egress.blocked_domains`,
//! through the engine's `egress::judge`) is then `domain_policy`, so an
//! off-site link the policy refuses too stays `out_of_scope` (ADRs 0017,
//! 0093 and 0120); both read the host as the WHATWG parser leaves it, the
//! scope alone ignoring one leading `www.`. One the crawl would otherwise
//! admit is then measured against its bounds, `crawl.max_depth` (seeds at depth 0) and
//! `crawl.max_requests`, and the caps on a URL's shape, each refusal
//! counted by its setting. The learned rules join behind this one
//! interface: a candidate that differs from an admitted request only by
//! parameters learned to have no effect is a `duplicate` merged by them,
//! one under a closed trap is not added with `trap_path`, and an offset
//! URL of a listing that prefers its cursor is a `duplicate`; each such
//! URL is recorded with its rule. What an admitted candidate shows, its
//! integer parameters and the page that linked it, teaches the rules in
//! turn.

use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use rusqlite::Connection;
use url::Url;

use crate::bounds::{Bounds, count_admitted, count_refused};
use crate::identity::{Declared, Seen, identities, keyed, seen, visit_all};
use crate::rules;
use crate::scope::{Narrowing, Scope};
use crate::url::parse;

/// What the Frontier is asked to admit.
#[derive(Clone, Copy, Debug)]
pub enum Candidate<'a> {
    /// A seed: an absolute URL, as the program gave it.
    Seed {
        /// Its URL.
        url: &'a str,
        /// How it is sent and identified.
        declared: Declared<'a>,
    },
    /// A handler's enqueue: a URL, relative to `base` or absolute.
    Enqueue {
        /// Its URL.
        url: &'a str,
        /// The base URL of the document the handler read.
        base: &'a Url,
        /// The URL of the crawl request whose attempt enqueued it, whose
        /// page gave the link.
        from: &'a Url,
        /// How it is sent and identified.
        declared: Declared<'a>,
        /// The call's own patterns, which narrow the crawl's scope, as an
        /// `enqueueLinks` call's `include` and `exclude`.
        narrowing: Narrowing<'a>,
        /// Its depth: one deeper than the request whose attempt added it.
        depth: u32,
    },
}

/// Admits `candidate` into the run file `connection` holds, within
/// `scope`, which an added seed widens, and under the Run's domain policy
/// `policy`, through `add`, which adds it as a
/// crawl request at its URL as parsed, the URL it is fetched at, and
/// answers its ID.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit(
    connection: &Connection,
    scope: &Scope,
    policy: &Policy,
    candidate: Candidate<'_>,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let (given, base, from, declared, narrowing, depth) = match candidate {
        Candidate::Seed { url, declared } => (url, None, None, declared, None, 0),
        Candidate::Enqueue {
            url,
            base,
            from,
            declared,
            narrowing,
            depth,
        } => (
            url,
            Some(base),
            Some(from),
            declared,
            Some(narrowing),
            depth,
        ),
    };
    let not_added = |reason: NotAddedReason| Enqueued::NotAdded {
        reason,
        url: given.to_owned(),
    };
    let url = match parse(given, base) {
        Ok(url) => url,
        Err(reason) => return Ok(not_added(reason)),
    };
    if let Some(narrowing) = narrowing
        && !scope.admits(connection, &url, narrowing)?
    {
        return Ok(not_added(NotAddedReason::OutOfScope));
    }
    let request = Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    if let Judgment::Refused(_) = judge(&request, policy) {
        return Ok(not_added(NotAddedReason::DomainPolicy));
    }
    let identities = identities(connection, &url, declared)?;
    match seen(connection, &identities)? {
        Seen::New => {}
        Seen::Visited(_) => return Ok(not_added(NotAddedReason::Duplicate)),
        Seen::Merged { rules, .. } => {
            for rule in rules {
                rules::merged(connection, rule, &url)?;
            }
            return Ok(not_added(NotAddedReason::Duplicate));
        }
    }
    let key = keyed(&url);
    if let Some((reason, rule)) = rules::refuses(connection, &key)? {
        rules::merged(connection, rule, &url)?;
        return Ok(not_added(reason));
    }
    if let Some(reason) = Bounds::read(connection)?.refuses(connection, &url, depth.into())? {
        count_refused(connection, reason)?;
        return Ok(not_added(reason));
    }
    let id = add(&url)?;
    count_admitted(connection)?;
    visit_all(connection, &identities, &id)?;
    rules::admitted(connection, &key, from.map(keyed).as_ref())?;
    if narrowing.is_none() {
        Scope::widen(connection, &url)?;
    }
    Ok(Enqueued::Added {
        id,
        url: url.into(),
    })
}

#[cfg(test)]
mod tests;
