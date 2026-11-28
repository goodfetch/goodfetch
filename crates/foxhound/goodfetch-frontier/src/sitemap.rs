//! A crawl's sitemap seeds (ADRs 0093, 0102 and 0120; CO-10; pitfall U20):
//! a sitemap seed names a sitemap, or a site's robots.txt whose `Sitemap:`
//! lines name its sitemaps, and is admitted as a sitemap fetch, a work item
//! of the crawl that runs no handler, after the Run's domain policy, a
//! sitemap the crawl already reads being a `duplicate`. The seed widens
//! the crawl's scope by the host of the URL it names. Its fetch, and each
//! nested one, waits in its domain's queue and goes through Politeness and
//! robots.txt as a page does (the crawl's delivery); what it read is
//! [`read`] here: a sitemap index's sitemaps are admitted as fetches of
//! their own, each one level deeper than the sitemap that names it, the
//! seed's at level 0: under the domain policy; on the seed's host only,
//! but for the sitemaps a robots.txt seed's own `Sitemap:` lines
//! cross-submit on another host and those such a sitemap names on its
//! own host, which are read; no deeper than
//! `crawl.max_sitemap_nesting`; and counted against the crawl's bounds as a
//! page's fetch is, its level as its depth (ADR 0093's note of
//! 2026-11-10). A sitemap's entries,
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
//!
//! Size: one concern, the crawl's sitemap XML/gzip reader, its SQLite tables,
//! nesting and admission helpers (shared with `syndication` for source entries),
//! and per-seed reporting.

use std::collections::BTreeMap;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_engine::response_limits::{BodyFailure, Decoding, GZIP_MAGIC, LimitExceeded, Limits};
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::crawl::SitemapSeed;
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason, PathBudget};
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

use crate::admit::{Candidate, admit, widen};
use crate::bounds::{SoFar, count_admitted, count_refused, cut_by};
use crate::invariants::{Invariants, Rules};
use crate::staged::Admitting;
use crate::url::parse as parse_url;

/// The crawl's sitemaps, each the fetch of one work item, with the rules
/// of the seed it came from and its nesting level under it, the seed's
/// own being 0, and how many of each one's entries were added
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
            read INTEGER NOT NULL,
            nesting INTEGER NOT NULL,
            is_news INTEGER NOT NULL DEFAULT 0
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

/// A date read from a sitemap's `<lastmod>` or `<news:publication_date>`
/// (ADR 0139).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SitemapDate {
    /// Milliseconds since the Unix epoch (UTC midnight for date-only values).
    pub ms: u64,
    /// `true` when the source carried no time component (`YYYY`, `YYYY-MM`, or
    /// `YYYY-MM-DD`), so comparisons may treat it by whole UTC day.
    pub date_only: bool,
}

/// A sitemap `<changefreq>` hint (sitemaps.org; ADR 0139).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeFreq {
    /// Changes each time accessed.
    Always,
    /// Changes hourly.
    Hourly,
    /// Changes daily.
    Daily,
    /// Changes weekly.
    Weekly,
    /// Changes monthly.
    Monthly,
    /// Changes yearly.
    Yearly,
    /// Archived URL that does not change.
    Never,
}

impl ChangeFreq {
    /// Parses a `<changefreq>` text value (case-insensitive, trimmed).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "always" => Some(Self::Always),
            "hourly" => Some(Self::Hourly),
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "yearly" => Some(Self::Yearly),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// The initial revisit interval in milliseconds suggested by this hint
    /// when `revisit.site_hints.changefreq` is enabled (clamped to `[min, max]`
    /// by the URL's Revisit policy).
    #[must_use]
    pub const fn interval_ms(self) -> u64 {
        match self {
            Self::Always => 0,
            Self::Hourly => 3_600_000,
            Self::Daily => 86_400_000,
            Self::Weekly => 604_800_000,
            Self::Monthly => 2_592_000_000,
            Self::Yearly => 31_536_000_000,
            Self::Never => u64::MAX,
        }
    }
}

/// One `<url>` entry in a sitemap `urlset` (ADR 0093, ADR 0139).
#[derive(Clone, Debug, PartialEq)]
pub struct SitemapEntry {
    /// The entry's `<loc>`, as written.
    pub loc: String,
    /// The entry's `<lastmod>`, when present and valid W3C Datetime.
    pub lastmod: Option<SitemapDate>,
    /// The entry's `<news:publication_date>`, when present and valid W3C Datetime.
    pub news_publication_date: Option<SitemapDate>,
    /// The entry's `<changefreq>`, when present and recognized.
    pub changefreq: Option<ChangeFreq>,
    /// The entry's `<priority>` (`0.0..=1.0`), when present and valid.
    pub priority: Option<f32>,
}

