//! robots.txt's file (RFC 9309; ADRs 0067, 0086 and 0098): [`parse`], a pure
//! function of a file's bytes that answers its groups, each with its
//! `User-agent` lines, its rules and its Crawl-delay, and the file's
//! Sitemap lines; [`judge`], whether a set of rules lets a crawler request
//! a URL's [`target`]; [`matches`], whether one rule's pattern matches a
//! target; and [`location`], the robots.txt of a URL's origin.
//!
//! The parser reads the first [`MAX_SIZE`] bytes, after a UTF-8 byte order
//! mark, as lines ended by LF, CRLF or CR, each `key: value` with its key
//! in any case, a `#` starting a comment. A run of `User-agent` lines opens
//! a group, and its rules and Crawl-delay belong to it until the next
//! `User-agent` line after them, as do its `Content-Signal` lines
//! (Cloudflare's Content Signals Policy); a rule before any `User-agent` line, an
//! unknown key and a line with no colon are ignored, and a Sitemap line
//! belongs to the file. Matching follows RFC 9309 §2.2.2: a rule matches a
//! target it is a prefix of, `*` matching any octets and a final `$` the
//! target's end; the longest matching rule wins, an Allow winning a tie;
//! `/robots.txt` itself is always allowed. Rules and targets are compared
//! as percent-encoded octets: a percent-encoded unreserved character is
//! decoded, any other octet outside US-ASCII encoded, the hex digits made
//! capitals, and a target's `*` and `$` encoded, so `%2A` and `%24` in a
//! rule match them.
//!
//! Which groups a crawler obeys, by its product token, is area 5's
//! (`goodfetch-page`'s `robots_txt`), Foxhound's gate and the reading of a
//! Collie program's own navigation calling it over these groups.
//! [`RunCopies`] is that reading's per-Run copy, one fetch per origin per
//! Run (ADR 0098), kept in the Run's run state with a bounded cache in
//! front; a reading that never reached the site ([`NotRead`]) is kept
//! nowhere. Foxhound's shared copy is no part of it.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension as _;
use tokio::sync::OnceCell;
use url::Url;

use crate::run_state::{RunState, RunStateTables};

#[cfg(test)]
mod tests;

/// How much of a file is read: RFC 9309 §2.5's least, 500 KiB.
pub const MAX_SIZE: usize = 500 * 1024;

/// A robots.txt file, as [`parse`] reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RobotsTxt {
    groups: Vec<Group>,
    sitemaps: Vec<String>,
}

impl RobotsTxt {
    /// Its groups, in file order.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The values of its Sitemap lines, in file order.
    pub fn sitemaps(&self) -> &[String] {
        &self.sitemaps
    }
}

/// One group of a file: its `User-agent` lines' values, as written, its
/// rules in file order, and its Crawl-delay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    user_agents: Vec<String>,
    rules: Vec<Rule>,
    crawl_delay: Option<Duration>,
    content_signals: Vec<(String, bool)>,
    line: usize,
}

impl Group {
    /// The values of its `User-agent` lines.
    pub fn user_agents(&self) -> &[String] {
        &self.user_agents
    }

    /// Its Allow and Disallow rules with a pattern, in file order.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Its first Crawl-delay that reads as a number of seconds.
    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }

    /// Its `Content-Signal` lines' signals, each name in lowercase with
    /// `true` for `yes` and `false` for `no`, in file order.
    pub fn content_signals(&self) -> &[(String, bool)] {
        &self.content_signals
    }

    /// The line of its first `User-agent` line, from 1.
    pub fn line(&self) -> usize {
        self.line
    }
}

/// Whether a rule allows or disallows what it matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An `Allow` line.
    Allow,
    /// A `Disallow` line.
    Disallow,
}

/// One Allow or Disallow rule with a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    kind: Kind,
    /// The pattern as written.
    pattern: String,
    /// The pattern as compared: percent-encoded, `*` its wildcard.
    octets: Vec<u8>,
    /// Whether its final `$` anchors it at the target's end.
    anchored: bool,
    line: usize,
    group: usize,
}

impl Rule {
    /// Whether it allows or disallows.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Its pattern, as written.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Its line, from 1.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The index of its group among the file's groups.
    pub fn group(&self) -> usize {
        self.group
    }

    /// Its length as RFC 9309 compares rules: its octets, `$` counted.
    fn length(&self) -> usize {
        self.octets.len() + usize::from(self.anchored)
    }

