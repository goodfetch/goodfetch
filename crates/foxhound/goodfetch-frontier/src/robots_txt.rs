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
//! the readers of one origin wait for one fetch, and an origin is held in
//! memory only while a reader of it is in flight.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use ::url::Url;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::{Politeness, PolitenessTables, milliseconds};
use goodfetch_engine::robots_txt::{Answer, Copied, Copies, Fetch, Fetched, MAX_SIZE};
use rusqlite::{Connection, OptionalExtension as _};
use tokio::sync::OnceCell;

/// How long a copy is used before its origin's robots.txt is fetched
/// again (ADR 0086).
pub(crate) const FRESH_FOR: Duration = Duration::from_hours(24);

/// The table the copies live in, in the per-user Politeness file.
pub(crate) const TABLES: PolitenessTables = PolitenessTables {
    name: "goodfetch-frontier robots.txt",
    statements: &["CREATE TABLE IF NOT EXISTS robots_txt (
            location TEXT NOT NULL PRIMARY KEY,
            fetched_at INTEGER NOT NULL,
            status INTEGER NOT NULL,
            body BLOB NOT NULL
        ) STRICT"],
};

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
    let lasts = i64::try_from(FRESH_FOR.as_millis()).unwrap_or(i64::MAX);
    copy.is_some_and(|copy| {
        now.checked_sub(copy.fetched_at)
            .is_some_and(|age| (0..lasts).contains(&age))
    })
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
        .query_row(
            "SELECT fetched_at, status, body FROM robots_txt WHERE location = ?1",
            [location],
            |row| {
                Ok(Copy {
                    fetched_at: row.get(0)?,
                    status: row.get(1)?,
                    body: row.get(2)?,
                })
            },
        )
        .optional()
}

pub(crate) fn store(connection: &Connection, location: &str, copy: &Copy) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO robots_txt (location, fetched_at, status, body) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (location) DO UPDATE SET fetched_at = ?2, status = ?3, body = ?4",
        (location, copy.fetched_at, copy.status, &copy.body),
    )?;
    Ok(())
}

/// The robots.txt copy a user's runs share, in the Politeness file they
/// share, read at the `Clock`'s wall time. Clones share its fetches in
/// flight.
#[derive(Clone)]
pub struct SharedRobotsTxt {
    politeness: Politeness,
    clock: Clock,
    /// The reading in flight of each origin a reader waits on, so readers
    /// of one engine wait for one fetch; an origin leaves once no reader
    /// waits on it, so the map holds at most one entry per reader in
    /// flight.
    in_flight: InFlight,
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
            in_flight: Arc::default(),
        }
    }

    /// The robots.txt at `location` as it applies now: the copy, while it
    /// is fresh; else what `fetch` answered, kept when it applies as it is.
    /// A file that cannot be read or written costs only a fetch. A
    /// concurrent reader of the same origin waits for the first one's
    /// reading and answers it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future of the reader whose fetch runs drops that fetch
    /// and keeps nothing: a waiting or later reader reads again with its
    /// own.
    async fn read(&self, location: &Url, fetch: Fetch<'_>) -> Arc<Fetched> {
        let held = Held::on(&self.in_flight, location.as_str().to_owned());
        let applied = held
            .reading()
            .get_or_init(|| self.fetched(location, fetch))
            .await;
        Arc::clone(applied)
    }

    /// What applies at `location`, from the copy while it is fresh, else
    /// from what `fetch` answered.
    async fn fetched(&self, location: &Url, fetch: Fetch<'_>) -> Arc<Fetched> {
        let key = location.as_str().to_owned();
        let copy = self
            .politeness
            .transact(move |connection| stored(connection, &key))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "the shared robots.txt copy could not be read; it is fetched");
                None
            });
        if let Some(copy) = copy
            .as_ref()
            .filter(|copy| fresh(Some(copy), milliseconds(self.clock.wall())))
        {
            return Arc::new(copy.applies());
        }
        let answer = fetch.await;
        let applied = applies(copy.as_ref(), &answer);
        if answer.usable()
            && let Some(kept) = kept(copy, &answer, milliseconds(self.clock.wall()))
        {
            let key = location.as_str().to_owned();
            if let Err(error) = self
                .politeness
                .transact(move |connection| store(connection, &key, &kept))
                .await
            {
                tracing::warn!(%error, "the shared robots.txt copy was not kept; it is fetched again");
            }
        }
        Arc::new(applied)
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
