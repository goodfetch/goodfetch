//! A crawl's bounds (ADRs 0005, 0093 and 0120; CO-07, CO-08; pitfalls U01
//! and U10): `crawl.max_depth`, its seeds at depth 0, and
//! `crawl.max_requests`, the requests admitted, seeds included, which have
//! no default; `crawl.max_sitemap_nesting`, the levels of sitemaps a
//! sitemap seed reads under it (5), which the sitemaps' admission checks
//! (`sitemap`); and three caps on a URL's shape, on by default, that end the
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
//! `crawl.path_budgets` bounds sections of a site (CO-08; Spider's
//! `with_budget`): each entry's glob is matched against a URL's path and
//! query, the first entry a URL matches counts it, and one past that
//! entry's count is not added with `path_budget`, checked after
//! `crawl.max_requests` and before the caps. Each entry counts what it
//! admitted and what it refused by its pattern, and a finished Run that it
//! refused a URL is `incomplete`, a reason naming the pattern and the
//! counts. Facets get no default rule (ADR 0120): robots.txt, a
//! `crawl.scope.exclude` pattern on their parameters, `crawl.max_requests`
//! or a path budget bound them.
//!
//! The values live in the run file, recorded when the Run begins and again
//! when it resumes under its new settings, so every admission point, a
//! seed added to a running crawl included, reads the same ones. Beside
//! them lives `frontier.learning` (ADR 0138), which admission reads with
//! them: off, an admission keeps no evidence for the learned rules and no
//! learned cursor refuses a URL.

use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::CompletenessReason;
use goodfetch_engine::settings::Resolved;
use goodfetch_page::url_pattern::{UnsupportedPattern, UrlPattern};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_MAX_DEPTH, CRAWL_MAX_PATH_SEGMENTS, CRAWL_MAX_REQUESTS, CRAWL_MAX_SEGMENT_REPEATS,
    CRAWL_MAX_SITEMAP_NESTING, CRAWL_MAX_URL_LENGTH, CRAWL_PATH_BUDGETS, FRONTIER_LEARNING,
    NotAddedReason, PathBudget,
};
use goodfetch_protocol::run::{AxisReason, Completeness};
use rusqlite::{Connection, OptionalExtension as _};
use url::{Position, Url};

use crate::staged::Staged;