    /// Whether it matches `target`, a target as compared.
    fn matches(&self, target: &[u8]) -> bool {
        wildcard_match(&self.octets, self.anchored, target)
    }
}

impl fmt::Display for Rule {
    /// The rule as `Allow: <pattern>` or `Disallow: <pattern>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = match self.kind {
            Kind::Allow => "Allow",
            Kind::Disallow => "Disallow",
        };
        write!(formatter, "{key}: {}", self.pattern)
    }
}

/// Whether a set of rules lets a crawler request a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judgment<'a> {
    /// Allowed, by the rule that won, or by none matching.
    Allowed(Option<&'a Rule>),
    /// Disallowed by the rule that won.
    Disallowed(&'a Rule),
}

/// The file `bytes` hold, read as RFC 9309 says: its first [`MAX_SIZE`]
/// bytes, a byte that is not UTF-8 read as U+FFFD.
pub fn parse(bytes: &[u8]) -> RobotsTxt {
    let bytes = &bytes[..bytes.len().min(MAX_SIZE)];
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut file = RobotsTxt::default();
    // Whether the last group's run of `User-agent` lines is still open.
    let mut agents_open = false;
    for (index, line) in lines(bytes).enumerate() {
        let number = index + 1;
        let line = String::from_utf8_lossy(line);
        let line = line.split('#').next().unwrap_or_default();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim_matches([' ', '\t']);
        match key.trim_matches([' ', '\t']).to_ascii_lowercase().as_str() {
            "user-agent" => {
                if !agents_open {
                    file.groups.push(Group {
                        user_agents: Vec::new(),
                        rules: Vec::new(),
                        crawl_delay: None,
                        content_signals: Vec::new(),
                        line: number,
                    });
                    agents_open = true;
                }
                if let Some(group) = file.groups.last_mut() {
                    group.user_agents.push(value.to_owned());
                }
            }
            key @ ("allow" | "disallow") => {
                agents_open = false;
                let Some(index) = file.groups.len().checked_sub(1) else {
                    continue;
                };
                let Some(group) = file.groups.last_mut() else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                let (octets, anchored) = pattern_octets(value);
                group.rules.push(Rule {
                    kind: if key == "allow" {
                        Kind::Allow
                    } else {
                        Kind::Disallow
                    },
                    pattern: value.to_owned(),
                    octets,
                    anchored,
                    line: number,
                    group: index,
                });
            }
            "crawl-delay" => {
                agents_open = false;
                if let Some(group) = file.groups.last_mut()
                    && group.crawl_delay.is_none()
                {
                    group.crawl_delay = seconds(value);
                }
            }
            "content-signal" => {
                agents_open = false;
                if let Some(group) = file.groups.last_mut() {
                    group.content_signals.extend(content_signals(value));
                }
            }
            "sitemap" if !value.is_empty() => file.sitemaps.push(value.to_owned()),
            _ => {}
        }
    }
    file
}

/// The signals of a `Content-Signal` line's `value`: each `name=yes` or
/// `name=no`, its name in lowercase; any other member says nothing.
fn content_signals(value: &str) -> impl Iterator<Item = (String, bool)> + '_ {
    value.split(',').filter_map(|member| {
        let (name, preference) = member.split_once('=')?;
        let allowed = match preference.trim().to_ascii_lowercase().as_str() {
            "yes" => true,
            "no" => false,
            _ => return None,
        };
        Some((name.trim().to_ascii_lowercase(), allowed))
    })
}

/// Whether `rules` let a crawler request `target`, a URL's path and query
/// as [`target`] gives them: the longest rule that matches decides, an
/// Allow winning a tie; none matching, or `/robots.txt`, allows.
pub fn judge<'a>(rules: impl IntoIterator<Item = &'a Rule>, target: &str) -> Judgment<'a> {
    if target == "/robots.txt" {
        return Judgment::Allowed(None);
    }
    let target = target_octets(target);
    let mut best: Option<&Rule> = None;
    for rule in rules {
        if !rule.matches(&target) {
            continue;
        }
        let wins = best.is_none_or(|current| {
            rule.length() > current.length()
                || (rule.length() == current.length()
                    && rule.kind == Kind::Allow
                    && current.kind == Kind::Disallow)
        });
        if wins {
            best = Some(rule);
        }
    }
    match best {
        Some(rule) if rule.kind == Kind::Disallow => Judgment::Disallowed(rule),
        best => Judgment::Allowed(best),
    }
}

