//! The Records a crawl commits (ADRs 0008 and 0103): each kept in the run
//! file's `records` table under its Dataset and its number, which counts
//! from 1 per Dataset, in commit order, with no gaps, beside the crawl
//! request whose attempt committed it, that request's URL, the time of
//! the commit and the Record's Provenance (ADR 0123). Every value the Run
//! resolved is replaced by its `%name%` before a Record, its Provenance or
//! a rejection is written, so no secret lands in a Dataset (ADR 0092).
//! Only Records that passed their Dataset's check, a
//! [`Checked`] value, are committed, inside the write that commits their
//! attempt, with the rejections the attempt staged, which enter their
//! Datasets' rejection tables. [`read`] answers the committed Records a
//! [`Query`] takes, in number order: the one query a reader, an Export and
//! a Sink share.

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetRecord;
use goodfetch_protocol::foxhound::outcomes::DatasetRecords;
use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use goodfetch_protocol::run::PointInTime;
use rusqlite::Connection;
use serde_json::Value;

use crate::rejections::{self, Rejection};

/// Records, in the order they were pushed, each of which holds for the
/// Dataset it names, and the rejections of those that did not: the only
/// value [`commit`] takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    records: Vec<PushedRecord>,
    rejected: Vec<Rejection>,
}

impl Checked {
    /// Records the check found holding.
    pub(crate) fn new(records: Vec<PushedRecord>) -> Self {
        Self {
            records,
            rejected: Vec::new(),
        }
    }

    /// The rejection of a Record the check refused.
    pub(crate) fn rejected(rejection: Rejection) -> Self {
        Self {
            records: Vec::new(),
            rejected: vec![rejection],
        }
    }

    /// Puts `later`'s Records and rejections after these.
    pub fn append(&mut self, mut later: Self) {
        self.records.append(&mut later.records);
        self.rejected.append(&mut later.rejected);
    }

    /// The Records, in order.
    #[must_use]
    pub fn records(&self) -> &[PushedRecord] {
        &self.records
    }
}

/// The attempt whose Records commit: its crawl request's ID and URL, the
/// time of its commit, the Provenance each of its Records carries, and the
/// values its Run resolved, which no Record keeps.
#[derive(Clone, Copy, Debug)]
pub struct Committing<'a> {
    /// The crawl request's ID, its work item's.
    pub request_id: &'a str,
    /// The crawl request's URL.
    pub url: &'a str,
    /// When the attempt commits.
    pub at: &'a PointInTime,
    /// The Provenance of each Record it commits.
    pub provenance: &'a RecordProvenance,
    /// The engine's masks, which hold the values the Run resolved.
    pub masks: &'a Masks,
    /// Where each Record's work is counted (F4, ADR 0140).
    pub work: &'a CountedWork,
}

/// Commits `records`, which the attempt `committing` names staged, in
/// `connection`'s transaction: each Record takes its Dataset's next number
/// and the hash of the schema the run file declares it with, and each of
/// its rejections its Dataset's next number in its rejection table; every
/// value the Run resolved is kept as its `%name%` (ADR 0092).
/// Answers, per Dataset in the order first pushed (those only rejected
/// after), how many it committed and rejected.
///
/// # Errors
///
/// The run file's, when a row cannot be written.
pub fn commit(
    connection: &Connection,
    committing: Committing<'_>,
    records: &Checked,
) -> rusqlite::Result<Vec<DatasetRecords>> {
    let mut insert = connection.prepare(
        "INSERT INTO records (dataset, number, work_item, url, committed_at, data, schema_hash, provenance)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM records WHERE dataset = ?1), ?2, ?3, ?4, ?5,
             (SELECT schema_hash FROM datasets WHERE name = ?1), ?6)",
    )?;
    let provenance = masked(
        committing.masks,
        serde_json::to_value(committing.provenance).expect("invariant: a Provenance encodes"),
    );
    let mut counted: Vec<DatasetRecords> = Vec::new();
    let work = committing.work;
    for record in records.records() {
        // What the row below does per Record: the masked copy of its data,
        // its JSON text, and the subquery on its Run's constant schema hash.
        work.count(Row::RecordDeepCopies);
        work.count(Row::RecordSerializations);
        work.count(Row::RecordConstantLookups);
        insert.execute((
            &record.dataset,
            committing.request_id,
            committing.url,
            committing.at.as_str(),
            masked(committing.masks, record.data.clone()),
            &provenance,
        ))?;
        match counted
            .iter_mut()
            .find(|counted| counted.dataset == record.dataset)
        {
            Some(counted) => counted.committed += 1,
            None => counted.push(DatasetRecords {
                dataset: record.dataset.clone(),
                committed: 1,
                dropped: 0,
                rejected: 0,
            }),
        }
    }
    rejections::write(
        connection,
        committing.request_id,
        &records.rejected,
        &mut |value| masked(committing.masks, value),
        &mut counted,
    )?;
    Ok(counted)
}

/// `value` as JSON text, each value the Run resolved replaced by its
/// `%name%`, as the engine keeps it.
fn masked(masks: &Masks, mut value: Value) -> String {
    masks.apply_value(&mut value, Scope::Kept);
    value.to_string()
}