/// The bounds' values and `frontier.learning`, one row, and the path
/// budgets, in order; the count
/// of requests admitted, one row; each bound's count of refusals, by its
/// setting's name; and each path budget's counts, by its pattern.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-bounds",
    statements: &[
        "CREATE TABLE IF NOT EXISTS bounds (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_depth INTEGER,
            max_requests INTEGER,
            max_url_length INTEGER,
            max_path_segments INTEGER,
            max_segment_repeats INTEGER,
            max_sitemap_nesting INTEGER,
            learning INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS bounds_admitted (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            count INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS bounds_refusals (
            setting TEXT NOT NULL PRIMARY KEY,
            count INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS bounds_path_budgets (
            position INTEGER NOT NULL PRIMARY KEY,
            pattern TEXT NOT NULL,
            max_requests INTEGER NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS bounds_path_counts (
            pattern TEXT NOT NULL PRIMARY KEY,
            admitted INTEGER NOT NULL,
            refused INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID",
    ],
};

/// A crawl's bounds and caps, each `None` when off, and whether its
/// admission learns.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// `crawl.max_sitemap_nesting`.
    pub max_sitemap_nesting: Option<u64>,
    /// `crawl.path_budgets`, in order.
    pub path_budgets: Vec<PathBudget>,
    /// `frontier.learning`: whether an admission keeps what it shows the
    /// learned rules and lets a learned cursor refuse a URL.
    pub learning: bool,
}

impl Default for Bounds {
    /// The built-in values: no bound, each cap at its default, and
    /// learning on.
    fn default() -> Self {
        let built_in = |setting: &Setting| (setting.default)().as_u64();
        Self {
            max_depth: built_in(&CRAWL_MAX_DEPTH),
            max_requests: built_in(&CRAWL_MAX_REQUESTS),
            max_url_length: built_in(&CRAWL_MAX_URL_LENGTH),
            max_path_segments: built_in(&CRAWL_MAX_PATH_SEGMENTS),
            max_segment_repeats: built_in(&CRAWL_MAX_SEGMENT_REPEATS),
            max_sitemap_nesting: built_in(&CRAWL_MAX_SITEMAP_NESTING),
            path_budgets: Vec::new(),
            learning: (FRONTIER_LEARNING.default)()
                .as_bool()
                .expect("invariant: `frontier.learning`'s default is a boolean"),
        }
    }
}

impl Bounds {
    /// The bounds `settings` hold.
    pub fn of_settings(settings: &Resolved) -> Self {
        Self {
            max_depth: settings.get(&CRAWL_MAX_DEPTH),
            max_requests: settings.get(&CRAWL_MAX_REQUESTS),
            max_url_length: settings.get(&CRAWL_MAX_URL_LENGTH),
            max_path_segments: settings.get(&CRAWL_MAX_PATH_SEGMENTS),
            max_segment_repeats: settings.get(&CRAWL_MAX_SEGMENT_REPEATS),
            max_sitemap_nesting: settings.get(&CRAWL_MAX_SITEMAP_NESTING),
            path_budgets: PathBudget::list(settings.value(&CRAWL_PATH_BUDGETS)),
            learning: settings.get(&FRONTIER_LEARNING),
        }
    }

    /// Whether the matcher reads every path budget's pattern.
    ///
    /// # Errors
    ///
    /// The first pattern it cannot read, which the start raises as
    /// `pattern_unsupported`.
    pub fn check(&self) -> Result<(), UnsupportedPattern> {
        for budget in &self.path_budgets {
            UrlPattern::glob(&budget.pattern)?;
        }
        Ok(())
    }

    /// Records the bounds in the run file `connection` holds, as the Run
    /// begins or resumes, in place of any it held.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written, or a path budget's pattern is
    /// one the matcher cannot read, as a resume's settings may hold.
    pub fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        self.check()
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        connection.prepare_cached("INSERT INTO bounds (id, max_depth, max_requests, max_url_length, max_path_segments, max_segment_repeats, max_sitemap_nesting, learning)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET max_depth = excluded.max_depth,
                 max_requests = excluded.max_requests, max_url_length = excluded.max_url_length,
                 max_path_segments = excluded.max_path_segments,
                 max_segment_repeats = excluded.max_segment_repeats,
                  max_sitemap_nesting = excluded.max_sitemap_nesting,
                  learning = excluded.learning").and_then(|mut cached| cached.execute((
                self.max_depth.map(sql),
                self.max_requests.map(sql),
                self.max_url_length.map(sql),
                self.max_path_segments.map(sql),
                self.max_segment_repeats.map(sql),
                self.max_sitemap_nesting.map(sql),
                self.learning,
            )))?;
        connection
            .prepare_cached("DELETE FROM bounds_path_budgets")
            .and_then(|mut cached| cached.execute([]))?;
        for (position, budget) in self.path_budgets.iter().enumerate() {
            connection
                .prepare_cached(
                    "INSERT INTO bounds_path_budgets (position, pattern, max_requests)
                 VALUES (?1, ?2, ?3)",
                )
                .and_then(|mut cached| {
                    cached.execute((
                        sql(u64::try_from(position).unwrap_or(u64::MAX)),
                        &budget.pattern,
                        sql(budget.max_requests),
                    ))
                })?;
        }
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
            .prepare_cached(
                "SELECT max_depth, max_requests, max_url_length, max_path_segments, max_segment_repeats,
                     max_sitemap_nesting, learning
                 FROM bounds WHERE id = 1",
            )?
            .query_row([], |row| {
                let value = |index| row.get::<_, Option<i64>>(index).map(|value| value.map(count));
                Ok(Self {
                    max_depth: value(0)?,
                    max_requests: value(1)?,
                    max_url_length: value(2)?,
                    max_path_segments: value(3)?,
                    max_segment_repeats: value(4)?,
                    max_sitemap_nesting: value(5)?,
                    path_budgets: Vec::new(),
                    learning: row.get(6)?,
                })
            })
            .optional()?;
        let mut bounds = recorded.unwrap_or_default();
        let mut statement = connection.prepare_cached(
            "SELECT pattern, max_requests FROM bounds_path_budgets ORDER BY position",
        )?;
        bounds.path_budgets = statement
            .query_map([], |row| {
                Ok(PathBudget {
                    pattern: row.get(0)?,
                    max_requests: count(row.get(1)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(bounds)
    }

    /// Why a request to `url` at `depth`, counted by the path budget
    /// `budget`, is not added, if it is not: a bound it is past, then a cap
    /// on its shape. The requests `staged`, what an attempt's earlier staged
    /// admissions did, counted are counted with the run file's.
    pub(crate) fn refuses(
        &self,
        connection: &Connection,
        url: &Url,
        depth: u64,
        budget: Option<&PathBudget>,
        staged: Option<&Staged>,
    ) -> rusqlite::Result<Option<NotAddedReason>> {
        Ok(self
            .past(connection, depth, budget, staged)?
            .or_else(|| self.shape_refuses(url)))
    }

    /// The bound a request at `depth`, counted by the path budget `budget`,
    /// is past, if any: its depth, then the Run's count of requests, then
    /// its path budget's, each count read where the Run's state is held
    /// (#2244), with what `staged` counted. The caps on its URL's shape,
    /// which come after them, are read from the URL alone
    /// ([`Bounds::shape_refuses`]).
    pub(crate) fn past(
        &self,
        connection: &Connection,
        depth: u64,
        budget: Option<&PathBudget>,
        staged: Option<&Staged>,
    ) -> rusqlite::Result<Option<NotAddedReason>> {
        if self.max_depth.is_some_and(|max| depth > max) {
            return Ok(Some(NotAddedReason::MaxDepth));
        }
        if let Some(max) = self.max_requests
            && admitted(connection)? + staged.map_or(0, Staged::admitted) >= max
        {
            return Ok(Some(NotAddedReason::MaxRequests));
        }
        if let Some(budget) = budget
            && path_admitted(connection, &budget.pattern)?
                + staged.map_or(0, |staged| staged.budget(&budget.pattern))
                >= budget.max_requests
        {
            return Ok(Some(NotAddedReason::PathBudget));
        }
        Ok(None)
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

/// Counts one request admitted into the run file `connection` holds, and
/// against the path budget `budget` that counts it.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn count_admitted(
    connection: &Connection,
    budget: Option<&PathBudget>,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO bounds_admitted (id, count) VALUES (1, 1)
             ON CONFLICT (id) DO UPDATE SET count = count + 1",
        )?
        .execute([])?;
    if let Some(budget) = budget {
        connection
            .prepare_cached(
                "INSERT INTO bounds_path_counts (pattern, admitted, refused) VALUES (?1, 1, 0)
                 ON CONFLICT (pattern) DO UPDATE SET admitted = admitted + 1",
            )?
            .execute([&budget.pattern])?;
    }
    Ok(())
}

/// Counts one refusal for `reason` when a bound or a cap gave it, against
/// the path budget `budget` when it was that budget's.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn count_refused(
    connection: &Connection,
    reason: NotAddedReason,
    budget: Option<&PathBudget>,
) -> rusqlite::Result<()> {
    if reason == NotAddedReason::PathBudget
        && let Some(budget) = budget
    {
        connection
            .prepare_cached(
                "INSERT INTO bounds_path_counts (pattern, admitted, refused) VALUES (?1, 0, 1)
                 ON CONFLICT (pattern) DO UPDATE SET refused = refused + 1",
            )?
            .execute([&budget.pattern])?;
        return Ok(());
    }
    let Some(setting) = cut_by(reason) else {
        return Ok(());
    };
    connection
        .prepare_cached(
            "INSERT INTO bounds_refusals (setting, count) VALUES (?1, 1)
             ON CONFLICT (setting) DO UPDATE SET count = count + 1",
        )?
        .execute([setting.name])?;
    Ok(())
}

/// What the bounds and caps of the run file `connection` holds cut: each
/// setting that refused a request or a URL, the Completeness it gives the
/// Run, and a reason coded by the setting's name that says how many it
/// refused, in the order of the settings' names; each path budget that
/// refused one is a reason of its own, in the order of the entries.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn completeness_reasons(connection: &Connection) -> rusqlite::Result<Vec<CompletenessReason>> {
    let mut statement =
        connection.prepare_cached("SELECT setting, count FROM bounds_refusals ORDER BY setting")?;
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
            completeness,
            reason: refused_by(name, refused),
        });
    }
    reasons.extend(budget_reasons(connection)?);
    reasons.sort_by(|one, other| one.reason.code.cmp(&other.reason.code));
    Ok(reasons)
}