/// Whether `pattern`, written as an Allow or Disallow rule's pattern,
/// matches `target`, a URL's path and query as [`target`] gives them, as
/// [`judge`] matches a rule: from its start, `*` matching any octets and a
/// final `$` the target's end, both compared as percent-encoded octets. One
/// pass of at most `pattern.len() * target.len()` steps, so no pattern
/// makes it slow: other files that state paths as robots.txt does, such as
/// `TDMRep`'s, match through it.
pub fn matches(pattern: &str, target: &str) -> bool {
    let (octets, anchored) = pattern_octets(pattern);
    wildcard_match(&octets, anchored, &target_octets(target))
}

/// What robots.txt rules are matched against for `url`: its path and its
/// query, with no fragment.
pub fn target(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// The robots.txt of `url`'s origin (scheme, host and port, RFC 9309
/// §2.3), for an `http:` or `https:` URL.
pub fn location(url: &Url) -> Option<Url> {
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    let mut location = url.clone();
    location.set_path("/robots.txt");
    location.set_query(None);
    location.set_fragment(None);
    location.set_username("").ok()?;
    location.set_password(None).ok()?;
    Some(location)
}

/// What one fetch of an origin's robots.txt gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// A `2xx` with the file.
    File(RobotsTxt),
    /// A `4xx` other than `429`: no rules (RFC 9309 §2.3.1.3).
    NotFound(u16),
    /// A `429`, a `5xx`, no answer or no file: RFC 9309 §2.3.1.4 has a
    /// crawler assume complete disallow.
    Unreachable(String),
}

impl Fetched {
    /// What a response with `status` and `body` gives.
    pub fn of_response(status: u16, body: &[u8]) -> Self {
        match status {
            200..=299 => Self::File(parse(body)),
            429 => Self::Unreachable(format!("answered {status}")),
            400..=499 => Self::NotFound(status),
            _ => Self::Unreachable(format!("answered {status}")),
        }
    }
}

/// What one fetch of an origin's robots.txt was answered, as a copy keeps
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// A response with its status and the first [`MAX_SIZE`] bytes of its
    /// body, after its redirects.
    Response {
        /// Its status.
        status: u16,
        /// Its body.
        body: Vec<u8>,
    },
    /// No response, or a body cut short, for the reason given.
    Unreachable(String),
}

impl Answer {
    /// What it gives (RFC 9309 §2.3.1).
    pub fn fetched(&self) -> Fetched {
        match self {
            Self::Response { status, body } => Fetched::of_response(*status, body),
            Self::Unreachable(reason) => Fetched::Unreachable(reason.clone()),
        }
    }

    /// Whether it applies as it is, a `2xx` file or a `4xx` other than
    /// `429` read as no rules, rather than leaving the last copy applying
    /// (ADR 0086).
    pub fn usable(&self) -> bool {
        matches!(self, Self::Response { status, .. }
            if (200..=299).contains(status) || ((400..=499).contains(status) && *status != 429))
    }
}

/// A fetch of an origin's robots.txt, not started until a copy polls it.
pub type Fetch<'a> = Pin<Box<dyn Future<Output = Answer> + Send + 'a>>;

/// What a copy of an origin's robots.txt answers.
pub type Copied<'a> = Pin<Box<dyn Future<Output = Arc<Fetched>> + Send + 'a>>;

/// Where a Run reads the robots.txt files of its origins: its own copies
/// ([`RunCopies`]) or, on a crawl's Run, the copy Foxhound shares across
/// the user's runs (ADR 0086).
pub trait Copies: Send + Sync + fmt::Debug {
    /// The robots.txt at `location` as it applies now, sending `fetch` only
    /// when the copy holds none to use.
    fn copy<'a>(&'a self, location: &'a Url, fetch: Fetch<'a>) -> Copied<'a>;
}

/// A reading of an origin's robots.txt that never reached the site, for
/// the reason given: the engine failed before or while it sent it, as when
/// the Session it read through closed, its lease ended or its browser went.
/// It is no answer of the site's, which RFC 9309 §2.3.1.4's "unreachable"
/// and ADR 0086's are, so no copy keeps it, and the next reader fetches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotRead(pub String);