impl SitemapEntry {
    /// Constructs an entry carrying only its `<loc>` and no dates or hints.
    #[must_use]
    pub fn loc(loc: impl Into<String>) -> Self {
        Self {
            loc: loc.into(),
            lastmod: None,
            news_publication_date: None,
            changefreq: None,
            priority: None,
        }
    }

    /// The entry's most authoritative date (`news:publication_date` first,
    /// falling back to `lastmod`).
    #[must_use]
    pub const fn effective_date(&self) -> Option<SitemapDate> {
        match self.news_publication_date {
            Some(date) => Some(date),
            None => self.lastmod,
        }
    }
}

/// One `<sitemap>` entry in a `sitemapindex` (or a `Sitemap:` line in
/// `robots.txt`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SitemapIndexEntry {
    /// The nested sitemap's `<loc>`, as written.
    pub loc: String,
    /// The nested sitemap's `<lastmod>`, when present and valid W3C Datetime.
    pub lastmod: Option<SitemapDate>,
}

impl SitemapIndexEntry {
    /// Constructs an index entry carrying only its `<loc>` and no `<lastmod>`.
    #[must_use]
    pub fn loc(loc: impl Into<String>) -> Self {
        Self {
            loc: loc.into(),
            lastmod: None,
        }
    }
}

/// What a sitemap lists.
#[derive(Clone, Debug, PartialEq)]
pub enum Document {
    /// A `urlset`'s entries, and whether the file declared or used the Google
    /// News sitemap extension (`news:`).
    Entries {
        /// Parsed `<url>` entries in document order.
        entries: Vec<SitemapEntry>,
        /// `true` when the `urlset` declared the sitemap-news namespace or any
        /// entry contained a `news` element.
        is_news: bool,
    },
    /// A sitemap index's sitemaps, or those a robots.txt's `Sitemap:`
    /// lines name, as written.
    Sitemaps(Vec<SitemapIndexEntry>),
}

/// Options governing how [`read`] handles dates and standing re-reads
/// (ADR 0139).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOptions {
    /// `crawl.sitemap.read_lastmod` (`true` by default): when `false`,
    /// `<lastmod>` and `<news:publication_date>` are ignored.
    pub read_lastmod: bool,
    /// `(now_ms, max_entry_age_ms)` in a Continuous crawl when
    /// `crawl.continuous.max_entry_age` is set: entries older than
    /// `now_ms - max_entry_age_ms` are refused with `max_entry_age`.
    pub max_entry_age: Option<(u64, u64)>,
    /// Wall-clock milliseconds of this sitemap's previous read, if this is a
    /// standing re-read in a Continuous crawl.
    pub previous_fetched_at: Option<u64>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            read_lastmod: true,
            max_entry_age: None,
            previous_fetched_at: None,
        }
    }
}

/// Summary of one [`read`] call, used by Continuous crawls to schedule the
/// sitemap's next standing re-read (ADR 0139).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOutcome {
    /// `true` when the read added at least one new entry or nested sitemap, or
    /// saw at least one entry dated strictly after `previous_fetched_at`.
    pub changed: bool,
    /// `true` when this sitemap is a Google News sitemap (now or on an earlier
    /// read).
    pub is_news: bool,
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
    let plain = decode_body(url, body, limits)?;
    parse(&plain).map_err(Refusal::Unreadable)
}

/// Decodes a plain or gzip-compressed XML source file at `url` under `limits`,
/// refusing files larger than [`MAX_BYTES`] uncompressed.
pub(crate) fn decode_body<'a>(
    url: &str,
    body: &'a [u8],
    limits: Limits,
) -> Result<std::borrow::Cow<'a, [u8]>, Refusal> {
    let plain = if body.starts_with(&GZIP_MAGIC) {
        let past = u64::try_from(MAX_BYTES + 1).expect("invariant: 50 MB fits in u64");
        let capped = Limits {
            max_body_size: limits.max_body_size.min(past),
            ..limits
        };
        let mut decoding = Decoding::new(url, std::iter::once("gzip"), capped);
        let decoded = decoding
            .push(body)
            .and_then(|()| decoding.finish())
            .map_err(|failure| match failure {
                BodyFailure::Limit(exceeded)
                    if exceeded.setting == LimitSetting::MaxBodySize && exceeded.limit == past =>
                {
                    Refusal::Unreadable(too_large())
                }
                BodyFailure::Limit(exceeded) => Refusal::Limit(exceeded),
                BodyFailure::Corrupt | BodyFailure::Cut => {
                    Refusal::Unreadable(Unreadable("its gzip stream does not decode".to_owned()))
                }
            })?;
        std::borrow::Cow::Owned(decoded)
    } else {
        std::borrow::Cow::Borrowed(body)
    };
    if plain.len() > MAX_BYTES {
        return Err(Refusal::Unreadable(too_large()));
    }
    Ok(plain)
}

