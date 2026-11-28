//! A crawl's syndication feed seeds (ADRs 0093, 0102, 0119 and 0139): a
//! syndication seed names an RSS 2.0, RSS 1.0 (RDF) or Atom 1.0 feed and is
//! admitted as a source fetch, a work item of the crawl that runs no handler,
//! after the Run's domain policy, a feed the crawl already reads being a
//! `duplicate`. The seed widens the crawl's scope by the host of the URL it
//! names. Its fetch waits in its domain's queue and goes through Politeness
//! and robots.txt as a page does; what it read is [`read`] here: each entry's
//! link, resolved against the feed's URL, is given the Label of the first
//! matching rule and checked as an enqueue is, at depth 0, with no Payload:
//! one no rule matches is not added with `no_rule`, and one outside the scope
//! with `out_of_scope`. Each source keeps how many times it was read, how many
//! times it failed with its latest failure, and how many of its entries were
//! added and not added by reason, so an unreadable feed is reported with its
//! URL and why while the others go on.
//!
//! A feed file is read plain or gzip-compressed ([`read_file`]), within the
//! same 50 MB uncompressed and 50,000-entry limits as sitemaps and the Run's
//! response limits (ADR 0067).

use std::collections::BTreeMap;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::crawl::SyndicationSeed;
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use goodfetch_protocol::foxhound::outcomes::{Ending, SkipReason};
use goodfetch_protocol::foxhound::report::{CountOf, SourceFailure, SourceReport};
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use rusqlite::{Connection, OptionalExtension as _};
use url::Url;

use crate::admit::{Candidate, widen};
use crate::invariants::Invariants;
use crate::sitemap::{
    MAX_BYTES, MAX_ENTRIES, Refusal, Unreadable, admit_source_entry, decode_body, grouped,
    reason_name,
};
use crate::staged::Admitting;
use crate::url::parse as parse_url;

/// The crawl's syndication sources, each the fetch of one work item, with the
/// rules of the seed it came from, its read and failure counts, its entries'
/// admission counts by reason, and its latest failure when unreadable.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-syndication",
    statements: &[
        "CREATE TABLE IF NOT EXISTS syndication_sources (
            work_item TEXT NOT NULL PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            rules TEXT NOT NULL,
            reads INTEGER NOT NULL,
            failures INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS syndication_entries (
            work_item TEXT NOT NULL,
            reason TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (work_item, reason)
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS syndication_failures (
            work_item TEXT NOT NULL PRIMARY KEY,
            url TEXT NOT NULL,
            code TEXT NOT NULL,
            status INTEGER,
            message TEXT NOT NULL
        ) STRICT",
    ],
};

/// The count of a feed's entries that were added, beside the refusal reasons.
const ADDED: &str = "added";

/// The statements [`read`] runs, which [`crate::admission_statements`] has the
/// Run prepare as it starts, so a feed read prepares no statement on its path
/// (ADR 0140, F3).
pub(crate) const READ: [&str; 4] = [
    "SELECT url, rules FROM syndication_sources WHERE work_item = ?1",
    "UPDATE syndication_sources SET reads = reads + 1 WHERE work_item = ?1",
    "DELETE FROM syndication_failures WHERE work_item = ?1",
    "INSERT INTO syndication_entries (work_item, reason, count) VALUES (?1, ?2, ?3)
     ON CONFLICT DO UPDATE SET count = count + excluded.count",
];

/// What a syndication feed lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedDocument {
    /// Each entry's link, as written in the feed, in document order.
    pub entries: Vec<String>,
}

/// The three syndication formats [`parse`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeedFormat {
    /// RSS 2.0 (`<rss><channel><item><link>`).
    Rss2,
    /// RSS 1.0 / RDF (`<rdf:RDF><item><link>` or `rdf:about`).
    Rss1,
    /// Atom 1.0 (`<feed><entry><link href="..."/>`).
    Atom1,
}

/// Reads the syndication feed `body` fetched at `url`, under `limits`: plain
/// XML or gzip-compressed, within [`MAX_BYTES`] uncompressed and
/// [`MAX_ENTRIES`] entries.
///
/// # Errors
///
/// Why it was not read: the response limit its decoding passed, or why it is
/// no readable RSS 2.0, RSS 1.0 or Atom 1.0 feed.
pub fn read_file(url: &str, body: &[u8], limits: Limits) -> Result<FeedDocument, Refusal> {
    let plain = decode_body(url, body, limits)?;
    parse(&plain).map_err(Refusal::Unreadable)
}