/// The table a Run's own copies live in ([`RunCopies`]), which every run
/// file holds: one row per robots.txt URL, the answer of its one fetch, a
/// response's `status` and the first [`MAX_SIZE`] bytes of its `body`, or
/// why it was `unreachable`.
pub const TABLES: RunStateTables = RunStateTables {
    name: "robots-txt-copies",
    statements: &[
        "CREATE TABLE IF NOT EXISTS robots_txt_copies (location TEXT NOT NULL PRIMARY KEY, status INTEGER, body BLOB, unreachable TEXT, \
         CHECK ((status IS NULL) = (body IS NULL)), \
         CHECK ((status IS NULL) <> (unreachable IS NULL))) STRICT",
    ],
};

/// The most copies a Run holds in memory beside its readings in flight
/// (STYLE.md: a collection that grows with load is bounded): the latest
/// read, each a parsed file of at most [`MAX_SIZE`] bytes. The run state
/// holds every one.
pub const CACHED: usize = 16;

/// A Run's own copies of the robots.txt files its program's navigation and
/// fetches read, one per origin (ADR 0098): the first reader of an origin
/// fetches it, and every later or concurrent one reads that answer, so a
/// Run fetches each origin's file once, whatever the site answered. The
/// answers live in the Run's run state ([`TABLES`]); in memory are only the
/// origins a reader is reading and the latest [`CACHED`] copies read. A
/// reading that never reached the site ([`NotRead`]) is kept nowhere.
/// Clones share them.
#[derive(Clone, Debug, Default)]
pub struct RunCopies {
    memory: Memory,
}

/// What a Run's copies hold in memory.
#[expect(
    clippy::disallowed_types,
    reason = "the memory is read or written in one step, never across an `.await` or another lock, so there is no lock order to test under loom (ADR 0060)"
)]
type Memory = Arc<std::sync::Mutex<InMemory>>;

#[derive(Debug, Default)]
struct InMemory {
    /// The reading in flight of each origin a reader reads, until its last
    /// reader ends.
    in_flight: HashMap<String, Reading>,
    /// The latest copies read, the latest last, at most [`CACHED`].
    cached: VecDeque<(String, Arc<Fetched>)>,
}

/// One origin's reading: its copy, once a reader read it.
type Reading = Arc<OnceCell<Arc<Fetched>>>;

impl InMemory {
    /// The copy of `location` it caches, made the latest read.
    fn cached(&mut self, location: &str) -> Option<Arc<Fetched>> {
        let at = self
            .cached
            .iter()
            .position(|(cached, _)| cached == location)?;
        let entry = self.cached.remove(at)?;
        let copy = Arc::clone(&entry.1);
        self.cached.push_back(entry);
        Some(copy)
    }

    /// Caches `copy` as `location`'s, the latest read, forgetting the
    /// earliest past [`CACHED`].
    fn cache(&mut self, location: &str, copy: &Arc<Fetched>) {
        if self.cached(location).is_some() {
            return;
        }
        if self.cached.len() == CACHED {
            self.cached.pop_front();
        }
        self.cached
            .push_back((location.to_owned(), Arc::clone(copy)));
    }
}

/// A reader's hold on its origin's [`Reading`]: dropped, by the reader's
/// end or its cancellation, the origin leaves the readings in flight once
/// no other reader holds it.
struct Held {
    memory: Memory,
    location: String,
    reading: Option<Reading>,
}

impl Held {
    /// A hold on `location`'s reading in `in_memory`, `memory` locked, which
    /// starts one when none is in flight.
    fn on(memory: &Memory, in_memory: &mut InMemory, location: &str) -> Self {
        let reading = Arc::clone(in_memory.in_flight.entry(location.to_owned()).or_default());
        Self {
            memory: Arc::clone(memory),
            location: location.to_owned(),
            reading: Some(reading),
        }
    }

    fn reading(&self) -> &Reading {
        self.reading
            .as_ref()
            .expect("invariant: a hold keeps its reading until it drops")
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut in_memory = lock(&self.memory);
        // Every hold is taken and dropped under the lock, so the count seen
        // here is exact: the map's and this hold's.
        if let Some(reading) = self.reading.take()
            && in_memory
                .in_flight
                .get(&self.location)
                .is_some_and(|held| Arc::ptr_eq(held, &reading))
            && Arc::strong_count(&reading) == 2
        {
            in_memory.in_flight.remove(&self.location);
        }
    }
}

