//! Re-keying the visited set when a learned rule starts or stops: the
//! visited identities of the rule's hosts get their learned identities,
//! a host's set read a slice at a time, and the requests they show to be
//! one page are answered.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension as _};
use url::Url;

use super::{DECLARED, learn_identity, url_key, visitor};
use crate::rules;
use crate::unwritten::Holding;

/// The crawl requests the learned rules show to be one page, and the
/// rules that say so: each rule that changed the identity of one of them,
/// so the merge holds only while every one of those rules does.
#[derive(Debug, Default)]
pub(crate) struct OnePage {
    /// The crawl requests' IDs.
    pub(crate) requests: BTreeSet<String>,
    /// The rules their merge relies on.
    pub(crate) rules: BTreeSet<i64>,
}

/// Gives each visited identity of `merging`'s hosts that its rule changes
/// its learned identity, so a URL that differs from it only by what the
/// learned rules merge is a `duplicate` from then on, whichever of the two
/// was admitted; answers each set of crawl requests the rule just showed to
/// be one page, the requests admitted before it was learned among them.
/// It holds the identities the host's rules change, which those sets are
/// made of and whose order is the order the crawl ends them in; the host's
/// visited set it reads a slice at a time.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn merge_learned(
    connection: &Connection,
    merging: &rules::Merging,
) -> rusqlite::Result<Vec<OnePage>> {
    let mut pages: BTreeMap<String, (OnePage, bool)> = BTreeMap::new();
    visited_on(
        connection,
        &merging.hosts,
        |learned, rules, crawl_request| {
            let changed = rules.contains(&merging.rule);
            if changed {
                learn_identity(connection, &learned, merging.rule, &crawl_request)?;
            }
            let page = pages.entry(learned).or_default();
            page.0.requests.insert(crawl_request);
            page.0.rules.extend(rules);
            page.1 |= changed;
            Ok(())
        },
    )?;
    let mut groups = Vec::new();
    for (learned, (mut page, changed)) in pages {
        if !changed {
            continue;
        }
        page.requests.extend(visitor(connection, &learned)?);
        if page.requests.len() > 1 {
            groups.push(page);
        }
    }
    Ok(groups)
}

/// Gives every visited identity of `merging`'s hosts its learned identity
/// again under the rules that still hold, once `merging`'s rule stopped,
/// so nothing it merged merges a URL from then on.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn unmerge(connection: &Connection, merging: &rules::Merging) -> rusqlite::Result<()> {
    for (low, high) in host_ranges(connection, Keyed::Learned, &merging.hosts)? {
        connection
            .prepare_cached(
                "DELETE FROM learned_identities WHERE identity >= ?1 AND identity < ?2",
            )?
            .execute((low, high))?;
    }
    visited_on(
        connection,
        &merging.hosts,
        |learned, rules, crawl_request| match rules.last() {
            Some(rule) => learn_identity(connection, &learned, *rule, &crawl_request),
            None => Ok(()),
        },
    )
}

/// Brings the learned identities `holding`, memory under its lock, holds
/// up to those the writer's connection `connection` holds, for an
/// admission that writes once memory's learned rules are the writer's
/// (#2169): every one, the first time, then those on `hosts`, whose rules
/// that merge changed.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn keep_learned(
    connection: &Connection,
    holding: &mut Holding<'_>,
    hosts: &[String],
) -> rusqlite::Result<()> {
    if holding.rules().is_none() {
        return Ok(());
    }
    if !holding.learned_read() {
        return holding.relearn(connection, None);
    }
    if hosts.is_empty() {
        return Ok(());
    }
    // The methods come from the visited set: each learned identity takes
    // its method from a visited one, and a stopped rule's learned
    // identities have left the file, while memory may still hold them.
    let ranges = host_ranges(connection, Keyed::Visited, hosts)?;
    holding.relearn(connection, Some(&ranges))
}

/// The statement [`keep_learned`] steps along the visited identities'
/// methods with, named in the statements a Run prepares as it opens.
pub(crate) const FIRST_VISITED: &str =
    "SELECT identity FROM visited WHERE identity >= ?1 ORDER BY identity LIMIT 1";

/// A table of identities, read by ranges of its key.
#[derive(Clone, Copy, Debug)]
enum Keyed {
    Visited,
    Learned,
}

impl Keyed {
    /// The first identity the table holds from `?1` on.
    fn first_from(self) -> &'static str {
        match self {
            Self::Visited => FIRST_VISITED,
            Self::Learned => {
                "SELECT identity FROM learned_identities WHERE identity >= ?1 ORDER BY identity LIMIT 1"
            }
        }
    }
}