/// Which committed Records a read takes: one Dataset's, after a number,
/// and with or without a Wall marker (ADR 0052). Every reader of a Dataset
/// filters here, alike.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    /// The Dataset's name.
    pub dataset: &'a str,
    /// The number of the last Record already read: 0 reads from the first.
    pub after: u64,
    /// The number of the last Record to read, such as the last one
    /// committed when an Export began; none reads to the last committed.
    pub through: Option<u64>,
    /// `true` takes only the Records whose Provenance marks a Wall read
    /// past, `false` only the others; none takes every Record.
    pub wall: Option<bool>,
}

/// One committed Record as an Export reads it: its number, its value's
/// JSON text as committed, its Wall marker's JSON text, and when it was
/// committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedText {
    /// Its number in its Dataset.
    pub number: u64,
    /// Its value's JSON text, as committed.
    pub value: String,
    /// The JSON text of the Wall marker its Provenance holds, if any.
    pub wall: Option<String>,
    /// When it was committed, RFC 3339.
    pub committed_at: String,
}

/// The number of `dataset`'s last committed Record, 0 when it has none.
///
/// # Errors
///
/// The run file's, when it cannot be read.
pub fn last_number(connection: &Connection, dataset: &str) -> rusqlite::Result<u64> {
    connection.query_row(
        "SELECT coalesce(max(number), 0) FROM records WHERE dataset = ?1",
        [dataset],
        |row| Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default()),
    )
}

/// The committed Records `query` takes, at most `most`, in number order,
/// each value its JSON text as committed: what an Export writes, byte for
/// byte.
///
/// # Errors
///
/// The run file's, when it cannot be read.
pub fn read_values(
    connection: &Connection,
    query: Query<'_>,
    most: usize,
) -> rusqlite::Result<Vec<CommittedText>> {
    selected(
        connection,
        "number, data, json_extract(provenance, '$.wall'), committed_at",
        query,
        most,
        |row| {
            Ok(CommittedText {
                number: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                value: row.get(1)?,
                wall: row.get(2)?,
                committed_at: row.get(3)?,
            })
        },
    )
}

/// The rows of `columns` of the committed Records `query` takes, each read
/// by `row`: the one query every reader of a Dataset filters through.
fn selected<T>(
    connection: &Connection,
    columns: &str,
    query: Query<'_>,
    most: usize,
    row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let number = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    connection
        .prepare(&format!(
            "SELECT {columns} FROM records
             WHERE dataset = ?1 AND number > ?2 AND number <= ?3
               AND (?5 IS NULL OR (json_extract(provenance, '$.wall') IS NOT NULL) = ?5)
             ORDER BY number LIMIT ?4"
        ))?
        .query_map(
            (
                query.dataset,
                number(query.after),
                query.through.map_or(i64::MAX, number),
                i64::try_from(most).unwrap_or(i64::MAX),
                query.wall,
            ),
            row,
        )?
        .collect()
}

/// The committed Records `query` takes, at most `most`, in number order.
///
/// # Errors
///
/// The run file's, when it cannot be read or a Record is no JSON.
pub fn read(
    connection: &Connection,
    query: Query<'_>,
    most: usize,
) -> rusqlite::Result<Vec<DatasetRecord>> {
    selected(connection, RECORD_COLUMNS, query, most, dataset_record)
}

/// The committed Records `query` takes, at most `most`, in number order,
/// and whether every Record numbered through `query.through` has
/// committed (`false` without one), both read in one transaction, so from
/// one state of the run file: when it has and fewer than `most` came, the
/// page holds the last Record through it, however the writer commits
/// meanwhile.
///
/// # Errors
///
/// As [`read`].
pub(crate) fn read_through(
    connection: &Connection,
    query: Query<'_>,
    most: usize,
) -> rusqlite::Result<(Vec<DatasetRecord>, bool)> {
    let transaction = connection.unchecked_transaction()?;
    let page = read(&transaction, query, most)?;
    let committed = match query.through {
        Some(through) => last_number(&transaction, query.dataset)? >= through,
        None => false,
    };
    transaction.finish()?;
    Ok((page, committed))
}

/// The columns of `records` [`dataset_record`] reads, in its order.
pub(crate) const RECORD_COLUMNS: &str = "number, data, work_item, url, committed_at, provenance";

/// The Record a row of [`RECORD_COLUMNS`] holds.
pub(crate) fn dataset_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DatasetRecord> {
    let data: String = row.get(1)?;
    let value = json_column(1, &data)?;
    let provenance: String = row.get(5)?;
    let provenance = json_column(5, &provenance)?;
    Ok(DatasetRecord {
        number: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
        value,
        request_id: row.get(2)?,
        url: row.get(3)?,
        committed_at: PointInTime::from_rfc3339(row.get(4)?),
        provenance,
    })
}

/// The JSON text `text` of the column `column` read as `T`.
fn json_column<T: serde::de::DeserializeOwned>(column: usize, text: &str) -> rusqlite::Result<T> {
    serde_json::from_str(text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

#[cfg(test)]
mod tests;