/// `memory`, locked.
fn lock(memory: &Memory) -> std::sync::MutexGuard<'_, InMemory> {
    memory
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl RunCopies {
    /// The copy of `location`'s file in the Run whose run state is `state`:
    /// the one a reader of the Run read before, else what `fetch` answers,
    /// kept in `state`. A concurrent reader of the same origin waits for
    /// the first one's reading and answers it. A run state that cannot be
    /// read or written costs only a fetch.
    ///
    /// # Errors
    ///
    /// [`NotRead`] when `fetch` answered it: nothing is kept, so the next
    /// reader fetches, and a waiting one with its own fetch.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future of the reader whose fetch runs drops that fetch
    /// and keeps nothing: a waiting or later reader reads again with its
    /// own.
    pub async fn copy<Fut>(
        &self,
        state: &RunState,
        location: &Url,
        fetch: Fut,
    ) -> Result<Arc<Fetched>, NotRead>
    where
        Fut: Future<Output = Result<Answer, NotRead>>,
    {
        let key = location.as_str();
        let held = {
            let mut in_memory = lock(&self.memory);
            if let Some(copy) = in_memory.cached(key) {
                return Ok(copy);
            }
            Held::on(&self.memory, &mut in_memory, key)
        };
        let copy = held
            .reading()
            .get_or_try_init(|| async {
                if let Some(answer) = stored(state, key).await {
                    return Ok(Arc::new(answer.fetched()));
                }
                let answer = fetch.await?;
                keep(state, key, &answer).await;
                Ok(Arc::new(answer.fetched()))
            })
            .await
            .map(Arc::clone)?;
        // Cached while the hold lasts, so a later reader finds it in memory
        // or in the run state.
        lock(&self.memory).cache(key, &copy);
        drop(held);
        Ok(copy)
    }

    /// How many origins' copies it holds in memory, those its readers are
    /// reading included: at most [`CACHED`] beside those.
    pub fn held(&self) -> usize {
        let in_memory = lock(&self.memory);
        let reading = in_memory
            .in_flight
            .keys()
            .filter(|location| {
                !in_memory
                    .cached
                    .iter()
                    .any(|(cached, _)| cached == *location)
            })
            .count();
        in_memory.cached.len() + reading
    }
}

/// The answer `state` keeps for `location`, none when it keeps none or
/// cannot be read.
async fn stored(state: &RunState, location: &str) -> Option<Answer> {
    let location = location.to_owned();
    state
        .read(move |connection| {
            connection
                .prepare_cached(
                    "SELECT status, body, unreachable FROM robots_txt_copies WHERE location = ?1",
                )?
                .query_row([location], |row| {
                    let status: Option<u16> = row.get(0)?;
                    let body: Option<Vec<u8>> = row.get(1)?;
                    let unreachable: Option<String> = row.get(2)?;
                    Ok(match (status, body, unreachable) {
                        (Some(status), Some(body), _) => Some(Answer::Response { status, body }),
                        (_, _, Some(reason)) => Some(Answer::Unreachable(reason)),
                        _ => None,
                    })
                })
                .optional()
                .map(Option::flatten)
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "the Run's robots.txt copy could not be read; it is fetched");
            None
        })
}

/// Keeps `answer` in `state` as `location`'s copy; a copy kept before stays.
async fn keep(state: &RunState, location: &str, answer: &Answer) {
    let location = location.to_owned();
    let (status, body, unreachable) = match answer {
        Answer::Response { status, body } => (Some(*status), Some(body.clone()), None),
        Answer::Unreachable(reason) => (None, None, Some(reason.clone())),
    };
    if let Err(error) = state
        .write(move |connection| {
            connection
                .prepare_cached(
                    "INSERT INTO robots_txt_copies (location, status, body, unreachable) \
                     VALUES (?1, ?2, ?3, ?4) ON CONFLICT (location) DO NOTHING",
                )?
                .execute((location, status, body, unreachable))
                .map(drop)
        })
        .await
    {
        tracing::warn!(%error, "the Run's robots.txt copy was not kept; a later reader may fetch it again");
    }
}

/// The copies a Run reads through [`Copies`] when no crawl shared one: its
/// own, in its run state.
#[derive(Clone)]
pub struct OwnCopies {
    /// Its copies.
    pub copies: RunCopies,
    /// Its run state.
    pub state: RunState,
}

impl fmt::Debug for OwnCopies {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnCopies")
            .field("copies", &self.copies)
            .finish_non_exhaustive()
    }
}

impl Copies for OwnCopies {
    fn copy<'a>(&'a self, location: &'a Url, fetch: Fetch<'a>) -> Copied<'a> {
        Box::pin(async move {
            self.copies
                .copy(&self.state, location, async move { Ok(fetch.await) })
                .await
                .unwrap_or_else(|NotRead(reason)| Arc::new(Fetched::Unreachable(reason)))
        })
    }
}

