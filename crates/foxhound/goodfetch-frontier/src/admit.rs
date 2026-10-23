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
//! seed added widens it. The bounds and the learned rules join behind this
//! one interface.

use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use rusqlite::Connection;
use url::Url;

use crate::identity::{Declared, identity, visit, visitor};
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
        /// How it is sent and identified.
        declared: Declared<'a>,
        /// The call's own patterns, which narrow the crawl's scope, as an
        /// `enqueueLinks` call's `include` and `exclude`.
        narrowing: Narrowing<'a>,
    },
}

/// Admits `candidate` into the run file `connection` holds, within
/// `scope`, which an added seed widens, through `add`, which adds it as a
/// crawl request at its URL as parsed, the URL it is fetched at, and
/// answers its ID.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit(
    connection: &Connection,
    scope: &Scope,
    candidate: Candidate<'_>,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let (given, base, declared, narrowing) = match candidate {
        Candidate::Seed { url, declared } => (url, None, declared, None),
        Candidate::Enqueue {
            url,
            base,
            declared,
            narrowing,
        } => (url, Some(base), declared, Some(narrowing)),
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
    let identity = identity(&url, declared);
    if visitor(connection, &identity)?.is_some() {
        return Ok(not_added(NotAddedReason::Duplicate));
    }
    let id = add(&url)?;
    visit(connection, &identity, &id)?;
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
