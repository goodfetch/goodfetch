//! A crawl's bounds (ADRs 0005, 0093 and 0120; CO-07, CO-08; pitfalls U01
//! and U10): `crawl.max_depth`, its seeds at depth 0, and
//! `crawl.max_requests`, the requests admitted, seeds included, which have
//! no default; and three caps on a URL's shape, on by default, that end the
//! traps no learned rule closed: `crawl.max_url_length` (2,083 characters
//! as parsed, without the fragment), `crawl.max_path_segments` (20, a
//! trailing slash not counted) and `crawl.max_segment_repeats` (2: a
//! segment, or a run of segments, back to back). `null` turns each off.
//!
//! They are checked on what would otherwise be admitted, past the scope
//! and the identity check, so a URL refused by a bound would have been
//! fetched: the bounds first, then the caps, so a cap's count is of URLs
//! only a heuristic cut. Each refusal is counted by its setting in the run
//! file, beside the count of requests admitted, so a finished Run reports
//! what cut it: a bound makes it `incomplete`, a cap `unverified`.
//!
//! The values live in the run file, recorded when the Run begins and again
//! when it resumes under its new settings, so every admission point, a
//! seed added to a running crawl included, reads the same ones.

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_MAX_DEPTH, CRAWL_MAX_PATH_SEGMENTS, CRAWL_MAX_REQUESTS, CRAWL_MAX_SEGMENT_REPEATS,
    CRAWL_MAX_URL_LENGTH, NotAddedReason,
};
use goodfetch_protocol::run::{Completeness, CompletenessReason};
use rusqlite::{Connection, OptionalExtension as _};
use url::{Position, Url};

/// The bounds' values, one row; the count of requests admitted, one row;
/// and each bound's count of refusals, by its setting's name.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-bounds",
    statements: &[
        "CREATE TABLE IF NOT EXISTS bounds (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_depth INTEGER,
            max_requests INTEGER,
            max_url_length INTEGER,
            max_path_segments INTEGER,
            max_segment_repeats INTEGER
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS bounds_admitted (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            count INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS bounds_refusals (
            setting TEXT NOT NULL PRIMARY KEY,
            count INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID",
    ],
};

/// A crawl's bounds and caps, each `None` when off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// `crawl.max_depth`.
    pub max_depth: Option<u64>,
    /// `crawl.max_requests`.
    pub max_requests: Option<u64>,
    /// `crawl.max_url_length`.
    pub max_url_length: Option<u64>,
    /// `crawl.max_path_segments`.
    pub max_path_segments: Option<u64>,
    /// `crawl.max_segment_repeats`.
    pub max_segment_repeats: Option<u64>,
}

impl Default for Bounds {
    /// The built-in values: no bound, and each cap at its default.
    fn default() -> Self {
        let built_in = |setting: &Setting| (setting.default)().as_u64();
        Self {
            max_depth: built_in(&CRAWL_MAX_DEPTH),
            max_requests: built_in(&CRAWL_MAX_REQUESTS),
            max_url_length: built_in(&CRAWL_MAX_URL_LENGTH),
            max_path_segments: built_in(&CRAWL_MAX_PATH_SEGMENTS),
            max_segment_repeats: built_in(&CRAWL_MAX_SEGMENT_REPEATS),
        }
    }
}

impl Bounds {
    /// The bounds `settings` hold.
    pub fn of_settings(settings: &Resolved) -> Self {
        Self {
            max_depth: settings.count(CRAWL_MAX_DEPTH.name),
            max_requests: settings.count(CRAWL_MAX_REQUESTS.name),
            max_url_length: settings.count(CRAWL_MAX_URL_LENGTH.name),
            max_path_segments: settings.count(CRAWL_MAX_PATH_SEGMENTS.name),
            max_segment_repeats: settings.count(CRAWL_MAX_SEGMENT_REPEATS.name),
        }
    }

    /// Records the bounds in the run file `connection` holds, as the Run
    /// begins or resumes, in place of any it held.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute(
            "INSERT INTO bounds (id, max_depth, max_requests, max_url_length, max_path_segments, max_segment_repeats)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET max_depth = excluded.max_depth,
                 max_requests = excluded.max_requests, max_url_length = excluded.max_url_length,
                 max_path_segments = excluded.max_path_segments,
                 max_segment_repeats = excluded.max_segment_repeats",
            (
                self.max_depth.map(sql),
                self.max_requests.map(sql),
                self.max_url_length.map(sql),
                self.max_path_segments.map(sql),
                self.max_segment_repeats.map(sql),
            ),
        )?;
        Ok(())
    }

    /// The bounds the run file `connection` holds; the built-in ones in a
    /// run file whose Run recorded none.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let recorded = connection
            .query_row(
                "SELECT max_depth, max_requests, max_url_length, max_path_segments, max_segment_repeats
                 FROM bounds WHERE id = 1",
                [],
                |row| {
                let value = |index| row.get::<_, Option<i64>>(index).map(|value| value.map(count));
                Ok(Self {
                    max_depth: value(0)?,
                    max_requests: value(1)?,
                    max_url_length: value(2)?,
                    max_path_segments: value(3)?,
                    max_segment_repeats: value(4)?,
                })
                },
            )
            .optional()?;
        Ok(recorded.unwrap_or_default())
    }

    /// Why a request to `url` at `depth` is not added, if it is not: a
    /// bound it is past, then a cap on its shape.
    pub(crate) fn refuses(
        &self,
        connection: &Connection,
        url: &Url,
        depth: u64,
    ) -> rusqlite::Result<Option<NotAddedReason>> {
        if self.max_depth.is_some_and(|max| depth > max) {
            return Ok(Some(NotAddedReason::MaxDepth));
        }
        if let Some(max) = self.max_requests
            && admitted(connection)? >= max
        {
            return Ok(Some(NotAddedReason::MaxRequests));
        }
        Ok(self.shape_refuses(url))
    }

    /// Which cap on its shape `url` is past, if any.
    pub(crate) fn shape_refuses(&self, url: &Url) -> Option<NotAddedReason> {
        let length = url[..Position::AfterQuery].chars().count();
        if self
            .max_url_length
            .is_some_and(|max| more_than(length, max))
        {
            return Some(NotAddedReason::MaxUrlLength);
        }
        let mut segments: Vec<&str> = url.path_segments().into_iter().flatten().collect();
        if segments.last() == Some(&"") {
            segments.pop();
        }
        if self
            .max_path_segments
            .is_some_and(|max| more_than(segments.len(), max))
        {
            return Some(NotAddedReason::MaxPathSegments);
        }
        if self
            .max_segment_repeats
            .is_some_and(|max| repeats_more_than(&segments, max))
        {
            return Some(NotAddedReason::MaxSegmentRepeats);
        }
        None
    }
}

