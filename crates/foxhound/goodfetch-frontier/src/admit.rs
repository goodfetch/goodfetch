//! The Frontier's admission of a candidate (ADRs 0093, 0102 and 0120):
//! `admit(candidate, context)` answers every seed and every enqueue with an
//! `Enqueued` value, added with its ID or not added with its reason, so a
//! crawl that does nothing says why. A seed is checked as an enqueue is,
//! but takes only an absolute URL, since nothing gives it a base; an
//! enqueue resolves against its document's base URL. The context, where
//! the crawl keeps its requests, adds a request unless one of the same
//! identity is already there, exactly, in its run state.
//!
//! A request's identity is its URL as parsed for now; the identity rules
//! (method, the URL's key and the body's hash, ADR 0093), the scope, the
//! bounds and the learned rules join behind this one interface.

use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use url::Url;

use crate::url::parse;

/// What the Frontier is asked to admit.
#[derive(Clone, Copy, Debug)]
pub enum Candidate<'a> {
    /// A seed: an absolute URL, as the program gave it.
    Seed {
        /// Its URL.
        url: &'a str,
    },
    /// A handler's enqueue: a URL, relative to `base` or absolute.
    Enqueue {
        /// Its URL.
        url: &'a str,
        /// The base URL of the document the handler read.
        base: &'a Url,
    },
}

/// A candidate the Frontier parsed and identified, for its context to add.
#[derive(Clone, Copy, Debug)]
pub struct Identified<'a> {
    /// Its URL as parsed: the URL it is fetched at.
    pub url: &'a Url,
    /// Its identity: two requests of one identity are one request.
    pub identity: &'a str,
}

/// Admits `candidate` through `add`, which adds it as a crawl request and
/// answers its ID, or `None` when a request of its identity is already
/// there.
///
/// # Errors
///
/// What `add` fails with.
pub fn admit<E>(
    candidate: Candidate<'_>,
    add: impl FnOnce(Identified<'_>) -> Result<Option<String>, E>,
) -> Result<Enqueued, E> {
    let (given, base) = match candidate {
        Candidate::Seed { url } => (url, None),
        Candidate::Enqueue { url, base } => (url, Some(base)),
    };
    let not_added = |reason: NotAddedReason| Enqueued::NotAdded {
        reason,
        url: given.to_owned(),
    };
    let url = match parse(given, base) {
        Ok(url) => url,
        Err(reason) => return Ok(not_added(reason)),
    };
    Ok(
        match add(Identified {
            url: &url,
            identity: url.as_str(),
        })? {
            Some(id) => Enqueued::Added {
                id,
                url: url.into(),
            },
            None => not_added(NotAddedReason::Duplicate),
        },
    )
}

#[cfg(test)]
mod tests;
