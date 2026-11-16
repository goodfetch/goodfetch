//! The robots.txt copy a user's crawls share (ADR 0086; spec #581): each
//! origin's robots.txt, kept in the per-user Politeness file beside the
//! domains' state, is used for at most [`FRESH_FOR`] after it was fetched,
//! by every run of the user that shares Politeness, then fetched again. An
//! answer that applies as it is, a `2xx` file or a `4xx` other than `429`
//! read as no rules, replaces the copy. A `429`, a `5xx` or no answer is
//! never kept as no rules: the last good copy, however old, keeps applying
//! meanwhile, and with none everything is disallowed (RFC 9309 §2.3.1.4;
//! pitfall U16). Whether a copy is used and what applies after a fetch are
//! pure functions of the copy, the answer and the `Clock`'s reading.
//!
//! On a crawl's Run this copy replaces the per-Run copy of Collie's
//! reading ([`Run::share_robots_txt`](goodfetch_engine::runs::Run::share_robots_txt)),
//! so a crawl fetches each origin's robots.txt once for all its pages; the
//! fetch itself, which the caller gives, is sent under a Permit of its
//! domain key, so it counts in the domain's Politeness. Within one engine,
//! the readers of one origin wait for one fetch, and an origin's reading is
//! held in memory only while a reader of it is in flight. A Run keeps the
//! parse of each copy it read while that copy is fresh, at most
//! [`KEPT_BYTES`] of them, so it reads and parses each origin's robots.txt
//! once a day (ADR 0140's E6): a fresh copy another run replaced meanwhile
//! is not read again until this one ages, and a copy that is not kept,
//! a failed fetch's or one the file did not take, is not kept in memory
//! either, so the next reader reads the file and fetches as before.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use ::url::Url;
use goodfetch_counted_work::Row;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::{
    KeyState, MemoryTables, Politeness, PolitenessTables, milliseconds,
};
use goodfetch_engine::robots_txt::{Answer, Copied, Copies, Fetch, Fetched, MAX_SIZE};
use rusqlite::{Connection, OptionalExtension as _};
use tokio::sync::OnceCell;

/// How long a copy is used before its origin's robots.txt is fetched
/// again (ADR 0086).
pub(crate) const FRESH_FOR: Duration = Duration::from_hours(24);

/// The most a Run's kept parses weigh, each the bytes of its URL and of
/// its file, at most [`MAX_SIZE`], and [`PARSE_BYTES`]: 32 files of the
/// largest size, or thousands of the few kilobytes most sites serve. Past
/// the bound the cache does not grow: moka's `TinyLFU` admits a parse by
/// how often its origin is read, so it evicts parses kept before or keeps
/// the new one out, and an origin not kept is read and parsed again by its
/// next reader.
pub(crate) const KEPT_BYTES: u64 = 16 * 1024 * 1024;

/// What a kept parse weighs beside its URL and file: its groups and rules.
const PARSE_BYTES: usize = 1024;

/// The table the copies live in, in the per-user Politeness file.
pub const TABLES: PolitenessTables = PolitenessTables {
    name: "goodfetch-frontier robots.txt",
    statements: &["CREATE TABLE IF NOT EXISTS robots_txt (
            location TEXT NOT NULL PRIMARY KEY,
            fetched_at INTEGER NOT NULL,
            status INTEGER NOT NULL,
            body BLOB NOT NULL
        ) STRICT"],
    prepared: &[STORED, STORE],
};

// The statements the copies are read and stored with, prepared when the file opens.
const STORED: &str = "SELECT fetched_at, status, body FROM robots_txt WHERE location = ?1";
const STORE: &str =
    "INSERT INTO robots_txt (location, fetched_at, status, body) VALUES (?1, ?2, ?3, ?4)
    ON CONFLICT (location) DO UPDATE SET fetched_at = ?2, status = ?3, body = ?4";