/// The fuzz target's body for sitemaps (ADR 0067): reads `data` as a
/// sitemap file, plain XML or gzip, under the catalog's response limits; a
/// file that is read lists at most sitemaps.org's 50,000 URLs. It panics
/// only on a bug.
pub fn fuzz_sitemap(data: &[u8]) {
    if let Ok(doc) = read_file("http://shop.test/sitemap.xml", data, Limits::default()) {
        let len = match &doc {
            Document::Entries { entries, .. } => entries.len(),
            Document::Sitemaps(sitemaps) => sitemaps.len(),
        };
        assert!(
            len <= MAX_ENTRIES,
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
pub(crate) fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Reads the sitemap `body`: a `urlset`'s entries (with `<lastmod>`, `news:`,
/// `<changefreq>` and `<priority>`) or a `sitemapindex`'s sitemaps.
///
/// # Errors
///
/// Why it is no sitemap: no XML, XML that breaks off, another root, or
/// more than sitemaps.org's 50,000 URLs.
pub fn parse(body: &[u8]) -> Result<Document, Unreadable> {
    let unreadable = |error: &dyn std::fmt::Display| Unreadable(format!("its XML breaks: {error}"));
    let mut reader = Reader::from_reader(body);
    let mut state = SitemapParser::default();
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
                state.push_text(&text.decode().map_err(|error| unreadable(&error))?);
            }
            Event::CData(data) => {
                state.push_text(&data.decode().map_err(|error| unreadable(&error))?);
            }
            Event::GeneralRef(reference) => {
                if state.capture.is_some() {
                    if let Some(character) = reference
                        .resolve_char_ref()
                        .map_err(|error| unreadable(&error))?
                    {
                        state.push_char(character);
                    } else {
                        let name = reference.decode().map_err(|error| unreadable(&error))?;
                        let text = resolve_predefined_entity(&name).ok_or_else(|| {
                            Unreadable(format!("it uses the undeclared entity `&{name};`"))
                        })?;
                        state.push_text(text);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    state.finish()
}

/// Which text field of the open `<url>` or `<sitemap>` element is being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureField {
    Loc,
    Lastmod,
    ChangeFreq,
    Priority,
    NewsPublicationDate,
}

#[derive(Default)]
struct PendingEntry {
    loc: Option<String>,
    lastmod: Option<SitemapDate>,
    news_publication_date: Option<SitemapDate>,
    changefreq: Option<ChangeFreq>,
    priority: Option<f32>,
}

#[derive(Default)]
enum OpenParent {
    #[default]
    None,
    Open(Option<Vec<u8>>),
}

impl OpenParent {
    const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    fn matches(&self, prefix: Option<&[u8]>) -> bool {
        matches!(self, Self::Open(open) if open.as_deref() == prefix)
    }

    fn take(&mut self) -> bool {
        matches!(std::mem::take(self), Self::Open(_))
    }
}

#[derive(Default)]
struct SitemapParser {
    /// `Some(false)` for `urlset`, `Some(true)` for `sitemapindex`.
    root: Option<bool>,
    is_news: bool,
    entries: Vec<SitemapEntry>,
    sitemaps: Vec<SitemapIndexEntry>,
    /// Prefix of the open `<url>` or `<sitemap>` element.
    parent: OpenParent,
    /// Open entry being populated until `</url>` or `</sitemap>`.
    pending: PendingEntry,
    /// Whether we are inside a `<news:news>` element at depth 3.
    in_news: bool,
    /// Currently captured text field and its buffer.
    capture: Option<(CaptureField, usize, String)>,
    depth: usize,
}

impl SitemapParser {
    fn on_start(&mut self, element: &BytesStart<'_>) -> Result<(), Unreadable> {
        self.depth += 1;
        let name = element.local_name();
        match (self.depth, name.as_ref()) {
            (1, b"urlset") => {
                self.root = Some(false);
                if declares_news_namespace(element) {
                    self.is_news = true;
                }
            }
            (1, b"sitemapindex") => self.root = Some(true),
            (1, _) => return Err(no_sitemap()),
            (2, child) => {
                let listed: &[u8] = if self.root == Some(true) {
                    b"sitemap"
                } else {
                    b"url"
                };
                if child == listed {
                    self.parent = OpenParent::Open(prefix_of(element));
                    self.pending = PendingEntry::default();
                    self.in_news = false;
                } else {
                    self.parent = OpenParent::None;
                }
            }
            (3, tag) if self.parent.is_open() => {
                let same_prefix = self.parent.matches(
                    element
                        .name()
                        .prefix()
                        .map(quick_xml::name::Prefix::into_inner),
                );
                if same_prefix && tag == b"loc" {
                    self.capture = Some((CaptureField::Loc, 3, String::new()));
                } else if same_prefix && tag == b"lastmod" {
                    self.capture = Some((CaptureField::Lastmod, 3, String::new()));
                } else if same_prefix && self.root == Some(false) && tag == b"changefreq" {
                    self.capture = Some((CaptureField::ChangeFreq, 3, String::new()));
                } else if same_prefix && self.root == Some(false) && tag == b"priority" {
                    self.capture = Some((CaptureField::Priority, 3, String::new()));
                } else if self.root == Some(false) && tag == b"news" {
                    self.is_news = true;
                    self.in_news = true;
                }
            }
            (4, b"publication_date") if self.in_news => {
                self.capture = Some((CaptureField::NewsPublicationDate, 4, String::new()));
            }
            _ => {}
        }
        Ok(())
    }

    fn on_empty(&mut self, element: &BytesStart<'_>) -> Result<Option<Document>, Unreadable> {
        if self.depth == 0 {
            return match element.local_name().as_ref() {
                b"urlset" => Ok(Some(Document::Entries {
                    entries: Vec::new(),
                    is_news: declares_news_namespace(element),
                })),
                b"sitemapindex" => Ok(Some(Document::Sitemaps(Vec::new()))),
                _ => Err(no_sitemap()),
            };
        }
        if self.depth == 2 && self.root == Some(false) && element.local_name().as_ref() == b"news" {
            self.is_news = true;
        }
        Ok(None)
    }

    fn on_end(&mut self) -> Result<(), Unreadable> {
        if let Some((field, at_depth, text)) = self.capture.take() {
            if self.depth == at_depth {
                self.apply_field(field, text.trim());
            } else {
                self.capture = Some((field, at_depth, text));
            }
        }
        if self.depth == 3 && self.in_news {
            self.in_news = false;
        }
        if self.depth == 2 && self.parent.take() {
            let pending = std::mem::take(&mut self.pending);
            if let Some(loc) = pending.loc {
                let count = if self.root == Some(true) {
                    self.sitemaps.len()
                } else {
                    self.entries.len()
                };
                if count == MAX_ENTRIES {
                    return Err(Unreadable(format!(
                        "it lists more than sitemaps.org's {} URLs",
                        grouped(MAX_ENTRIES)
                    )));
                }
                if self.root == Some(true) {
                    self.sitemaps.push(SitemapIndexEntry {
                        loc,
                        lastmod: pending.lastmod,
                    });
                } else {
                    self.entries.push(SitemapEntry {
                        loc,
                        lastmod: pending.lastmod,
                        news_publication_date: pending.news_publication_date,
                        changefreq: pending.changefreq,
                        priority: pending.priority,
                    });
                }
            }
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(())
    }

    fn apply_field(&mut self, field: CaptureField, text: &str) {
        match field {
            CaptureField::Loc => {
                if self.pending.loc.is_none() {
                    self.pending.loc = Some(text.to_owned());
                }
            }
            CaptureField::Lastmod => {
                if self.pending.lastmod.is_none() {
                    self.pending.lastmod = parse_w3c_datetime(text);
                }
            }
            CaptureField::ChangeFreq => {
                if self.pending.changefreq.is_none() {
                    self.pending.changefreq = ChangeFreq::parse(text);
                }
            }
            CaptureField::Priority => {
                if self.pending.priority.is_none() {
                    self.pending.priority = text
                        .parse::<f32>()
                        .ok()
                        .filter(|p| p.is_finite() && (0.0..=1.0).contains(p));
                }
            }
            CaptureField::NewsPublicationDate => {
                if self.pending.news_publication_date.is_none() {
                    self.pending.news_publication_date = parse_w3c_datetime(text);
                }
            }
        }
    }

    fn push_text(&mut self, s: &str) {
        if let Some((_, _, buf)) = &mut self.capture {
            buf.push_str(s);
        }
    }

    fn push_char(&mut self, ch: char) {
        if let Some((_, _, buf)) = &mut self.capture {
            buf.push(ch);
        }
    }

    fn finish(self) -> Result<Document, Unreadable> {
        if self.depth > 0 {
            return Err(Unreadable("its XML ends inside an element".to_owned()));
        }
        match self.root {
            Some(false) => Ok(Document::Entries {
                entries: self.entries,
                is_news: self.is_news,
            }),
            Some(true) => Ok(Document::Sitemaps(self.sitemaps)),
            None => Err(no_sitemap()),
        }
    }
}

/// Whether `element` declares a Google News sitemap XML namespace
/// (`http://www.google.com/schemas/sitemap-news/0.9`).
fn declares_news_namespace(element: &BytesStart<'_>) -> bool {
    element.attributes().flatten().any(|attr| {
        let key = attr.key.as_ref();
        (key == b"xmlns" || key.starts_with(b"xmlns:"))
            && std::str::from_utf8(attr.value.as_ref())
                .is_ok_and(|value| value.contains("sitemap-news"))
    })
}

/// Parses a W3C Datetime / ISO 8601 date or timestamp (`YYYY`, `YYYY-MM`,
/// `YYYY-MM-DD`, or `YYYY-MM-DDThh:mm[:ss[.fff]][Z|±hh:mm]`) into a
/// [`SitemapDate`]. Returns `None` for malformed or pre-1970 values.
#[must_use]
pub fn parse_w3c_datetime(raw: &str) -> Option<SitemapDate> {
    let s = raw.trim();
    if s.len() < 4 || !s.is_ascii() {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    if s.len() == 4 {
        let days = days_from_civil(year, 1, 1)?;
        return Some(SitemapDate {
            ms: u64::try_from(days).ok()?.checked_mul(86_400_000)?,
            date_only: true,
        });
    }
    if s.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let month: u32 = s.get(5..7)?.parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    if s.len() == 7 {
        let days = days_from_civil(year, month, 1)?;
        return Some(SitemapDate {
            ms: u64::try_from(days).ok()?.checked_mul(86_400_000)?,
            date_only: true,
        });
    }
    if s.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let day: u32 = s.get(8..10)?.parse().ok()?;
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    if s.len() == 10 {
        return Some(SitemapDate {
            ms: u64::try_from(days).ok()?.checked_mul(86_400_000)?,
            date_only: true,
        });
    }
    let sep = *s.as_bytes().get(10)?;
    if sep != b'T' && sep != b't' && sep != b' ' {
        return None;
    }
    let time_part = s.get(11..)?;
    let (hour, minute, second, frac_ms, tz_offset_secs) = parse_time_and_tz(time_part)?;
    let total_secs = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))?
        .checked_sub(tz_offset_secs)?;
    let ms = u64::try_from(total_secs)
        .ok()?
        .checked_mul(1_000)?
        .checked_add(u64::from(frac_ms))?;
    Some(SitemapDate {
        ms,
        date_only: false,
    })
}

fn parse_time_and_tz(s: &str) -> Option<(u32, u32, u32, u32, i64)> {
    let hour: u32 = s.get(0..2)?.parse().ok()?;
    if s.as_bytes().get(2) != Some(&b':') {
        return None;
    }
    let minute: u32 = s.get(3..5)?.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    let mut rest = s.get(5..)?;
    let mut second = 0_u32;
    let mut frac_ms = 0_u32;
    if let Some(after_colon) = rest.strip_prefix(':') {
        second = after_colon.get(0..2)?.parse().ok()?;
        if second > 60 {
            return None;
        }
        rest = after_colon.get(2..)?;
        if let Some(after_dot) = rest.strip_prefix('.') {
            let digits_end = after_dot
                .bytes()
                .position(|b| !b.is_ascii_digit())
                .unwrap_or(after_dot.len());
            if digits_end == 0 {
                return None;
            }
            let frac_str = &after_dot[..digits_end];
            frac_ms = parse_millis_fraction(frac_str);
            rest = &after_dot[digits_end..];
        }
    }
    let tz_offset_secs = parse_tz_offset(rest)?;
    Some((hour, minute, second.min(59), frac_ms, tz_offset_secs))
}

fn parse_millis_fraction(digits: &str) -> u32 {
    let mut ms = 0_u32;
    let mut scale = 100_u32;
    for byte in digits.bytes().take(3) {
        ms += u32::from(byte - b'0') * scale;
        scale /= 10;
    }
    ms
}

fn parse_tz_offset(rest: &str) -> Option<i64> {
    if rest.is_empty() || rest == "Z" || rest == "z" {
        return Some(0);
    }
    let sign = match rest.as_bytes().first()? {
        b'+' => 1_i64,
        b'-' => -1_i64,
        _ => return None,
    };
    let offset = &rest[1..];
    let (h_str, m_str) = if let Some((h, m)) = offset.split_once(':') {
        (h, m)
    } else if offset.len() == 4 {
        (&offset[..2], &offset[2..])
    } else {
        return None;
    };
    if h_str.len() != 2 || m_str.len() != 2 {
        return None;
    }
    let hours: i64 = h_str.parse().ok()?;
    let mins: i64 = m_str.parse().ok()?;
    if hours > 23 || mins > 59 {
        return None;
    }
    Some(sign * (hours * 3_600 + mins * 60))
}

/// Days since 1970-01-01 (Howard Hinnant's civil calendar algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days >= 0).then_some(days)
}

const fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
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
/// seed widens the scope by its URL's host, and the Run's `invariants`
/// keep its rules compiled for the reads of its sitemaps.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_seed(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    seed: &SitemapSeed,
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
    invariants: &Invariants,
    policy: &Policy,
    seed: &SitemapSeed,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let rules = serde_json::to_string(&seed.rules).expect("invariant: rules encode as JSON");
    let fetch = Fetch {
        given: &seed.sitemap,
        base: None,
        nested: None,
        rules: &rules,
    };
    let answer = admit_fetch(connection, policy, fetch, admitting, add)?;
    if let Enqueued::Added { url, .. } = &answer
        && let Ok(url) = Url::parse(url)
    {
        let mut holding = invariants.unwritten().map(|unwritten| unwritten.lock());
        widen(connection, invariants, &url, admitting, holding.as_mut())?;
        // The seed's rules were checked as it was given.
        let _unreadable = invariants.know_sitemap_rules(&seed.rules);
    }
    Ok(answer)
}

/// A sitemap's fetch to admit: the sitemap `given`, resolved against
/// `base` when given, `nested` under a seed (not for the seed itself),
/// with `rules`.
#[derive(Clone, Copy)]
struct Fetch<'a> {
    given: &'a str,
    base: Option<&'a Url>,
    nested: Option<Nested<'a>>,
    rules: &'a str,
}

/// How a sitemap is nested under its seed, and what bounds its fetch.
#[derive(Clone, Copy)]
struct Nested<'a> {
    /// The seed's work item.
    seed: &'a str,
    /// The hosts it may be on: its seed's and that of the sitemap naming
    /// it, which differ only beneath a sitemap a robots.txt seed
    /// cross-submitted; or `None` for the sitemaps a robots.txt seed's
    /// own lines name, which may cross-submit one on any host.
    hosts: Option<[Option<&'a str>; 2]>,
    /// Its nesting level: one more than that of the sitemap naming it.
    level: u64,
    /// The Run's invariants, its bounds among them.
    invariants: &'a Invariants,
}

impl Nested<'_> {
    /// Why the fetch of the nested sitemap at `url` is not added, if it is
    /// not: its host is neither the seed's nor its index's, it is nested past
    /// `crawl.max_sitemap_nesting`, or a bound of the crawl refuses it as
    /// it refuses a page at its level's depth.
    fn refuses(
        &self,
        url: &Url,
        budget: Option<&PathBudget>,
        so_far: &mut SoFar<'_, '_>,
    ) -> rusqlite::Result<Option<NotAddedReason>> {
        if self
            .hosts
            .is_some_and(|hosts| !hosts.contains(&url.host_str()))
        {
            return Ok(Some(NotAddedReason::OutOfScope));
        }
        let bounds = self.invariants.bounds();
        if bounds
            .max_sitemap_nesting
            .is_some_and(|max| self.level > max)
        {
            return Ok(Some(NotAddedReason::MaxSitemapNesting));
        }
        bounds.refuses(url, self.level, budget, so_far)
    }
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
        nested,
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
        admitting.refused(&url, reason.clone(), None);
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
    let budget = nested.and_then(|nested| nested.invariants.budget_of(&url));
    // Held from the bounds' check through the count, as a page's
    // admission holds it (#2169).
    let mut holding = nested
        .and_then(|nested| nested.invariants.unwritten())
        .map(|unwritten| unwritten.lock());
    if let Some(nested) = nested {
        let mut so_far = SoFar {
            connection,
            holding: holding.as_mut(),
            admitting,
            learning: nested.invariants.bounds().learning,
        };
        if let Some(reason) = nested.refuses(&url, budget, &mut so_far)? {
            count_refused(connection, &reason, budget)?;
            return Ok(not_added(admitting, reason));
        }
    }
    let id = add(&url)?;
    if nested.is_some() {
        count_admitted(connection, budget)?;
        if let Some(holding) = &mut holding
            && admitting.writes()
        {
            holding.counted(budget.map(|budget| budget.pattern.as_str()));
        }
    }
    drop(holding);
    let (seed, level) = nested.map_or((id.as_str(), 0), |nested| (nested.seed, nested.level));
    connection
        .prepare_cached(
            "INSERT INTO sitemaps (work_item, url, seed, rules, read, nesting)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
        )
        .and_then(|mut cached| {
            cached.execute((
                &id,
                url.as_str(),
                seed,
                rules,
                i64::try_from(level).unwrap_or(i64::MAX),
            ))
        })?;
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

/// What a sitemap's read adds: an entry, with its Label and optional sitemap
/// hints, or the fetch of a sitemap it nests.
#[derive(Clone, Copy, Debug)]
pub enum Adding<'a> {
    /// An entry, a crawl request of this Label with no Payload.
    Entry {
        /// The matched Label name.
        label: &'a str,
        /// Optional `<changefreq>` hint from the sitemap entry.
        changefreq: Option<ChangeFreq>,
        /// Optional `<priority>` hint from the sitemap entry.
        priority: Option<f32>,
    },
    /// A nested sitemap's fetch.
    Sitemap,
}