/// Whether `count` is more than `max`.
fn more_than(count: usize, max: u64) -> bool {
    u64::try_from(count).map_or(true, |count| count > max)
}

/// Whether a segment of `segments`, or a run of them, appears more than
/// `max` times back to back, as Heritrix's `maxRepetitions` pattern finds
/// one: a run of `width` segments appears `max + 1` times back to back
/// where `width × max` segments in a row each equal the one `width` before.
fn repeats_more_than(segments: &[&str], max: u64) -> bool {
    let Ok(max) = usize::try_from(max) else {
        return false;
    };
    if max == 0 {
        return !segments.is_empty();
    }
    (1..=segments.len() / (max + 1)).any(|width| {
        let mut equal_in_a_row = 0;
        segments.windows(width + 1).any(|window| {
            if window[0] == window[width] {
                equal_in_a_row += 1;
            } else {
                equal_in_a_row = 0;
            }
            equal_in_a_row >= width * max
        })
    })
}

/// Counts one request admitted into the run file `connection` holds.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn count_admitted(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO bounds_admitted (id, count) VALUES (1, 1)
         ON CONFLICT (id) DO UPDATE SET count = count + 1",
        [],
    )?;
    Ok(())
}

/// Counts one refusal for `reason` when a bound or a cap gave it.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn count_refused(
    connection: &Connection,
    reason: NotAddedReason,
) -> rusqlite::Result<()> {
    let Some((setting, _)) = cut_by(reason) else {
        return Ok(());
    };
    connection.execute(
        "INSERT INTO bounds_refusals (setting, count) VALUES (?1, 1)
         ON CONFLICT (setting) DO UPDATE SET count = count + 1",
        [setting.name],
    )?;
    Ok(())
}

/// What the bounds and caps of the run file `connection` holds cut: each
/// setting that refused a request or a URL, the Completeness it gives the
/// Run, and how many it refused, in the order of the settings' names.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn completeness_reasons(connection: &Connection) -> rusqlite::Result<Vec<CompletenessReason>> {
    let mut statement =
        connection.prepare("SELECT setting, count FROM bounds_refusals ORDER BY setting")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, count(row.get::<_, i64>(1)?)))
    })?;
    let mut reasons = Vec::new();
    for row in rows {
        let (name, refused) = row?;
        let completeness = REFUSALS
            .iter()
            .find(|(_, setting, _)| setting.name == name)
            .map_or(Completeness::Unverified, |(_, _, completeness)| {
                *completeness
            });
        reasons.push(CompletenessReason {
            name,
            completeness,
            count: refused,
        });
    }
    Ok(reasons)
}

/// Each reason a bound or a cap gives, its setting, and the Completeness a
/// refusal of it gives a finished Run: a bound ends it early, a cap leaves
/// it unvouched for.
const REFUSALS: [(NotAddedReason, Setting, Completeness); 5] = [
    (
        NotAddedReason::MaxDepth,
        CRAWL_MAX_DEPTH,
        Completeness::Incomplete,
    ),
    (
        NotAddedReason::MaxRequests,
        CRAWL_MAX_REQUESTS,
        Completeness::Incomplete,
    ),
    (
        NotAddedReason::MaxUrlLength,
        CRAWL_MAX_URL_LENGTH,
        Completeness::Unverified,
    ),
    (
        NotAddedReason::MaxPathSegments,
        CRAWL_MAX_PATH_SEGMENTS,
        Completeness::Unverified,
    ),
    (
        NotAddedReason::MaxSegmentRepeats,
        CRAWL_MAX_SEGMENT_REPEATS,
        Completeness::Unverified,
    ),
];

/// The setting behind `reason` and the Completeness it gives, when a bound
/// or a cap gave it.
fn cut_by(reason: NotAddedReason) -> Option<(&'static Setting, Completeness)> {
    REFUSALS
        .iter()
        .find(|(refused, _, _)| *refused == reason)
        .map(|(_, setting, completeness)| (setting, *completeness))
}

/// The count of requests the run file `connection` holds as admitted.
fn admitted(connection: &Connection) -> rusqlite::Result<u64> {
    Ok(connection
        .query_row(
            "SELECT count FROM bounds_admitted WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map_or(0, count))
}

/// A count as SQLite stores it, saturated.
fn sql(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// A count SQLite stored, never below zero.
fn count(stored: i64) -> u64 {
    u64::try_from(stored).unwrap_or(0)
}

#[cfg(test)]
mod tests;