/// Each path budget of the run file `connection` holds that refused a URL,
/// in the order of the entries, one no longer set last: a reason naming its
/// pattern, its count and how many it refused, making the Run `incomplete`.
fn budget_reasons(connection: &Connection) -> rusqlite::Result<Vec<CompletenessReason>> {
    let mut statement = connection.prepare_cached(
        "SELECT counts.pattern, counts.refused, budgets.max_requests
         FROM bounds_path_counts AS counts
         LEFT JOIN (SELECT pattern, max_requests, MIN(position) AS position
                    FROM bounds_path_budgets GROUP BY pattern) AS budgets
           ON budgets.pattern = counts.pattern
         WHERE counts.refused > 0
         ORDER BY budgets.position IS NULL, budgets.position, counts.pattern",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            count(row.get(1)?),
            row.get::<_, Option<i64>>(2)?.map(count),
        ))
    })?;
    let plural = |count: u64, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    rows.map(|row| {
        let (pattern, refused, max_requests) = row?;
        let past = max_requests.map_or_else(String::new, |max| {
            format!(" past its {}", plural(max, "request", "requests"))
        });
        Ok(CompletenessReason {
            completeness: Completeness::Incomplete,
            reason: AxisReason {
                code: CRAWL_PATH_BUDGETS.name.to_owned(),
                message: format!(
                    "`{}`'s entry `{pattern}` refused {}{past} the crawl would otherwise have admitted.",
                    CRAWL_PATH_BUDGETS.name,
                    plural(refused, "URL", "URLs"),
                ),
                count: Some(refused),
                call_id: None,
            },
        })
    })
    .collect()
}

