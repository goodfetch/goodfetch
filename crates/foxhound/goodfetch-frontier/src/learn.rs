//! What a fetched page teaches the Frontier (ADR 0005): `learn(page)`,
//! called once a tier fetched a crawl request's document, hands the
//! learned rules its URL and its page signature, and merges the visited
//! set under each parameter they then learn has no effect, so a URL that
//! differs from one admitted before only by it is a `duplicate`. Only a
//! `GET` with no body is evidence: another request's page says nothing of
//! its URL alone.

use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rusqlite::Connection;
use url::Url;

use crate::{identity, rules};

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
}

/// Learns from `page` in the run file `connection` holds, once per crawl
/// request.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub fn learn(connection: &Connection, page: PageSeen<'_>) -> rusqlite::Result<()> {
    if page.method.as_str() != "GET" || page.body.is_some_and(|body| !body.is_empty()) {
        return Ok(());
    }
    let key = identity::keyed(page.url);
    for no_effect in rules::learn(connection, page.crawl_request, &key, &page.signature)? {
        identity::merge_learned(connection, &no_effect)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