/// An origin's last good copy: the answer that applies as it is, and when
/// it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Copy {
    /// When it was fetched, in milliseconds since the Unix epoch.
    pub(crate) fetched_at: i64,
    /// Its status: a `2xx` with a file, or a `4xx` other than `429`.
    pub(crate) status: u16,
    /// Its body, at most [`MAX_SIZE`] bytes.
    pub(crate) body: Vec<u8>,
}

impl Copy {
    /// What it gives.
    pub(crate) fn applies(&self) -> Fetched {
        Fetched::of_response(self.status, &self.body)
    }
}

/// Whether `copy` is used as it is at `now`, in milliseconds: it was
/// fetched less than [`FRESH_FOR`] before, and not after `now`.
pub(crate) fn fresh(copy: Option<&Copy>, now: i64) -> bool {
    copy.is_some_and(|copy| fresh_since(copy.fetched_at, now))
}

/// Whether a copy fetched at `fetched_at` is used as it is at `now`, both
/// in milliseconds.
fn fresh_since(fetched_at: i64, now: i64) -> bool {
    let lasts = i64::try_from(FRESH_FOR.as_millis()).unwrap_or(i64::MAX);
    now.checked_sub(fetched_at)
        .is_some_and(|age| (0..lasts).contains(&age))
}

/// The copy kept after a fetch answered `answer` at `now`: the answer,
/// when it applies as it is; else `copy` unchanged.
pub(crate) fn kept(copy: Option<Copy>, answer: &Answer, now: i64) -> Option<Copy> {
    match answer {
        Answer::Response { status, body } if answer.usable() => Some(Copy {
            fetched_at: now,
            status: *status,
            body: capped(body),
        }),
        Answer::Response { .. } | Answer::Unreachable(_) => copy,
    }
}

/// What applies after a fetch answered `answer`, `copy` being the last
/// good copy before it: the answer when it applies as it is; else that
/// copy, however old; else nothing is allowed.
pub(crate) fn applies(copy: Option<&Copy>, answer: &Answer) -> Fetched {
    match copy {
        Some(copy) if !answer.usable() => copy.applies(),
        Some(_) | None => answer.fetched(),
    }
}

pub(crate) fn stored(connection: &Connection, location: &str) -> rusqlite::Result<Option<Copy>> {
    connection
        .prepare_cached(STORED)
        .and_then(|mut cached| {
            cached.query_row([location], |row| {
                Ok(Copy {
                    fetched_at: row.get(0)?,
                    status: row.get(1)?,
                    body: row.get(2)?,
                })
            })
        })
        .optional()
}

pub(crate) fn store(connection: &Connection, location: &str, copy: &Copy) -> rusqlite::Result<()> {
    connection.prepare_cached(STORE).and_then(|mut cached| {
        cached.execute((location, copy.fetched_at, copy.status, &copy.body))
    })?;
    Ok(())
}

/// The robots.txt copies a Run's own Politeness keeps in memory, by
/// location.
#[derive(Default)]
struct KeptCopies(HashMap<String, Copy>);

/// The copy a Run's own Politeness keeps in `tables` for `location`.
pub(crate) fn kept_in<'a>(tables: &'a MemoryTables, location: &str) -> Option<&'a Copy> {
    tables
        .get::<KeptCopies>()
        .and_then(|copies| copies.0.get(location))
}

/// The robots.txt copy kept for a location, as a plain value the
/// Politeness file loads and saves.
struct RobotsCopy(Option<Copy>);

impl KeyState for RobotsCopy {
    /// Its location.
    type Key = str;
    /// Whether a change reads the copy.
    type Reads = bool;
    /// Whether a change wrote it.
    type Writes = bool;

    fn load(connection: &Connection, location: &str, reads: bool) -> rusqlite::Result<Self> {
        Ok(Self(if reads {
            stored(connection, location)?
        } else {
            None
        }))
    }

    fn save(&self, connection: &Connection, location: &str, wrote: &bool) -> rusqlite::Result<()> {
        match &self.0 {
            Some(copy) if *wrote => store(connection, location, copy),
            _ => Ok(()),
        }
    }

    fn load_from(tables: &MemoryTables, location: &str, reads: bool) -> Self {
        Self(reads.then(|| kept_in(tables, location).cloned()).flatten())
    }