/// The body of the fuzz target `robots_txt` (ADR 0067): `data` read as a
/// file, then judged against a target made of its own first line. Parsing
/// and judging never panic, the parse is deterministic, and every rule's
/// group holds it.
pub fn fuzz_robots_txt(data: &[u8]) {
    let file = parse(data);
    assert_eq!(file, parse(data), "parsing is not deterministic");
    for (index, group) in file.groups().iter().enumerate() {
        assert!(
            group.rules().iter().all(|rule| rule.group() == index),
            "a rule names another group than its own"
        );
    }
    let target = lines(data)
        .next()
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let rules = file.groups().iter().flat_map(Group::rules);
    let _ = judge(rules, &format!("/{target}"));
}

/// The lines of `bytes`, ended by LF, CRLF or CR.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .iter()
            .position(|&byte| byte == b'\n' || byte == b'\r')
            .unwrap_or(rest.len());
        let line = &rest[..end];
        let after = match rest.get(end..end + 2) {
            Some(b"\r\n") => end + 2,
            _ => (end + 1).min(rest.len()),
        };
        rest = &rest[after..];
        Some(line)
    })
}

/// A Crawl-delay's value as a number of seconds, when it is one.
fn seconds(value: &str) -> Option<Duration> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    Duration::try_from_secs_f64(value.parse().ok()?).ok()
}

/// A rule's pattern as compared, and whether a final `$` anchors it: a `*`
/// stays the wildcard, any `$` before the last is a literal.
fn pattern_octets(pattern: &str) -> (Vec<u8>, bool) {
    let (pattern, anchored) = pattern
        .strip_suffix('$')
        .map_or((pattern, false), |pattern| (pattern, true));
    let mut octets = Vec::with_capacity(pattern.len());
    encode(pattern.as_bytes(), &mut octets, |byte| byte == b'$');
    (octets, anchored)
}

/// A target as compared: its `*` and `$` encoded, so only a rule's own
/// `*` is a wildcard.
fn target_octets(target: &str) -> Vec<u8> {
    let mut octets = Vec::with_capacity(target.len());
    encode(target.as_bytes(), &mut octets, |byte| {
        byte == b'*' || byte == b'$'
    });
    octets
}

/// Appends `bytes` to `out` percent-encoded as RFC 9309 compares them: a
/// percent-encoded unreserved character decoded, every other escape's hex
/// in capitals, an octet outside US-ASCII or one `escaped` names encoded.
fn encode(bytes: &[u8], out: &mut Vec<u8>, escaped: impl Fn(u8) -> bool) {
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%'
            && let Some(value) = bytes
                .get(at + 1..at + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            if value.is_ascii_alphanumeric() || matches!(value, b'-' | b'.' | b'_' | b'~') {
                out.push(value);
            } else {
                push_escape(out, value);
            }
            at += 3;
            continue;
        }
        if !byte.is_ascii() || escaped(byte) {
            push_escape(out, byte);
        } else {
            out.push(byte);
        }
        at += 1;
    }
}

/// Appends `byte` to `out` as `%` and two capital hex digits.
fn push_escape(out: &mut Vec<u8>, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.extend_from_slice(&[
        b'%',
        HEX[usize::from(byte >> 4)],
        HEX[usize::from(byte & 0xf)],
    ]);
}

/// Whether `pattern`, `*` matching any octets, matches a prefix of
/// `target`, or all of it when `anchored`: one greedy pass that returns to
/// the last `*` on a mismatch, in at most `pattern.len() * target.len()`
/// steps, so no pattern makes it slow.
fn wildcard_match(pattern: &[u8], anchored: bool, target: &[u8]) -> bool {
    let (mut at_pattern, mut at_target) = (0, 0);
    // The last `*` and where in the target it was last tried.
    let mut star: Option<(usize, usize)> = None;
    loop {
        match pattern.get(at_pattern) {
            Some(b'*') => {
                star = Some((at_pattern, at_target));
                at_pattern += 1;
                continue;
            }
            None if !anchored || at_target == target.len() => return true,
            Some(&octet) if target.get(at_target) == Some(&octet) => {
                at_pattern += 1;
                at_target += 1;
                continue;
            }
            _ => {}
        }
        match star {
            Some((star_at, tried)) if tried < target.len() => {
                star = Some((star_at, tried + 1));
                at_pattern = star_at + 1;
                at_target = tried + 1;
            }
            _ => return false,
        }
    }
}