/// The count of requests the path budget of `pattern` admitted so far.
fn path_admitted(connection: &Connection, pattern: &str) -> rusqlite::Result<u64> {
    Ok(connection
        .prepare_cached("SELECT admitted FROM bounds_path_counts WHERE pattern = ?1")?
        .query_row([pattern], |row| row.get::<_, i64>(0))
        .optional()?
        .map_or(0, count))
}

/// The reason the setting `name` gives a finished Run, having refused
/// `count` URLs.
fn refused_by(name: String, count: u64) -> AxisReason {
    let urls = if count == 1 { "URL" } else { "URLs" };
    AxisReason {
        message: format!(
            "`{name}` refused {count} {urls} the crawl would otherwise have admitted."
        ),
        code: name,
        count: Some(count),
        call_id: None,
    }
}

/// Each reason a bound or a cap gives, its setting, and the Completeness a
/// refusal of it gives a finished Run: a bound ends it early, a cap leaves
/// it unvouched for.
const REFUSALS: [(NotAddedReason, Setting, Completeness); 6] = [
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
        NotAddedReason::MaxSitemapNesting,
        CRAWL_MAX_SITEMAP_NESTING,
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

/// The setting behind `reason`, when a bound or a cap gave it.
pub(crate) fn cut_by(reason: NotAddedReason) -> Option<&'static Setting> {
    REFUSALS
        .iter()
        .find(|(refused, _, _)| *refused == reason)
        .map(|(_, setting, _)| setting)
}

/// The count of requests the run file `connection` holds as admitted.
fn admitted(connection: &Connection) -> rusqlite::Result<u64> {
    Ok(connection
        .prepare_cached("SELECT count FROM bounds_admitted WHERE id = 1")?
        .query_row([], |row| row.get::<_, i64>(0))
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