    fn save_to(&self, tables: &mut MemoryTables, location: &str, wrote: &bool) {
        if let Some(copy) = &self.0
            && *wrote
        {
            tables
                .get_mut::<KeptCopies>()
                .0
                .insert(location.to_owned(), copy.clone());
        }
    }
}

/// The robots.txt copy a user's runs share, in the Politeness file they
/// share, read at the `Clock`'s wall time, one per Run. Clones share its
/// fetches in flight and its kept parses.
#[derive(Clone)]
pub struct SharedRobotsTxt {
    politeness: Politeness,
    clock: Clock,
    /// The parse of each copy the Run read, by its URL, while the copy is
    /// fresh, at most [`KEPT_BYTES`].
    parses: moka::sync::Cache<String, Parse>,
    /// The reading in flight of each origin a reader waits on, so readers
    /// of one engine wait for one fetch; an origin leaves once no reader
    /// waits on it, so the map holds at most one entry per reader in
    /// flight.
    in_flight: InFlight,
}

/// A copy's parse a Run keeps: what applies, and when the copy was
/// fetched, in milliseconds since the Unix epoch.
#[derive(Clone)]
struct Parse {
    applies: Arc<Fetched>,
    fetched_at: i64,
    weight: u32,
}

/// The readings in flight, by origin.
#[expect(
    clippy::disallowed_types,
    reason = "the map is read or written in one step, never across an `.await` or another lock, so there is no lock order to test under loom (ADR 0060)"
)]
type InFlight = Arc<std::sync::Mutex<HashMap<String, Reading>>>;

/// One origin's reading: what applies, once its first reader read it.
type Reading = Arc<OnceCell<Arc<Fetched>>>;

/// A reader's hold on its origin's [`Reading`]: dropped, by the reader's
/// end or its cancellation, the origin leaves [`InFlight`] once no other
/// reader holds it.
struct Held {
    in_flight: InFlight,
    origin: String,
    reading: Option<Reading>,
}

impl Held {
    /// A hold on `origin`'s reading in `in_flight`, which starts it when
    /// none is in flight.
    fn on(in_flight: &InFlight, origin: String) -> Self {
        let reading = Arc::clone(
            in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(origin.clone())
                .or_default(),
        );
        Self {
            in_flight: Arc::clone(in_flight),
            origin,
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
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Every hold is taken and dropped under the lock, so the count
        // seen here is exact: the map's and this hold's.
        if let Some(reading) = self.reading.take()
            && in_flight
                .get(&self.origin)
                .is_some_and(|held| Arc::ptr_eq(held, &reading))
            && Arc::strong_count(&reading) == 2
        {
            in_flight.remove(&self.origin);
        }
    }
}

impl fmt::Debug for SharedRobotsTxt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedRobotsTxt")
            .finish_non_exhaustive()
    }
}

impl SharedRobotsTxt {
    /// The copy kept in `politeness`'s file, read on `clock`.
    pub fn new(politeness: Politeness, clock: Clock) -> Self {
        Self {
            politeness,
            clock,
            parses: moka::sync::Cache::builder()
                .max_capacity(KEPT_BYTES)
                .weigher(|_, parse: &Parse| parse.weight)
                .build(),
            in_flight: Arc::default(),
        }
    }

    /// The robots.txt at `location` as it applies now: the copy, while it
    /// is fresh; else what `fetch` answered, kept when it applies as it is.
    /// A file that cannot be read or written costs only a fetch. A
    /// concurrent reader of the same origin waits for the first one's
    /// reading and answers it, and a later reader of the Run, while the copy
    /// is fresh, its kept parse.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future of the reader whose fetch runs drops that fetch
    /// and keeps nothing: a waiting or later reader reads again with its
    /// own.
    async fn read(&self, location: &Url, fetch: Fetch<'_>) -> Arc<Fetched> {
        if let Some(applies) = self.kept_parse(location.as_str()) {
            // Yields once, as the file read it replaces did: a crawl whose
            // every origin is kept would otherwise run page after page in
            // one poll, and the tasks beside it, the Connection answering
            // the SDK's calls among them, wait until it idles.
            tokio::task::yield_now().await;
            return applies;
        }
        let held = Held::on(&self.in_flight, location.as_str().to_owned());
        let applied = held
            .reading()
            .get_or_init(|| self.fetched(location, fetch))
            .await;
        Arc::clone(applied)
    }

