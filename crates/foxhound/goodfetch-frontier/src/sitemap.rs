//! A crawl's sitemap seeds (ADRs 0093, 0102 and 0120; CO-10; pitfall U20):
//! a sitemap seed names a sitemap, or a site's robots.txt whose `Sitemap:`
//! lines name its sitemaps, and is admitted as a sitemap fetch, a work item
//! of the crawl that runs no handler, after the Run's domain policy, a
//! sitemap the crawl already reads being a `duplicate`. The seed widens
//! the crawl's scope by the host of the URL it names. Its fetch, and each
//! nested one, waits in its domain's queue and goes through Politeness and
//! robots.txt as a page does (the crawl's delivery); what it read is
//! [`read`] here: a sitemap index's sitemaps are admitted as fetches of
//! their own, under the domain policy and never the scope, so a sitemap
//! the site cross-submits on another host is read; a sitemap's entries,
//! each resolved against its sitemap's URL, are given the Label of the
//! first rule of the seed whose pattern matches, and checked as an enqueue
//! is, at depth 0, with no Payload: one no rule matches is not added, with
//! `no_rule`, and one outside the scope with `out_of_scope`, so a
//! third-party sitemap never widens the crawl. Each sitemap read keeps how
//! many of its entries were added and why the others were not, which the
//! Run's report gives.
//!
//! The parser reads a `urlset`'s or a `sitemapindex`'s `<loc>`s, by their
//! local names, so a namespace prefix changes nothing; their text is
//! trimmed, its entities and character references read, a CDATA section
//! kept as written. gzip, and sitemaps.org's 50,000 entries and 50 MB, are
//! #909's.

use std::collections::BTreeMap;

use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_page::url_pattern::{UnsupportedPattern, UrlPattern};
use goodfetch_protocol::foxhound::crawl::{SitemapRule, SitemapSeed};
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use goodfetch_protocol::foxhound::report::{CountOf, SitemapReport};
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use rusqlite::{Connection, OptionalExtension as _};
use url::Url;

use crate::admit::{Candidate, admit};
use crate::scope::Scope;
use crate::url::parse as parse_url;

/// The crawl's sitemaps, each the fetch of one work item, with the rules
/// of the seed it came from, and how many of each one's entries were added
/// and not added, by reason.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-sitemaps",
    statements: &[
        "CREATE TABLE IF NOT EXISTS sitemaps (
            work_item TEXT NOT NULL PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            seed TEXT NOT NULL,
            rules TEXT NOT NULL,
            read INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS sitemap_entries (
            work_item TEXT NOT NULL,
            reason TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (work_item, reason)
        ) STRICT, WITHOUT ROWID",
    ],
};

/// The count of a sitemap's entries that were added, beside the reasons.
const ADDED: &str = "added";

/// A sitemap seed's rules, compiled: each a URL pattern and the Label it
/// gives the entries it matches.
#[derive(Clone, Debug)]
pub struct Rules {
    compiled: Vec<(UrlPattern, String)>,
}

impl Rules {
    /// `rules`, compiled.
    ///
    /// # Errors
    ///
    /// The first pattern the matcher cannot read, which the call that gave
    /// it raises as `pattern_unsupported`.
    pub fn of(rules: &[SitemapRule]) -> Result<Self, UnsupportedPattern> {
        let compiled = rules
            .iter()
            .map(|rule| Ok((UrlPattern::glob(&rule.pattern)?, rule.label.clone())))
            .collect::<Result<_, UnsupportedPattern>>()?;
        Ok(Self { compiled })
    }

    /// The Label of the first rule whose pattern matches `url`, if any.
    pub fn label_of(&self, url: &Url) -> Option<&str> {
        self.compiled
            .iter()
            .find(|(pattern, _)| pattern.matches(url.as_str()))
            .map(|(_, label)| label.as_str())
    }
}

/// What a sitemap lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Document {
    /// A `urlset`'s entries, as written.
    Entries(Vec<String>),
    /// A sitemap index's sitemaps, or those a robots.txt's `Sitemap:`
    /// lines name, as written.
    Sitemaps(Vec<String>),
}

/// Why a file is no sitemap the crawl can read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable(pub String);