/// The fuzz target's body for syndication feeds (ADR 0067): reads `data` as a
/// feed, plain XML or gzip, under the catalog's response limits; a feed that
/// is read lists at most [`MAX_ENTRIES`] entries. Panics only on a bug.
pub fn fuzz_syndication(data: &[u8]) {
    if let Ok(document) = read_file("http://shop.test/feed.xml", data, Limits::default()) {
        assert!(
            document.entries.len() <= MAX_ENTRIES,
            "a feed read lists at most {} entries",
            grouped(MAX_ENTRIES)
        );
    }
}

/// Parses `body` as an RSS 2.0, RSS 1.0 (RDF) or Atom 1.0 feed, returning its
/// entry links in document order.
///
/// # Errors
///
/// Why it is no readable feed: broken XML, another root element, an undeclared
/// entity, or more than [`MAX_ENTRIES`] entries.
pub fn parse(body: &[u8]) -> Result<FeedDocument, Unreadable> {
    let unreadable = |error: &dyn std::fmt::Display| Unreadable(format!("its XML breaks: {error}"));
    let mut reader = Reader::from_reader(body);
    let mut state = FeedParser::default();
    loop {
        let event = reader.read_event().map_err(|error| unreadable(&error))?;
        match event {
            Event::Start(element) => state.on_start(&element)?,
            Event::Empty(element) => {
                if let Some(doc) = state.on_empty(&element)? {
                    return Ok(doc);
                }
            }
            Event::End(_) => state.on_end()?,
            Event::Text(text) => {
                state.push_text(text.decode().map_err(|e| unreadable(&e))?.as_ref());
            }
            Event::CData(data) => {
                state.push_text(data.decode().map_err(|e| unreadable(&e))?.as_ref());
            }
            Event::GeneralRef(reference) => state.on_ref(&reference)?,
            Event::Eof => break,
            _ => {}
        }
    }
    state.finish()
}

#[derive(Default)]
struct FeedParser {
    format: Option<FeedFormat>,
    entries: Vec<String>,
    depth: usize,
    in_channel: bool,
    in_item: bool,
    item_prefix: Option<Vec<u8>>,
    item_fallback_link: Option<String>,
    item_link: Option<String>,
    reading_link_text: Option<String>,
}

impl FeedParser {
    fn matches_item_prefix(&self, element: &BytesStart<'_>) -> bool {
        self.in_item && self.item_prefix == prefix_of(element)
    }

