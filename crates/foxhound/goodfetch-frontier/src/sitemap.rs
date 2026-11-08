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
//! many of its entries were added and why the others were not; each one
//! the seed could not read, its fetch ended ([`ended`]) or its nesting
//! refused, keeps its URL and why (pitfall U19), so one bad sitemap never
//! stops the rest. [`reports`] gives them per seed, for the Run's progress
//! and report (ADR 0102).
//!
//! A file is read plain or gzip-compressed ([`read_file`]), a gzip one
//! decoded under the Run's response limits (ADR 0067), each within
//! sitemaps.org's 50,000 URLs and 50 MB uncompressed. The parser reads a
//! `urlset`'s or a `sitemapindex`'s `<loc>`s, by their local names, so a
//! namespace prefix changes nothing; their text is trimmed, its entities
//! and character references read, a CDATA section kept as written.

use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_engine::response_limits::{BodyFailure, Decoding, GZIP_MAGIC, LimitExceeded, Limits};
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_page::url_pattern::{UnsupportedPattern, UrlPattern};
use goodfetch_protocol::foxhound::crawl::{SitemapRule, SitemapSeed};
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use goodfetch_protocol::foxhound::outcomes::{Ending, SkipReason};
use goodfetch_protocol::foxhound::report::{
    CountOf, SITEMAP_FAILURES_LISTED, SitemapFailure, SitemapReport,
};
use goodfetch_protocol::response_limits::LimitSetting;
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use rusqlite::{Connection, OptionalExtension as _};
use url::Url;

use crate::admit::{Candidate, admit};
use crate::scope::Scope;
use crate::staged::Admitting;
use crate::url::parse as parse_url;

/// The crawl's sitemaps, each the fetch of one work item, with the rules
/// of the seed it came from, and how many of each one's entries were added
/// and not added, by reason; and each sitemap a seed could not read, by
/// URL, with why.
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
        "CREATE TABLE IF NOT EXISTS sitemap_failures (
            url TEXT NOT NULL PRIMARY KEY,
            seed TEXT NOT NULL,
            code TEXT NOT NULL,
            status INTEGER,
            message TEXT NOT NULL
        ) STRICT",
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

/// sitemaps.org's most URLs in one sitemap or sitemap index.
pub const MAX_ENTRIES: usize = 50_000;

/// sitemaps.org's most megabytes of one sitemap, uncompressed.
const MAX_MEGABYTES: usize = 50;

/// sitemaps.org's most bytes of one sitemap, uncompressed: 50 MB.
pub const MAX_BYTES: usize = MAX_MEGABYTES * 1024 * 1024;

/// Why a sitemap's file was not read.
#[derive(Debug)]
pub enum Refusal {
    /// It is no sitemap the crawl can read.
    Unreadable(Unreadable),
    /// Decoding it passed one of the Run's response limits (ADR 0067).
    Limit(LimitExceeded),
}

/// Reads the sitemap file `body` fetched at `url`, under `limits`. A file
/// that opens with gzip's two bytes (RFC 1952) is decoded first, whatever
/// its URL or type says, under the Run's `limits`, so a decompression bomb
/// is refused as a response's body is (ADR 0067), and never past one byte
/// more than sitemaps.org's 50 MB. Then a file larger than those 50 MB, or
/// listing more than its 50,000 URLs, is unreadable, with the limit named.
///
/// # Errors
///
/// Why it was not read: the limit its decoding passed, or why it is
/// unreadable.
pub fn read_file(url: &str, body: &[u8], limits: Limits) -> Result<Document, Refusal> {
    let decoded;
    let plain = if body.starts_with(&GZIP_MAGIC) {
        let past = u64::try_from(MAX_BYTES + 1).expect("invariant: 50 MB fits in u64");
        let capped = Limits {
            max_body_size: limits.max_body_size.min(past),
            ..limits
        };
        let mut decoding = Decoding::new(url, std::iter::once("gzip"), capped);
        decoded = decoding
            .push(body)
            .and_then(|()| decoding.finish())
            .map_err(|failure| match failure {
                BodyFailure::Limit(exceeded)
                    if exceeded.setting == LimitSetting::MaxBodySize && exceeded.limit == past =>
                {
                    Refusal::Unreadable(too_large())
                }
                BodyFailure::Limit(exceeded) => Refusal::Limit(exceeded),
                BodyFailure::Decoding => {
                    Refusal::Unreadable(Unreadable("its gzip stream does not decode".to_owned()))
                }
            })?;
        decoded.as_slice()
    } else {
        body
    };
    if plain.len() > MAX_BYTES {
        return Err(Refusal::Unreadable(too_large()));
    }
    parse(plain).map_err(Refusal::Unreadable)
}

