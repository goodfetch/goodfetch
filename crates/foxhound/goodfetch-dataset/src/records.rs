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
//! a Sink share, which [`Pages`] reads a page at a time.

use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::StoredTime;
use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::runs::{RunError, RunReader};
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetRecord;
use goodfetch_protocol::foxhound::outcomes::DatasetRecords;
use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use rusqlite::{Connection, ToSql as _};

use crate::rejections::{self, Rejection};

/// Records, in the order they were pushed, each of which holds for the
/// Dataset it names, and the rejections of those that did not: the only
/// value [`commit`] takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    records: Vec<PushedRecord>,
    /// The hash of the schema each Record was checked against, at the
    /// Record's position: none for a Dataset with no schema.
    schema_hashes: Vec<Option<Arc<str>>>,
    rejected: Vec<Rejection>,
}

impl Checked {
    /// Records the check found holding, each with the hash of the schema
    /// its Dataset declares.
    pub(crate) fn new(checked: Vec<(PushedRecord, Option<Arc<str>>)>) -> Self {
        let (records, schema_hashes) = checked.into_iter().unzip();
        Self {
            records,
            schema_hashes,
            rejected: Vec::new(),
        }
    }

    /// Records the check found holding, as [`Checked::new`] takes them,
    /// and the rejections of those it refused.
    pub(crate) fn with_rejected(
        checked: Vec<(PushedRecord, Option<Arc<str>>)>,
        rejected: Vec<Rejection>,
    ) -> Self {
        Self {
            rejected,
            ..Self::new(checked)
        }
    }