    fn on_start(&mut self, element: &BytesStart<'_>) -> Result<(), Unreadable> {
        self.depth += 1;
        let local = element.local_name();
        match (self.depth, self.format, local.as_ref()) {
            (1, _, b"rss") => self.format = Some(FeedFormat::Rss2),
            (1, _, b"RDF") => self.format = Some(FeedFormat::Rss1),
            (1, _, b"feed") => self.format = Some(FeedFormat::Atom1),
            (1, _, _) => return Err(no_feed()),
            (2, Some(FeedFormat::Rss2), b"channel") => self.in_channel = true,
            (3, Some(FeedFormat::Rss2), b"item") if self.in_channel => {
                self.begin_item(element, None);
            }
            (2, Some(FeedFormat::Rss1), b"item") => {
                let about = attr_local(element, b"about")?;
                self.begin_item(element, about);
            }
            (2, Some(FeedFormat::Atom1), b"entry") => {
                self.begin_item(element, None);
            }
            (4, Some(FeedFormat::Rss2), b"link") | (3, Some(FeedFormat::Rss1), b"link")
                if self.matches_item_prefix(element) && self.item_link.is_none() =>
            {
                self.reading_link_text = Some(String::new());
            }
            (3, Some(FeedFormat::Atom1), b"link")
                if self.matches_item_prefix(element) && self.item_link.is_none() =>
            {
                if let Some(href) = atom_alternate_href(element)? {
                    self.item_link = Some(href);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_item(&mut self, element: &BytesStart<'_>, fallback: Option<String>) {
        self.in_item = true;
        self.item_prefix = prefix_of(element);
        self.item_link = None;
        self.item_fallback_link = fallback;
    }

    fn on_empty(&mut self, element: &BytesStart<'_>) -> Result<Option<FeedDocument>, Unreadable> {
        if self.depth == 0 {
            return match element.local_name().as_ref() {
                b"rss" | b"RDF" | b"feed" => Ok(Some(FeedDocument {
                    entries: Vec::new(),
                })),
                _ => Err(no_feed()),
            };
        }
        if self.depth == 2
            && self.format == Some(FeedFormat::Atom1)
            && element.local_name().as_ref() == b"link"
            && self.matches_item_prefix(element)
            && self.item_link.is_none()
            && let Some(href) = atom_alternate_href(element)?
        {
            self.item_link = Some(href);
        }
        Ok(None)
    }

    fn on_end(&mut self) -> Result<(), Unreadable> {
        let link_depth = match self.format {
            Some(FeedFormat::Rss2) => 4,
            Some(FeedFormat::Rss1) => 3,
            _ => 0,
        };
        if self.depth == link_depth
            && let Some(text) = self.reading_link_text.take()
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                self.item_link = Some(trimmed.to_owned());
            }
        }
        let item_depth = match self.format {
            Some(FeedFormat::Rss2) => 3,
            Some(FeedFormat::Rss1 | FeedFormat::Atom1) => 2,
            None => 0,
        };
        if self.depth == item_depth && std::mem::take(&mut self.in_item) {
            self.item_prefix = None;
            if let Some(link) = self
                .item_link
                .take()
                .or_else(|| self.item_fallback_link.take())
            {
                push_entry(&mut self.entries, link)?;
            }
            self.item_fallback_link = None;
        }
        if self.depth == 2 && self.format == Some(FeedFormat::Rss2) {
            self.in_channel = false;
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(())
    }

    fn push_text(&mut self, text: &str) {
        if let Some(link) = self.reading_link_text.as_mut() {
            link.push_str(text);
        }
    }

    fn on_ref(&mut self, reference: &quick_xml::events::BytesRef<'_>) -> Result<(), Unreadable> {
        let unreadable =
            |error: &dyn std::fmt::Display| Unreadable(format!("its XML breaks: {error}"));
        if let Some(link) = self.reading_link_text.as_mut() {
            if let Some(character) = reference
                .resolve_char_ref()
                .map_err(|error| unreadable(&error))?
            {
                link.push(character);
            } else {
                let name = reference.decode().map_err(|error| unreadable(&error))?;
                let text = resolve_predefined_entity(&name).ok_or_else(|| {
                    Unreadable(format!("it uses the undeclared entity `&{name};`"))
                })?;
                link.push_str(text);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<FeedDocument, Unreadable> {
        if self.depth > 0 {
            return Err(Unreadable("its XML ends inside an element".to_owned()));
        }
        if self.format.is_none() {
            return Err(no_feed());
        }
        Ok(FeedDocument {
            entries: self.entries,
        })
    }
}

fn push_entry(entries: &mut Vec<String>, link: String) -> Result<(), Unreadable> {
    if entries.len() == MAX_ENTRIES {
        return Err(Unreadable(format!(
            "it lists more than {} entries",
            grouped(MAX_ENTRIES)
        )));
    }
    entries.push(link);
    Ok(())
}

fn no_feed() -> Unreadable {
    Unreadable("it is no syndication feed: its root is not `rss`, `RDF` or `feed`".to_owned())
}

fn prefix_of(element: &BytesStart<'_>) -> Option<Vec<u8>> {
    element
        .name()
        .prefix()
        .map(|prefix| prefix.as_ref().to_vec())
}

fn attr_local(element: &BytesStart<'_>, local: &[u8]) -> Result<Option<String>, Unreadable> {
    let unreadable = |error: &dyn std::fmt::Display| Unreadable(format!("its XML breaks: {error}"));
    for attr in element.attributes() {
        let attr = attr.map_err(|error| unreadable(&error))?;
        if attr.key.local_name().as_ref() == local {
            let value = attr
                .normalized_value(quick_xml::XmlVersion::default())
                .map_err(|error| unreadable(&error))?;
            let trimmed = value.trim();
            return Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()));
        }
    }
    Ok(None)
}

/// Extracts `href` from an Atom `<link>` element if its `rel` is absent, empty,
/// `"alternate"`, or the RFC 4287 alternate relation URI.
fn atom_alternate_href(element: &BytesStart<'_>) -> Result<Option<String>, Unreadable> {
    const ALTERNATE_URI: &str = "http://www.iana.org/assignments/relation/alternate";
    let unreadable = |error: &dyn std::fmt::Display| Unreadable(format!("its XML breaks: {error}"));
    let mut rel: Option<String> = None;
    let mut href: Option<String> = None;
    for attr in element.attributes() {
        let attr = attr.map_err(|error| unreadable(&error))?;
        match attr.key.local_name().as_ref() {
            b"rel" => {
                let value = attr
                    .normalized_value(quick_xml::XmlVersion::default())
                    .map_err(|error| unreadable(&error))?;
                rel = Some(value.trim().to_owned());
            }
            b"href" => {
                let value = attr
                    .normalized_value(quick_xml::XmlVersion::default())
                    .map_err(|error| unreadable(&error))?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    href = Some(trimmed.to_owned());
                }
            }
            _ => {}
        }
    }
    let is_alternate = rel
        .as_deref()
        .is_none_or(|r| r.is_empty() || r == "alternate" || r == ALTERNATE_URI);
    Ok(if is_alternate { href } else { None })
}

/// Admits the syndication seed `seed` into the run file `connection` holds,
/// under the Run's domain policy `policy`, through `add`, which adds its fetch
/// as a work item at its URL as parsed and answers its ID. An added seed
/// widens the scope by its URL's host, and `invariants` keeps its rules
/// compiled for the feed's read.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_seed(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    seed: &SyndicationSeed,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    admit_seed_in(
        connection,
        invariants,
        policy,
        seed,
        &mut Admitting::RunFile,
        add,
    )
}

/// Admits the syndication seed `seed` against what `admitting` adds to the run
/// file, mirroring [`crate::sitemap::admit_seed_in`].
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_seed_in(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    seed: &SyndicationSeed,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let url = match parse_url(&seed.syndication, None) {
        Ok(url) => url,
        Err(reason) => {
            return Ok(Enqueued::NotAdded {
                reason,
                url: seed.syndication.clone(),
            });
        }
    };
    let not_added = |admitting: &mut Admitting, reason: NotAddedReason| {
        admitting.refused(&url, reason.clone(), None);
        Enqueued::NotAdded {
            reason,
            url: seed.syndication.clone(),
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
        .prepare_cached("SELECT 1 FROM syndication_sources WHERE url = ?1")
        .and_then(|mut cached| cached.query_row([url.as_str()], |_| Ok(())))
        .optional()?
        .is_some()
        || admitting
            .earlier()
            .is_some_and(|staged| staged.holds_syndication(url.as_str()));
    if known {
        return Ok(not_added(admitting, NotAddedReason::Duplicate));
    }
    let id = add(&url)?;
    let rules = serde_json::to_string(&seed.rules).expect("invariant: rules encode as JSON");
    connection
        .prepare_cached(
            "INSERT INTO syndication_sources (work_item, url, rules, reads, failures)
             VALUES (?1, ?2, ?3, 0, 0)",
        )
        .and_then(|mut cached| cached.execute((&id, url.as_str(), &rules)))?;
    admitting.syndication_added(url.as_str());
    let mut holding = invariants.unwritten().map(|unwritten| unwritten.lock());
    widen(connection, invariants, &url, admitting, holding.as_mut())?;
    let _unreadable = invariants.know_sitemap_rules(&seed.rules);
    Ok(Enqueued::Added {
        id,
        url: url.into(),
    })
}

/// Whether the work item `id` is a syndication feed's fetch.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn is_fetch(connection: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM syndication_sources WHERE work_item = ?1")
        .and_then(|mut cached| cached.query_row([id], |_| Ok(())))
        .optional()?
        .is_some())
}

/// Admits the entries of `document`, read by work item `id`, as one links
/// batch through the Frontier admission within `invariants` and `policy`, with
/// 0 reads of the Run invariants from the run file and 0 rule compilations
/// (ADR 0140, F3).
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn read(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    id: &str,
    document: &FeedDocument,
    work: &CountedWork,
    mut add: impl FnMut(&Url, &str) -> rusqlite::Result<String>,
) -> rusqlite::Result<()> {
    let Some((url, given_rules)) = connection
        .prepare_cached(READ[0])
        .and_then(|mut cached| {
            cached.query_row([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
        })
        .optional()?
    else {
        return Ok(());
    };
    let base = Url::parse(&url).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let rules = invariants.sitemap_rules(&given_rules, work)?;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for given in &document.entries {
        let answer = admit_source_entry(
            connection,
            invariants,
            policy,
            &base,
            given,
            &rules,
            Candidate::SyndicationEntry {
                url: given,
                feed: &base,
            },
            &mut add,
        )?;
        let name = match answer {
            Enqueued::Added { .. } => ADDED.to_owned(),
            Enqueued::NotAdded { reason, .. } => reason_name(&reason),
        };
        *counts.entry(name).or_insert(0) += 1;
    }
    connection
        .prepare_cached(READ[1])
        .and_then(|mut cached| cached.execute([id]))?;
    connection
        .prepare_cached(READ[2])
        .and_then(|mut cached| cached.execute([id]))?;
    let mut insert = connection.prepare_cached(READ[3])?;
    for (reason, count) in counts {
        insert.execute((id, reason, i64::try_from(count).unwrap_or(i64::MAX)))?;
    }
    Ok(())
}

/// Records how a syndication source's fetch ended when it could not be read,
/// keeping its URL and why in `syndication_failures` so the Run report names
/// it while the other sources go on.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub fn ended(connection: &Connection, id: &str, ending: &Ending) -> rusqlite::Result<()> {
    let Some(url) = connection
        .prepare_cached("SELECT url FROM syndication_sources WHERE work_item = ?1")
        .and_then(|mut cached| cached.query_row([id], |row| row.get::<_, String>(0)))
        .optional()?
    else {
        return Ok(());
    };
    let (code, status, message) = match ending {
        Ending::Succeeded { .. } | Ending::Unchanged => return Ok(()),
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
            let name = match serde_json::to_value(reason) {
                Ok(serde_json::Value::String(name)) => name,
                _ => String::new(),
            };
            let message = message.clone().unwrap_or_else(|| {
                if *reason == SkipReason::RobotsTxt {
                    format!(
                        "The site's robots.txt disallows the syndication feed {url}, so it was not fetched."
                    )
                } else {
                    format!("The syndication feed {url} was skipped, with `{name}`.")
                }
            });
            (name, None, message)
        }
        Ending::Blocked { cause } => {
            let name = cause.as_str().to_owned();
            let message = format!("The site blocked the syndication feed {url}, with `{name}`.");
            (name, None, message)
        }
        unknown => {
            let name = unknown.kind().as_str().to_owned();
            let message = format!("The syndication feed {url} ended with `{name}`.");
            (name, None, message)
        }
    };
    connection
        .prepare_cached(
            "UPDATE syndication_sources SET failures = failures + 1 WHERE work_item = ?1",
        )
        .and_then(|mut cached| cached.execute([id]))?;
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO syndication_failures (work_item, url, code, status, message)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut cached| cached.execute((id, &url, &code, status, &message)))?;
    Ok(())
}