/// The ranges of `table`'s key that hold the identities of the URLs on
/// `hosts`, each from its first identity to past its last: for each method
/// the table holds, each scheme a crawl fetches and each host, the URLs
/// with no port and those with one. The methods are found by stepping
/// from one to the next along the key, one lookup each, so no read goes
/// past the rows of the hosts; an identity a program declared, `key:…`,
/// names no URL.
fn host_ranges(
    connection: &Connection,
    table: Keyed,
    hosts: &[String],
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut methods = Vec::new();
    let mut from = String::new();
    while let Some(identity) = connection
        .prepare_cached(table.first_from())?
        .query_row([&from], |row| row.get::<_, String>(0))
        .optional()?
    {
        from = if identity.starts_with(DECLARED) {
            // Past every declared key: `;` follows `:`.
            "key;".to_owned()
        } else if let Some((method, _)) = identity.split_once(' ') {
            // Past every identity of `method`: `!` follows the space.
            let next = format!("{method}!");
            methods.push(method.to_owned());
            next
        } else {
            format!("{identity}\u{1}")
        };
    }
    let mut ranges = Vec::new();
    for method in &methods {
        for scheme in crate::url::FETCHED {
            for host in hosts {
                let prefix = format!("{method} {scheme}://{host}");
                // `0` follows `/`, and `;` follows `:`.
                ranges.push((format!("{prefix}/"), format!("{prefix}0")));
                ranges.push((format!("{prefix}:"), format!("{prefix};")));
            }
        }
    }
    Ok(ranges)
}

/// How many visited identities a re-keying reads at once: what it holds of
/// a host's visited set, however many identities the host has.
pub(crate) const SLICE: usize = 256;

/// Hands `each` every visited identity whose URL is on one of `hosts` and
/// that the learned rules that hold change, in the order of the visited
/// set's key within each of the hosts' ranges: its identity under those
/// rules, the rules that changed it and its crawl request. The rules are
/// read once per host for the call, and the visited set [`SLICE`]
/// identities at a time, each slice read whole before `each` writes.
fn visited_on(
    connection: &Connection,
    hosts: &[String],
    mut each: impl FnMut(String, Vec<i64>, String) -> rusqlite::Result<()>,
) -> rusqlite::Result<()> {
    let mut holding = rules::Holding::of(connection);
    for (low, high) in host_ranges(connection, Keyed::Visited, hosts)? {
        let mut last: Option<String> = None;
        loop {
            let slice = visited_slice(connection, (&low, last.as_deref()), &high)?;
            for (visited, crawl_request) in &slice {
                let Some((method, rest)) = visited.split_once(' ') else {
                    continue;
                };
                let (key, hash) = rest
                    .split_once(' ')
                    .map_or((rest, None), |(key, hash)| (key, Some(hash)));
                let Ok(url) = Url::parse(key) else {
                    continue;
                };
                let (merged, rules) = holding.without_learned(&url)?;
                if rules.is_empty() {
                    continue;
                }
                let mut learned = format!("{method} {}", url_key(&merged));
                if let Some(hash) = hash {
                    learned.push(' ');
                    learned.push_str(hash);
                }
                each(learned, rules, crawl_request.clone())?;
            }
            if slice.len() < SLICE {
                break;
            }
            last = slice.into_iter().next_back().map(|(visited, _)| visited);
        }
    }
    Ok(())
}

/// At most [`SLICE`] visited identities with their crawl requests, in the
/// order of the key: those after `after`, or from `low` on when none was
/// read yet, and before `high`.
fn visited_slice(
    connection: &Connection,
    (low, after): (&str, Option<&str>),
    high: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let limit = i64::try_from(SLICE).unwrap_or(i64::MAX);
    let read = |row: &rusqlite::Row<'_>| Ok((row.get(0)?, row.get(1)?));
    match after {
        None => connection
            .prepare_cached(
                "SELECT identity, crawl_request FROM visited
                 WHERE identity >= ?1 AND identity < ?2 ORDER BY identity LIMIT ?3",
            )?
            .query_map((low, high, limit), read)?
            .collect(),
        Some(after) => connection
            .prepare_cached(
                "SELECT identity, crawl_request FROM visited
                 WHERE identity > ?1 AND identity < ?2 ORDER BY identity LIMIT ?3",
            )?
            .query_map((after, high, limit), read)?
            .collect(),
    }
}