impl std::fmt::Display for Unreadable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reads the sitemap `body`: a `urlset`'s entries or a `sitemapindex`'s
/// sitemaps.
///
/// # Errors
///
/// Why it is no sitemap: no XML, XML that breaks off, or another root.
pub fn parse(body: &[u8]) -> Result<Document, Unreadable> {
    let unreadable = |error: &dyn std::fmt::Display| Unreadable(format!("its XML breaks: {error}"));
    let mut reader = Reader::from_reader(body);
    let mut root: Option<bool> = None;
    let mut locs = Vec::new();
    // The `<loc>` being read, and the depth of the open elements.
    let mut loc: Option<String> = None;
    // The prefix of the open `<url>` or `<sitemap>`, when one is open.
    let mut parent: Option<Option<Vec<u8>>> = None;
    let mut depth = 0_usize;
    loop {
        let event = reader.read_event().map_err(|error| unreadable(&error))?;
        match event {
            Event::Start(element) => {
                depth += 1;
                let name = element.local_name();
                match (depth, name.as_ref()) {
                    (1, b"urlset") => root = Some(false),
                    (1, b"sitemapindex") => root = Some(true),
                    (1, _) => return Err(no_sitemap()),
                    (2, child) => {
                        let listed: &[u8] = if root == Some(true) {
                            b"sitemap"
                        } else {
                            b"url"
                        };
                        parent = (child == listed).then(|| prefix_of(&element));
                    }
                    (3, b"loc") if parent.as_ref() == Some(&prefix_of(&element)) => {
                        loc = Some(String::new());
                    }
                    _ => {}
                }
            }
            Event::Empty(element) if depth == 0 => {
                return match element.local_name().as_ref() {
                    b"urlset" => Ok(Document::Entries(Vec::new())),
                    b"sitemapindex" => Ok(Document::Sitemaps(Vec::new())),
                    _ => Err(no_sitemap()),
                };
            }
            Event::End(_) => {
                if depth == 3
                    && let Some(text) = loc.take()
                {
                    locs.push(text.trim().to_owned());
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) => {
                if let Some(loc) = loc.as_mut() {
                    loc.push_str(&text.decode().map_err(|error| unreadable(&error))?);
                }
            }
            Event::CData(data) => {
                if let Some(loc) = loc.as_mut() {
                    loc.push_str(&data.decode().map_err(|error| unreadable(&error))?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(loc) = loc.as_mut() {
                    if let Some(character) = reference
                        .resolve_char_ref()
                        .map_err(|error| unreadable(&error))?
                    {
                        loc.push(character);
                    } else {
                        let name = reference.decode().map_err(|error| unreadable(&error))?;
                        let text = resolve_predefined_entity(&name).ok_or_else(|| {
                            Unreadable(format!("it uses the undeclared entity `&{name};`"))
                        })?;
                        loc.push_str(text);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth > 0 {
        return Err(Unreadable("its XML ends inside an element".to_owned()));
    }
    match root {
        Some(false) => Ok(Document::Entries(locs)),
        Some(true) => Ok(Document::Sitemaps(locs)),
        None => Err(no_sitemap()),
    }
}

/// A file whose root is neither `urlset` nor `sitemapindex`.
fn no_sitemap() -> Unreadable {
    Unreadable("it is no sitemap: its root is neither `urlset` nor `sitemapindex`".to_owned())
}

/// The namespace prefix `element` is written with, if any: a `<loc>` is
/// an entry's only under the prefix of its `<url>` or `<sitemap>`, so an
/// extension's (`image:loc`) is none.
fn prefix_of(element: &BytesStart<'_>) -> Option<Vec<u8>> {
    element
        .name()
        .prefix()
        .map(|prefix| prefix.as_ref().to_vec())
}

/// Whether `url` names a site's robots.txt, whose `Sitemap:` lines a
/// sitemap seed reads: its path `/robots.txt`, with no query.
pub fn is_robots_txt(url: &Url) -> bool {
    url.path() == "/robots.txt" && url.query().is_none()
}

/// Admits the sitemap seed `seed` into the run file `connection` holds,
/// under the Run's domain policy `policy`, through `add`, which adds its
/// fetch as a work item at its URL as parsed and answers its ID. An added
/// seed widens the scope by its URL's host.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_seed(
    connection: &Connection,
    policy: &Policy,
    seed: &SitemapSeed,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let rules = serde_json::to_string(&seed.rules).expect("invariant: rules encode as JSON");
    let answer = admit_fetch(connection, policy, &seed.sitemap, None, None, &rules, add)?;
    if let Enqueued::Added { url, .. } = &answer
        && let Ok(url) = Url::parse(url)
    {
        Scope::widen(connection, &url)?;
    }
    Ok(answer)
}

/// Admits the fetch of the sitemap `given`, resolved against `base` when
/// given, the seed `seed`'s (none for the seed itself) with `rules`.
fn admit_fetch(
    connection: &Connection,
    policy: &Policy,
    given: &str,
    base: Option<&Url>,
    seed: Option<&str>,
    rules: &str,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let not_added = |reason: NotAddedReason| Enqueued::NotAdded {
        reason,
        url: given.to_owned(),
    };
    let url = match parse_url(given, base) {
        Ok(url) => url,
        Err(reason) => return Ok(not_added(reason)),
    };
    let request = Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    if let Judgment::Refused(_) = judge(&request, policy) {
        return Ok(not_added(NotAddedReason::DomainPolicy));
    }
    let known = connection
        .query_row(
            "SELECT 1 FROM sitemaps WHERE url = ?1",
            [url.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if known {
        return Ok(not_added(NotAddedReason::Duplicate));
    }
    let id = add(&url)?;
    connection.execute(
        "INSERT INTO sitemaps (work_item, url, seed, rules, read) VALUES (?1, ?2, ?3, ?4, 0)",
        (&id, url.as_str(), seed.unwrap_or(&id), rules),
    )?;
    Ok(Enqueued::Added {
        id,
        url: url.into(),
    })
}

/// Whether the work item `id` is a sitemap's fetch.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn is_fetch(connection: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row("SELECT 1 FROM sitemaps WHERE work_item = ?1", [id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

/// What a sitemap's read adds: an entry, with its Label, or the fetch of
/// a sitemap it nests.
#[derive(Clone, Copy, Debug)]
pub enum Adding<'a> {
    /// An entry, a crawl request of this Label with no Payload.
    Entry(&'a str),
    /// A nested sitemap's fetch.
    Sitemap,
}

/// Reads `document`, what the fetch of the work item `id` gave, into the
/// run file `connection` holds, within `scope` and under the Run's domain
/// policy `policy`, through `add`, which adds a work item at its URL as
/// parsed and answers its ID; and keeps how many of its entries were added
/// and why the others were not. A work item that is no sitemap's fetch
/// reads nothing.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn read(
    connection: &Connection,
    scope: &Scope,
    policy: &Policy,
    id: &str,
    document: &Document,
    mut add: impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<()> {
    let Some((url, seed, given_rules)) = connection
        .query_row(
            "SELECT url, seed, rules FROM sitemaps WHERE work_item = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(());
    };
    let base = Url::parse(&url).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut count = |answer: &Enqueued| {
        let name = match answer {
            Enqueued::Added { .. } => ADDED.to_owned(),
            Enqueued::NotAdded { reason, .. } => reason_name(*reason),
        };
        *counts.entry(name).or_insert(0) += 1;
    };
    match document {
        Document::Sitemaps(sitemaps) => {
            for given in sitemaps {
                let answer = admit_fetch(
                    connection,
                    policy,
                    given,
                    Some(&base),
                    Some(&seed),
                    &given_rules,
                    |url| add(url, Adding::Sitemap),
                )?;
                count(&answer);
            }
        }
        Document::Entries(entries) => {
            let rules: Vec<SitemapRule> = serde_json::from_str(&given_rules).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            // The rules were compiled when the seed was given.
            let rules = Rules::of(&rules).unwrap_or(Rules {
                compiled: Vec::new(),
            });
            for given in entries {
                let answer =
                    admit_entry(connection, scope, policy, &base, given, &rules, &mut add)?;
                count(&answer);
            }
        }
    }
    connection.execute("UPDATE sitemaps SET read = 1 WHERE work_item = ?1", [id])?;
    let mut insert = connection.prepare(
        "INSERT INTO sitemap_entries (work_item, reason, count) VALUES (?1, ?2, ?3)
         ON CONFLICT DO UPDATE SET count = count + excluded.count",
    )?;
    for (reason, count) in counts {
        insert.execute((id, reason, i64::try_from(count).unwrap_or(i64::MAX)))?;
    }
    Ok(())
}

/// Admits the entry `given` of the sitemap at `base` as a crawl request
/// of its first matching rule's Label, or answers `no_rule`.
fn admit_entry(
    connection: &Connection,
    scope: &Scope,
    policy: &Policy,
    base: &Url,
    given: &str,
    rules: &Rules,
    add: &mut impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let not_added = |reason: NotAddedReason| Enqueued::NotAdded {
        reason,
        url: given.to_owned(),
    };
    let url = match parse_url(given, Some(base)) {
        Ok(url) => url,
        Err(reason) => return Ok(not_added(reason)),
    };
    let Some(label) = rules.label_of(&url) else {
        return Ok(not_added(NotAddedReason::NoRule));
    };
    admit(
        connection,
        scope,
        policy,
        Candidate::SitemapEntry {
            url: given,
            sitemap: base,
        },
        |url| add(url, Adding::Entry(label)),
    )
}

/// The wire name of `reason`.
fn reason_name(reason: NotAddedReason) -> String {
    match serde_json::to_value(reason) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// Each sitemap the crawl read, in the order their fetches were added,
/// with how many of its entries were added and not added, by reason.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn reports(connection: &Connection) -> rusqlite::Result<Vec<SitemapReport>> {
    let has_sitemaps = connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'sitemaps'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !has_sitemaps {
        return Ok(Vec::new());
    }
    let mut counts = connection.prepare(
        "SELECT reason, count FROM sitemap_entries WHERE work_item = ?1 ORDER BY reason",
    )?;
    let read: Vec<(String, String)> = connection
        .prepare("SELECT work_item, url FROM sitemaps WHERE read = 1 ORDER BY rowid")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    read.into_iter()
        .map(|(id, url)| {
            let mut report = SitemapReport {
                url,
                added: 0,
                not_added: 0,
                not_added_by_reason: Vec::new(),
            };
            for row in counts.query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })? {
                let (name, count) = row?;
                let count = u64::try_from(count).unwrap_or_default();
                if name == ADDED {
                    report.added = count;
                } else {
                    report.not_added += count;
                    report.not_added_by_reason.push(CountOf { name, count });
                }
            }
            Ok(report)
        })
        .collect()
}

#[cfg(test)]
mod tests;