/// The statements [`read`] runs, which [`crate::admission_statements`]
/// has the Run prepare as it starts: the sitemap's row, its read, the removal of
/// its failure and its entries' counts.
pub(crate) const READ: [&str; 4] = [
    "SELECT sitemaps.url, sitemaps.seed, sitemaps.rules, sitemaps.nesting, seeds.url, sitemaps.is_news
     FROM sitemaps JOIN sitemaps AS seeds ON seeds.work_item = sitemaps.seed
     WHERE sitemaps.work_item = ?1",
    "UPDATE sitemaps SET read = 1, is_news = is_news | ?2 WHERE work_item = ?1",
    "DELETE FROM sitemap_failures WHERE url = ?1",
    "INSERT INTO sitemap_entries (work_item, reason, count) VALUES (?1, ?2, ?3)
     ON CONFLICT DO UPDATE SET count = count + excluded.count",
];

/// Reads `document`, what the fetch of the work item `id` gave, into the
/// run file `connection` holds, within the Run's `invariants` and under its
/// domain policy `policy`, through `add`, which adds a work item at its URL as
/// parsed and answers its ID; and keeps how many of its entries were added
/// and why the others were not. A work item that is no sitemap's fetch
/// reads nothing.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
#[expect(
    clippy::too_many_arguments,
    reason = "sitemap read takes connection, invariants, policy, id, document, options, work and adder"
)]
pub fn read(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    id: &str,
    document: &Document,
    options: ReadOptions,
    work: &CountedWork,
    mut add: impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<ReadOutcome> {
    let Some((url, seed, given_rules, nesting, seed_url, prev_is_news)) = connection
        .prepare_cached(READ[0])
        .and_then(|mut cached| {
            cached.query_row([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)? != 0,
                ))
            })
        })
        .optional()?
    else {
        return Ok(ReadOutcome::default());
    };
    let stored = |url: &str| {
        Url::parse(url).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };
    let base = stored(&url)?;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut is_news = prev_is_news;
    let changed = match document {
        Document::Sitemaps(sitemaps) => {
            let seed_url = stored(&seed_url)?;
            let cross_submits = id == seed && is_robots_txt(&base);
            let nested = Nested {
                seed: &seed,
                hosts: (!cross_submits).then(|| [seed_url.host_str(), base.host_str()]),
                level: u64::try_from(nesting).unwrap_or(u64::MAX).saturating_add(1),
                invariants,
            };
            read_nested_sitemaps(
                connection,
                policy,
                &base,
                &seed,
                &given_rules,
                nested,
                sitemaps,
                options,
                &mut add,
            )?
        }
        Document::Entries {
            entries,
            is_news: doc_is_news,
        } => {
            is_news |= *doc_is_news;
            let rules = invariants.sitemap_rules(&given_rules, work)?;
            read_urlset_entries(
                connection,
                invariants,
                policy,
                &base,
                entries,
                &rules,
                options,
                &mut counts,
                &mut add,
            )?
        }
    };
    connection
        .prepare_cached(READ[1])
        .and_then(|mut cached| cached.execute((id, i64::from(is_news))))?;
    // Read at last, after `run.retry_failed` re-admitted it.
    connection
        .prepare_cached(READ[2])
        .and_then(|mut cached| cached.execute([&url]))?;
    let mut insert = connection.prepare_cached(READ[3])?;
    for (reason, count) in counts {
        insert.execute((id, reason, i64::try_from(count).unwrap_or(i64::MAX)))?;
    }
    Ok(ReadOutcome { changed, is_news })
}