/// The fuzz target's body for sitemaps (ADR 0067): reads `data` as a
/// sitemap file, plain XML or gzip, under the catalog's response limits; a
/// file that is read lists at most sitemaps.org's 50,000 URLs. It panics
/// only on a bug.
pub fn fuzz_sitemap(data: &[u8]) {
    if let Ok(Document::Entries(locs) | Document::Sitemaps(locs)) =
        read_file("http://shop.test/sitemap.xml", data, Limits::default())
    {
        assert!(
            locs.len() <= MAX_ENTRIES,
            "a sitemap read lists at most {} URLs",
            grouped(MAX_ENTRIES)
        );
    }
}

/// A file past sitemaps.org's [`MAX_BYTES`] uncompressed.
fn too_large() -> Unreadable {
    Unreadable(format!(
        "it is larger than sitemaps.org's {MAX_MEGABYTES} MB ({} bytes) uncompressed",
        grouped(MAX_BYTES)
    ))
}

/// `count` as its messages write it, its digits grouped by three with
/// commas: `52,428,800`.
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Reads the sitemap `body`: a `urlset`'s entries or a `sitemapindex`'s
/// sitemaps.
///
/// # Errors
///
/// Why it is no sitemap: no XML, XML that breaks off, another root, or
/// more than sitemaps.org's 50,000 URLs.
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
                    if locs.len() == MAX_ENTRIES {
                        return Err(Unreadable(format!(
                            "it lists more than sitemaps.org's {} URLs",
                            grouped(MAX_ENTRIES)
                        )));
                    }
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
    admit_seed_in(connection, policy, seed, &mut Admitting::RunFile, add)
}

/// Admits the sitemap seed `seed` as [`admit_seed`] does, against what
/// `admitting` adds to the run file, as [`crate::admit_in`] admits a
/// candidate: while an attempt stages, the sitemaps and seeds of its
/// earlier admissions count as added.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_seed_in(
    connection: &Connection,
    policy: &Policy,
    seed: &SitemapSeed,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let rules = serde_json::to_string(&seed.rules).expect("invariant: rules encode as JSON");
    let fetch = Fetch {
        given: &seed.sitemap,
        base: None,
        seed: None,
        rules: &rules,
    };
    let answer = admit_fetch(connection, policy, fetch, admitting, add)?;
    if let Enqueued::Added { url, .. } = &answer
        && let Ok(url) = Url::parse(url)
    {
        Scope::widen(connection, &url)?;
        admitting.widened(&url);
    }
    Ok(answer)
}

/// A sitemap's fetch to admit: the sitemap `given`, resolved against
/// `base` when given, of the seed `seed` (none for the seed itself), with
/// `rules`.
#[derive(Clone, Copy)]
struct Fetch<'a> {
    given: &'a str,
    base: Option<&'a Url>,
    seed: Option<&'a str>,
    rules: &'a str,
}