/// Each syndication source the crawl admitted, in the order it was added, with
/// its read count, failure count, latest failure if any, and entry counts
/// (ADR 0102, ADR 0139).
///
/// # Errors
///
/// When the run file cannot be read.
pub fn reports(connection: &Connection) -> rusqlite::Result<Vec<SourceReport>> {
    let has_table = connection
        .prepare_cached(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'syndication_sources'",
        )
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some();
    if !has_table {
        return Ok(Vec::new());
    }
    let count = |count: i64| u64::try_from(count).unwrap_or_default();
    let mut entries = connection.prepare_cached(
        "SELECT reason, count FROM syndication_entries WHERE work_item = ?1 ORDER BY reason",
    )?;
    let mut failure_stmt = connection.prepare_cached(
        "SELECT url, code, status, message FROM syndication_failures WHERE work_item = ?1",
    )?;
    let sources: Vec<(String, String, i64, i64)> = connection
        .prepare_cached(
            "SELECT work_item, url, reads, failures FROM syndication_sources ORDER BY rowid",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    sources
        .into_iter()
        .map(|(id, url, reads, failures)| {
            let failure = failure_stmt
                .query_row([&id], |row| {
                    Ok(SourceFailure {
                        url: row.get(0)?,
                        code: row.get(1)?,
                        status: row.get(2)?,
                        message: row.get(3)?,
                    })
                })
                .optional()?;
            let mut report = SourceReport {
                url,
                read: count(reads),
                failed: count(failures),
                failure,
                added: 0,
                not_added: 0,
                not_added_by_reason: Vec::new(),
            };
            for row in entries.query_map([&id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })? {
                let (name, n) = row?;
                if name == ADDED {
                    report.added = count(n);
                } else {
                    report.not_added += count(n);
                    report.not_added_by_reason.push(CountOf {
                        name,
                        count: count(n),
                    });
                }
            }
            Ok(report)
        })
        .collect()
}

/// Re-exports of [`MAX_BYTES`] and [`MAX_ENTRIES`] for syndication callers.
pub const MAX_FEED_BYTES: usize = MAX_BYTES;
/// Re-export of [`MAX_ENTRIES`] for syndication callers.
pub const MAX_FEED_ENTRIES: usize = MAX_ENTRIES;

#[cfg(test)]
mod tests;