#[expect(clippy::too_many_arguments, reason = "helper for sitemap index reads")]
fn read_nested_sitemaps(
    connection: &Connection,
    policy: &Policy,
    base: &Url,
    seed: &str,
    given_rules: &str,
    nested: Nested<'_>,
    sitemaps: &[SitemapIndexEntry],
    options: ReadOptions,
    add: &mut impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<bool> {
    let mut changed = false;
    for entry in sitemaps {
        if options.read_lastmod
            && let Some(prev) = options.previous_fetched_at
            && let Some(date) = entry.lastmod
            && date.ms > prev
        {
            changed = true;
        }
        let fetch = Fetch {
            given: &entry.loc,
            base: Some(base),
            nested: Some(nested),
            rules: given_rules,
        };
        let answer = admit_fetch(connection, policy, fetch, &mut Admitting::RunFile, |url| {
            add(url, Adding::Sitemap)
        })?;
        if matches!(answer, Enqueued::Added { .. }) {
            changed = true;
        }
        keep_refused_nesting(connection, seed, base, answer)?;
    }
    Ok(changed)
}

#[expect(clippy::too_many_arguments, reason = "helper for sitemap urlset reads")]
fn read_urlset_entries(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    base: &Url,
    entries: &[SitemapEntry],
    rules: &Rules,
    options: ReadOptions,
    counts: &mut BTreeMap<String, u64>,
    add: &mut impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<bool> {
    let mut changed = false;
    let mut count = |answer: &Enqueued| {
        let name = match answer {
            Enqueued::Added { .. } => ADDED.to_owned(),
            Enqueued::NotAdded { reason, .. } => reason_name(reason),
        };
        *counts.entry(name).or_insert(0) += 1;
    };
    for entry in entries {
        if options.read_lastmod
            && let Some(date) = entry.effective_date()
        {
            if let Some((now_ms, max_age_ms)) = options.max_entry_age
                && now_ms.saturating_sub(date.ms) > max_age_ms
            {
                count(&Enqueued::NotAdded {
                    reason: NotAddedReason::MaxEntryAge,
                    url: entry.loc.clone(),
                });
                continue;
            }
            if let Some(prev) = options.previous_fetched_at
                && date.ms > prev
            {
                changed = true;
            }
        }
        let answer = admit_entry(connection, invariants, policy, base, entry, rules, add)?;
        if matches!(answer, Enqueued::Added { .. }) {
            changed = true;
        }
        count(&answer);
    }
    Ok(changed)
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
    let message = match (&reason, cut_by(&reason)) {
        (NotAddedReason::DomainPolicy, _) => format!(
            "The Run's domain policy refuses the host of the sitemap {url}, so it was not fetched."
        ),
        (NotAddedReason::OutOfScope, _) => format!(
            "The sitemap {url} is on another host than its sitemap seed's and the sitemap's that names it, where only a robots.txt seed's own `Sitemap:` lines name one that is fetched, so it was not fetched."
        ),
        (NotAddedReason::MaxSitemapNesting, _) => format!(
            "The sitemap {url} is nested more levels under its sitemap seed than `crawl.max_sitemap_nesting`, so it was not fetched."
        ),
        (NotAddedReason::PathBudget, _) => format!(
            "A path budget of `crawl.path_budgets` refuses the sitemap {url}, so it was not fetched."
        ),
        (_, Some(setting)) => format!(
            "`{}` refuses the sitemap {url}, so it was not fetched.",
            setting.name
        ),
        (_, None) => format!("The sitemap {url} names no URL the crawl fetches."),
    };
    keep_failure(
        connection,
        seed,
        &SitemapFailure {
            url,
            code: reason_name(&reason),
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
        unknown => {
            let name = unknown.kind().as_str().to_owned();
            let message = format!("The sitemap {url} ended with `{name}`.");
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

/// Admits the entry `given` of a sitemap or syndication source at `base` as a
/// crawl request of its first matching rule's Label, or answers `no_rule`.
#[expect(
    clippy::too_many_arguments,
    reason = "shared entry admission helper between sitemap and syndication readers"
)]
pub(crate) fn admit_source_entry(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    base: &Url,
    given: &str,
    rules: &Rules,
    candidate: Candidate<'_>,
    add: &mut impl FnMut(&Url, &str) -> rusqlite::Result<String>,
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
    admit(connection, invariants, policy, candidate, |url| {
        add(url, label)
    })
}

/// Admits the entry `entry` of the sitemap at `base` as a crawl request
/// of its first matching rule's Label, or answers `no_rule`.
fn admit_entry(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    base: &Url,
    entry: &SitemapEntry,
    rules: &Rules,
    add: &mut impl FnMut(&Url, Adding<'_>) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    admit_source_entry(
        connection,
        invariants,
        policy,
        base,
        &entry.loc,
        rules,
        Candidate::SitemapEntry {
            url: &entry.loc,
            sitemap: base,
        },
        &mut |url, label| {
            add(
                url,
                Adding::Entry {
                    label,
                    changefreq: entry.changefreq,
                    priority: entry.priority,
                },
            )
        },
    )
}

/// The wire name of `reason`.
pub(crate) fn reason_name(reason: &NotAddedReason) -> String {
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
