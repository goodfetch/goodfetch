//! What a fetched page and a redirect teach the Frontier (ADRs 0005 and
//! 0120): `learn(page)`, called once a tier fetched a crawl request's
//! document, hands the learned rules its URL, its page signature and its
//! `rel=canonical`; `learn_redirect(redirect)`, called for each hop a tier
//! follows, hands them the two URLs. When they learn a parameter with no
//! effect or an equivalent form, the visited set is merged under it, so a
//! URL that differs from one admitted before only by what it merges is a
//! `duplicate`, and each request admitted before that it shows to be the
//! same page as another and that is still pending is handed to the crawl,
//! which ends it `skipped` with `duplicate`; when a contradicting pair
//! stops one, the visited set is merged again without it. Only a `GET`
//! with no body is evidence: another request's page says nothing of its
//! URL alone.

use std::collections::BTreeSet;

use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rusqlite::Connection;
use url::Url;

use crate::identity;
use crate::rules::{self, Change};

/// A crawl request's page as a tier fetched it.
#[derive(Clone, Copy, Debug)]
pub struct PageSeen<'a> {
    /// The crawl request's ID.
    pub crawl_request: &'a str,
    /// The URL its document was fetched at.
    pub url: &'a Url,
    /// The method it was sent with.
    pub method: &'a HttpMethod,
    /// The body it was sent with, if any.
    pub body: Option<&'a [u8]>,
    /// Its page signature: the SHA-256 of its document's title and main
    /// text (`goodfetch_extractor::page_signature`).
    pub signature: [u8; 32],
    /// The URL its `rel=canonical` names, resolved: the first
    /// `<link rel="canonical">` in its `<head>`, or else a `Link` header's
    /// with `rel="canonical"`; `None` when it names none.
    pub canonical: Option<&'a Url>,
}

/// One redirect hop of a crawl request a tier followed.
#[derive(Clone, Copy, Debug)]
pub struct Redirect<'a> {
    /// The URL that answered with the redirect.
    pub from: &'a Url,
    /// The URL its `Location` named, resolved.
    pub to: &'a Url,
    /// The method the request was sent with.
    pub method: &'a HttpMethod,
    /// The body it was sent with, if any.
    pub body: Option<&'a [u8]>,
}

/// What the crawl does with the requests a rule learned later merged:
/// given the IDs of crawl requests a rule just showed to be one page, it
/// ends each one still pending `skipped` with `duplicate`, keeping the one
/// it fetched or is fetching, or else the first, and answers the URL of
/// each it ended, which the rule records as merged.
pub trait EndMerged {
    /// Ends the pending ones of `crawl_requests`.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read or written.
    fn end_merged(
        &mut self,
        connection: &Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<String>>;
}

/// Learns from `page` in the run file `connection` holds, once per crawl
/// request, handing `pending` the requests a rule it teaches merges.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub fn learn(
    connection: &Connection,
    page: PageSeen<'_>,
    pending: &mut dyn EndMerged,
) -> rusqlite::Result<()> {
    if !is_evidence(page.method, page.body) {
        return Ok(());
    }
    let key = identity::keyed(page.url);
    let canonical = page.canonical.map(identity::keyed);
    let changes = rules::learn(
        connection,
        &rules::Page {
            crawl_request: page.crawl_request,
            key: &key,
            signature: &page.signature,
            canonical: canonical.as_ref(),
        },
    )?;
    apply(connection, changes, pending)
}

/// Learns from `redirect` in the run file `connection` holds, handing
/// `pending` the requests a rule it teaches merges.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub fn learn_redirect(
    connection: &Connection,
    redirect: Redirect<'_>,
    pending: &mut dyn EndMerged,
) -> rusqlite::Result<()> {
    if !is_evidence(redirect.method, redirect.body) {
        return Ok(());
    }
    let change = rules::learn_redirect(
        connection,
        &identity::keyed(redirect.from),
        &identity::keyed(redirect.to),
    )?;
    apply(connection, change.into_iter().collect(), pending)
}

/// Whether a request sent with `method` and `body` says anything of its
/// URL alone: a `GET` with no body.
fn is_evidence(method: &HttpMethod, body: Option<&[u8]>) -> bool {
    method.as_str() == "GET" && body.is_none_or(<[u8]>::is_empty)
}

/// Merges the visited set under each rule learned, ending the pending
/// requests it merged, and again without each rule stopped.
fn apply(
    connection: &Connection,
    changes: Vec<Change>,
    pending: &mut dyn EndMerged,
) -> rusqlite::Result<()> {
    for change in changes {
        match change {
            Change::Learned(merging) => {
                for group in identity::merge_learned(connection, &merging)? {
                    for url in pending.end_merged(connection, &group)? {
                        rules::merged(connection, merging.rule, &url)?;
                    }
                }
            }
            Change::Contradicted(merging) => identity::unmerge(connection, &merging)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