    /// Puts `later`'s Records and rejections after these.
    pub fn append(&mut self, mut later: Self) {
        self.records.append(&mut later.records);
        self.schema_hashes.append(&mut later.schema_hashes);
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
    pub at: &'a SystemTime,
    /// The Provenance of each Record it commits.
    pub provenance: &'a RecordProvenance,
    /// The engine's masks, which hold the values the Run resolved.
    pub masks: &'a Masks,
    /// Where each Record's work is counted (F4, ADR 0140).
    pub work: &'a CountedWork,
}

/// Commits `records`, which the attempt `committing` names staged, in
/// `connection`'s transaction: each Record takes its Dataset's next number
/// and the hash of the schema it was checked against, the one the run file
/// declares its Dataset with, and each of its rejections its Dataset's next
/// number in its rejection table; every value the Run resolved is kept as
/// its `%name%` (ADR 0092). A Record's data is written from where it is,
/// never copied, and nothing constant for the Run is read for it (ADR
/// 0140's F4).
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
    let mut insert = connection.prepare_cached("INSERT INTO records (dataset, number, work_item, url, committed_at, data, schema_hash, provenance)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM records WHERE dataset = ?1), ?2, ?3, ?4, ?5, ?6, ?7)")?;
    let masks = committing.masks;
    let provenance = masks.json(
        &serde_json::to_value(committing.provenance).expect("invariant: a Provenance encodes"),
        Scope::Kept,
    );
    // The commit's time, written once for all of its Records.
    let committed_at = StoredTime(*committing.at);
    let committed_at = committed_at.to_sql()?;
    let mut counted: Vec<DatasetRecords> = Vec::new();
    let work = committing.work;
    for (record, schema_hash) in records.records.iter().zip(&records.schema_hashes) {
        // What the row below does per Record: its JSON text, masked as it
        // is written, its fields in the order pushed.
        work.count(Row::RecordSerializations);
        insert.execute((
            &record.dataset,
            committing.request_id,
            committing.url,
            &committed_at,
            masks.json_text(record.data.raw(), Scope::Kept),
            schema_hash.as_deref(),
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
        masks,
        &mut counted,
    )?;
    Ok(counted)
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
pub(crate) struct CommittedText {
    /// Its number in its Dataset.
    pub(crate) number: u64,
    /// Its value's JSON text, as committed.
    pub(crate) value: String,
    /// The JSON text of the Wall marker its Provenance holds, if any.
    pub(crate) wall: Option<String>,
    /// When it was committed, RFC 3339.
    pub(crate) committed_at: String,
}

/// The most Records one read of the run file takes.
const PAGE: usize = 256;

/// A cursor over one Dataset's committed Records that a [`Query`] takes,
/// read a page of at most 256 at a time in number order, each page after
/// the last Record the one before held: the paging of `run.records` and of
/// an Export; `dataset.page` and the rejected Records' read run their own
/// SQL.
#[derive(Clone, Debug)]
pub(crate) struct Pages {
    dataset: String,
    after: u64,
    through: Option<u64>,
    wall: Option<bool>,
}

impl Pages {
    /// The cursor over `dataset`'s Records after the number `after`,
    /// through the number `through` (none: to the last committed), with or
    /// without a Wall marker as `wall` says.
    pub(crate) fn new(dataset: &str, after: u64, through: Option<u64>, wall: Option<bool>) -> Self {
        Self {
            dataset: dataset.to_owned(),
            after,
            through,
            wall,
        }
    }

    /// The number of the last Record it read, or the one it started after.
    pub(crate) fn after(&self) -> u64 {
        self.after
    }

    fn query(&self) -> Query<'_> {
        Query {
            dataset: &self.dataset,
            after: self.after,
            through: self.through,
            wall: self.wall,
        }
    }

    /// Goes on after `last`, the number of the last of the `read` Records
    /// of a page, and answers whether the page was full, so another may
    /// follow.
    fn advance(&mut self, read: usize, last: Option<u64>) -> bool {
        if let Some(last) = last {
            self.after = last;
        }
        read == PAGE
    }

    /// The next page, each value its JSON text as committed, as an Export
    /// writes it, and whether it was full.
    ///
    /// # Errors
    ///
    /// The run file's, when it cannot be read.
    pub(crate) fn values(
        &mut self,
        connection: &Connection,
    ) -> rusqlite::Result<(Vec<CommittedText>, bool)> {
        let page = read_values(connection, self.query(), PAGE)?;
        let full = self.advance(page.len(), page.last().map(|record| record.number));
        Ok((page, full))
    }

    /// The next page, whether it was full, and whether every Record
    /// numbered through its `through` has committed (`false` without one),
    /// all read from one state of the run file, as [`read_through`] does.
    ///
    /// # Errors
    ///
    /// As [`read`].
    pub(crate) fn records_through(
        &mut self,
        connection: &Connection,
    ) -> rusqlite::Result<(Vec<DatasetRecord>, bool, bool)> {
        let (page, committed) = read_through(connection, self.query(), PAGE)?;
        let full = self.advance(page.len(), page.last().map(|record| record.number));
        Ok((page, full, committed))
    }

    /// [`Pages::values`] on `reader`'s read-only connection.
    ///
    /// # Errors
    ///
    /// The Run's, when its run file cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: a dropped read leaves the cursor where it was.
    pub(crate) async fn next_values(
        &mut self,
        reader: &mut RunReader,
    ) -> Result<(Vec<CommittedText>, bool), RunError> {
        self.next_on(reader, Self::values).await
    }

    /// [`Pages::records_through`] on `reader`'s read-only connection.
    ///
    /// # Errors
    ///
    /// The Run's, when its run file cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: a dropped read leaves the cursor where it was.
    pub(crate) async fn next_records_through(
        &mut self,
        reader: &mut RunReader,
    ) -> Result<(Vec<DatasetRecord>, bool, bool), RunError> {
        self.next_on(reader, Self::records_through).await
    }

    /// The next page `read` reads on `reader`'s read-only connection, the
    /// cursor moved there and back.
    async fn next_on<T: Send + 'static>(
        &mut self,
        reader: &mut RunReader,
        read: fn(&mut Self, &Connection) -> rusqlite::Result<T>,
    ) -> Result<T, RunError> {
        let mut pages = self.clone();
        let (pages, page) = reader
            .read(move |connection| {
                let page = read(&mut pages, connection)?;
                Ok((pages, page))
            })
            .await?;
        *self = pages;
        Ok(page)
    }
}

/// The number of `dataset`'s last committed Record, 0 when it has none.
///
/// # Errors
///
/// The run file's, when it cannot be read.
pub(crate) fn last_number(connection: &Connection, dataset: &str) -> rusqlite::Result<u64> {
    connection
        .prepare_cached("SELECT coalesce(max(number), 0) FROM records WHERE dataset = ?1")
        .and_then(|mut cached| {
            cached.query_row([dataset], |row| {
                Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default())
            })
        })
}

/// The committed Records `query` takes, at most `most`, in number order,
/// each value its JSON text as committed: what an Export writes, byte for
/// byte.
///
/// # Errors
///
/// The run file's, when it cannot be read.
pub(crate) fn read_values(
    connection: &Connection,
    query: Query<'_>,
    most: usize,
) -> rusqlite::Result<Vec<CommittedText>> {
    selected(
        connection,
        &format!("number, data, {WALL_MARKER}, committed_at"),
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

/// A committed Record's Wall marker, as SQL reads it from its Provenance:
/// null for a Record read with no Wall in its way (ADR 0052). The one place
/// that says where the marker is stored.
const WALL_MARKER: &str = "json_extract(provenance, '$.wall')";

/// The condition that keeps the committed Records the Wall filter bound as
/// `?{at}` asks for: those with a Wall marker when it is true, those
/// without one when false, every Record when it is null. Every reader of
/// a Dataset filters by it: a page of Records and an Export alike.
pub(crate) fn wall_condition(at: usize) -> String {
    format!("(?{at} IS NULL OR ({WALL_MARKER} IS NOT NULL) = ?{at})")
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
    let wall = wall_condition(5);
    connection
        .prepare_cached(&format!(
            "SELECT {columns} FROM records
             WHERE dataset = ?1 AND number > ?2 AND number <= ?3
               AND {wall}
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
        committed_at: row.get::<_, StoredTime>(4)?.0,
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