    /// What applies at `location`, the parse the Run keeps, else from the
    /// copy while it is fresh, else from what `fetch` answered. The parse
    /// is looked for again here, under the origin's reading, as the reader
    /// before may have kept it after this one first looked.
    async fn fetched(&self, location: &Url, fetch: Fetch<'_>) -> Arc<Fetched> {
        if let Some(applies) = self.kept_parse(location.as_str()) {
            return applies;
        }
        let key = location.as_str().to_owned();
        self.politeness.work().count(Row::RobotsReads);
        let copy = self
            .politeness
            .change(key, true, |copy: &mut RobotsCopy| (copy.0.take(), false))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "the shared robots.txt copy could not be read; it is fetched");
                None
            });
        if let Some(copy) = copy
            .as_ref()
            .filter(|copy| fresh(Some(copy), milliseconds(self.clock.wall())))
        {
            let applies = self.parsed(copy.applies());
            self.keep(
                location.as_str(),
                copy.fetched_at,
                copy.body.len(),
                &applies,
            );
            return applies;
        }
        let answer = fetch.await;
        let applied = self.parsed(applies(copy.as_ref(), &answer));
        if answer.usable()
            && let Some(kept) = kept(copy, &answer, milliseconds(self.clock.wall()))
        {
            let key = location.as_str().to_owned();
            let (fetched_at, bytes) = (kept.fetched_at, kept.body.len());
            match self
                .politeness
                .change(key, false, move |copy: &mut RobotsCopy| {
                    copy.0 = Some(kept);
                    ((), true)
                })
                .await
            {
                Ok(()) => self.keep(location.as_str(), fetched_at, bytes, &applied),
                Err(error) => {
                    tracing::warn!(%error, "the shared robots.txt copy was not kept; it is fetched again");
                }
            }
        }
        applied
    }

    /// The parse the Run keeps of `location`'s copy, while the copy is
    /// fresh.
    fn kept_parse(&self, location: &str) -> Option<Arc<Fetched>> {
        let parse = self.parses.get(location)?;
        if fresh_since(parse.fetched_at, milliseconds(self.clock.wall())) {
            return Some(parse.applies);
        }
        self.parses.invalidate(location);
        None
    }

    /// Keeps `applies`, the parse of the copy fetched at `fetched_at` whose
    /// body is `bytes` long, as `location`'s while that copy is fresh.
    fn keep(&self, location: &str, fetched_at: i64, bytes: usize, applies: &Arc<Fetched>) {
        let weight = u32::try_from(location.len() + bytes + PARSE_BYTES).unwrap_or(u32::MAX);
        self.parses.insert(
            location.to_owned(),
            Parse {
                applies: Arc::clone(applies),
                fetched_at,
                weight,
            },
        );
    }

    /// `fetched`, a file parsed counted as such (ADR 0140's E6): a robots.txt
    /// is parsed exactly where a copy or an answer gives a file.
    fn parsed(&self, fetched: Fetched) -> Arc<Fetched> {
        if matches!(fetched, Fetched::File(_)) {
            self.politeness.work().count(Row::RobotsParses);
        }
        Arc::new(fetched)
    }
}

impl Copies for SharedRobotsTxt {
    fn copy<'a>(&'a self, location: &'a Url, fetch: Fetch<'a>) -> Copied<'a> {
        Box::pin(self.read(location, fetch))
    }
}

/// The body to keep of a response's `body`: its first [`MAX_SIZE`] bytes.
pub(crate) fn capped(body: &[u8]) -> Vec<u8> {
    body.iter().copied().take(MAX_SIZE).collect()
}

#[cfg(test)]
mod tests;