/// Admits the sitemap fetch `fetch`, against what `admitting` adds to the
/// run file.
fn admit_fetch(
    connection: &Connection,
    policy: &Policy,
    fetch: Fetch<'_>,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let Fetch {
        given,
        base,
        seed,
        rules,
    } = fetch;
    let url = match parse_url(given, base) {
        Ok(url) => url,
        Err(reason) => {
            return Ok(Enqueued::NotAdded {
                reason,
                url: given.to_owned(),
            });
        }
    };
    let not_added = |admitting: &mut Admitting, reason: NotAddedReason| {
        admitting.refused(&url, reason, None);
        Enqueued::NotAdded {
            reason,
            url: given.to_owned(),
        }
    };
    let request = Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    if let Judgment::Refused(_) = judge(&request, policy) {
        return Ok(not_added(admitting, NotAddedReason::DomainPolicy));
    }
    let known = connection
        .prepare_cached("SELECT 1 FROM sitemaps WHERE url = ?1")
        .and_then(|mut cached| cached.query_row([url.as_str()], |_| Ok(())))
        .optional()?
        .is_some()
        || admitting
            .earlier()
            .is_some_and(|staged| staged.holds_sitemap(url.as_str()));
    if known {
        return Ok(not_added(admitting, NotAddedReason::Duplicate));
    }
    let id = add(&url)?;
    connection
        .prepare_cached(
            "INSERT INTO sitemaps (work_item, url, seed, rules, read) VALUES (?1, ?2, ?3, ?4, 0)",
        )
        .and_then(|mut cached| cached.execute((&id, url.as_str(), seed.unwrap_or(&id), rules)))?;
    admitting.sitemap_added(url.as_str());
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
        .prepare_cached("SELECT 1 FROM sitemaps WHERE work_item = ?1")
        .and_then(|mut cached| cached.query_row([id], |_| Ok(())))
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
    work: &CountedWork,
    mut add: impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<()> {
    let Some((url, seed, given_rules)) = connection
        .prepare_cached("SELECT url, seed, rules FROM sitemaps WHERE work_item = ?1")
        .and_then(|mut cached| {
            cached.query_row([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
        })
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
                let fetch = Fetch {
                    given,
                    base: Some(&base),
                    seed: Some(&seed),
                    rules: &given_rules,
                };
                let answer =
                    admit_fetch(connection, policy, fetch, &mut Admitting::RunFile, |url| {
                        add(url, Adding::Sitemap)
                    })?;
                keep_refused_nesting(connection, &seed, &base, answer)?;
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
            for _ in &rules {
                work.count(Row::SitemapCompilations);
            }
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
    connection
        .prepare_cached("UPDATE sitemaps SET read = 1 WHERE work_item = ?1")
        .and_then(|mut cached| cached.execute([id]))?;
    // Read at last, after `run.retry_failed` re-admitted it.
    connection
        .prepare_cached("DELETE FROM sitemap_failures WHERE url = ?1")
        .and_then(|mut cached| cached.execute([&url]))?;
    let mut insert = connection.prepare_cached(
        "INSERT INTO sitemap_entries (work_item, reason, count) VALUES (?1, ?2, ?3)
         ON CONFLICT DO UPDATE SET count = count + excluded.count",
    )?;
    for (reason, count) in counts {
        insert.execute((id, reason, i64::try_from(count).unwrap_or(i64::MAX)))?;
    }
    Ok(())
}

/// Keeps, as a sitemap the seed `seed` could not read, the sitemap the
/// index at `base` nests when `answer` refused it: a nested sitemap is no
/// entry, and one the crawl already reads is read once, no failure.
fn keep_refused_nesting(
    connection: &Connection,
    seed: &str,
    base: &Url,
    answer: Enqueued,
) -> rusqlite::Result<()> {
    let Enqueued::NotAdded { reason, url } = answer else {
        return Ok(());
    };
    if reason == NotAddedReason::Duplicate {
        return Ok(());
    }
    let url = parse_url(&url, Some(base)).map_or(url, String::from);
    let message = if reason == NotAddedReason::DomainPolicy {
        format!(
            "The Run's domain policy refuses the host of the sitemap {url}, so it was not fetched."
        )
    } else {
        format!("The sitemap {url} names no URL the crawl fetches.")
    };
    keep_failure(
        connection,
        seed,
        &SitemapFailure {
            url,
            code: reason_name(reason),
            status: None,
            message,
        },
    )
}

/// The wire name of `$value`, a unit variant of the protocol.
macro_rules! name_of {
    ($value:expr) => {
        match serde_json::to_value($value) {
            Ok(serde_json::Value::String(name)) => name,
            _ => String::new(),
        }
    };
}

/// Keeps `failure`, a sitemap the seed `seed` could not read, in place of
/// what an earlier attempt kept of it.
fn keep_failure(
    connection: &Connection,
    seed: &str,
    failure: &SitemapFailure,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO sitemap_failures (url, seed, code, status, message)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut cached| {
            cached.execute((
                &failure.url,
                seed,
                &failure.code,
                failure.status,
                &failure.message,
            ))
        })?;
    Ok(())
}

/// Keeps how the fetch of the work item `id` ended, when it is a
/// sitemap's fetch that read nothing: its sitemap is one its seed could
/// not read, with its fetch's Error code and the status its site answered,
/// its skip reason or its Block cause (pitfall U19). Its read, which ends
/// it `succeeded`, kept the rest.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub fn ended(connection: &Connection, id: &str, ending: &Ending) -> rusqlite::Result<()> {
    let Some((url, seed)) = connection
        .prepare_cached("SELECT url, seed FROM sitemaps WHERE work_item = ?1")
        .and_then(|mut cached| {
            cached.query_row([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
        })
        .optional()?
    else {
        return Ok(());
    };
    let (code, status, message) = match ending {
        Ending::Succeeded { .. } => return Ok(()),
        Ending::Failed { error } => (
            error.code.clone(),
            error
                .details
                .get("status")
                .and_then(serde_json::Value::as_u64)
                .and_then(|status| u16::try_from(status).ok()),
            error.message.clone(),
        ),
        Ending::Skipped { reason, message } => {
            let name = name_of!(reason);
            let message = message.clone().unwrap_or_else(|| {
                if *reason == SkipReason::RobotsTxt {
                    format!(
                        "The site's robots.txt disallows the sitemap {url}, so it was not fetched."
                    )
                } else {
                    format!("The sitemap {url} was skipped, with `{name}`.")
                }
            });
            (name, None, message)
        }
        Ending::Blocked { cause } => {
            let name = name_of!(cause);
            let message = format!("The site blocked the sitemap {url}, with `{name}`.");
            (name, None, message)
        }
    };
    keep_failure(
        connection,
        &seed,
        &SitemapFailure {
            url,
            code,
            status,
            message,
        },
    )
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

/// Each sitemap seed the crawl admitted, in the order it was given, with
/// how many of its sitemaps were read and could not be, the first
/// [`SITEMAP_FAILURES_LISTED`] of those by URL with why, and how many of
/// their entries were added and not added, by reason (ADR 0102).
///
/// # Errors
///
/// When the run file cannot be read.
pub fn reports(connection: &Connection) -> rusqlite::Result<Vec<SitemapReport>> {
    let has_sitemaps = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'sitemaps'")
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some();
    if !has_sitemaps {
        return Ok(Vec::new());
    }
    let count = |count: i64| u64::try_from(count).unwrap_or_default();
    let mut read =
        connection.prepare_cached("SELECT url FROM sitemaps WHERE seed = ?1 AND read = 1")?;
    let mut entries = connection.prepare_cached(
        "SELECT entries.reason, sum(entries.count) FROM sitemap_entries AS entries
         JOIN sitemaps ON sitemaps.work_item = entries.work_item
         WHERE sitemaps.seed = ?1 GROUP BY entries.reason ORDER BY entries.reason",
    )?;
    let mut failed =
        connection.prepare_cached("SELECT count(*) FROM sitemap_failures WHERE seed = ?1")?;
    let mut failures = connection.prepare_cached(
        "SELECT url, code, status, message FROM sitemap_failures WHERE seed = ?1
         ORDER BY url LIMIT ?2",
    )?;
    let listed = i64::try_from(SITEMAP_FAILURES_LISTED).expect("invariant: 20 fits in i64");
    let seeds: Vec<(String, String)> = connection
        .prepare_cached(
            "SELECT work_item, url FROM sitemaps WHERE seed = work_item ORDER BY rowid",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    seeds
        .into_iter()
        .map(|(id, seed)| {
            let mut report = SitemapReport {
                seed,
                read: 0,
                failed: count(failed.query_row([&id], |row| row.get(0))?),
                failures: failures
                    .query_map((&id, listed), |row| {
                        Ok(SitemapFailure {
                            url: row.get(0)?,
                            code: row.get(1)?,
                            status: row.get(2)?,
                            message: row.get(3)?,
                        })
                    })?
                    .collect::<rusqlite::Result<_>>()?,
                added: 0,
                not_added: 0,
                not_added_by_reason: Vec::new(),
            };
            for url in read.query_map([&id], |row| row.get::<_, String>(0))? {
                // A robots.txt names sitemaps; it is none itself.
                if !Url::parse(&url?).is_ok_and(|url| is_robots_txt(&url)) {
                    report.read += 1;
                }
            }
            for row in entries.query_map([&id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })? {
                let (name, entries) = row?;
                if name == ADDED {
                    report.added = count(entries);
                } else {
                    report.not_added += count(entries);
                    report.not_added_by_reason.push(CountOf {
                        name,
                        count: count(entries),
                    });
                }
            }
            Ok(report)
        })
        .collect()
}

#[cfg(test)]
mod tests;
