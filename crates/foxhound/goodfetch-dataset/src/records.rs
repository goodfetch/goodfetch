//! The Records a crawl commits (ADRs 0008 and 0103): each kept in the run
//! file's `records` table under its Dataset and its number, which counts
//! from 1 per Dataset, in commit order, with no gaps, beside the crawl
//! request whose attempt committed it, that request's URL and the time of
//! the commit. Only Records that passed their Dataset's check, a
//! [`Checked`] value, are committed, inside the write that commits their
//! attempt. [`read`] answers the committed Records a [`Query`] takes, in
//! number order: the one query a reader, an Export and a Sink share.

use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::dataset::DatasetRecord;
use goodfetch_protocol::foxhound::outcomes::DatasetRecords;
use goodfetch_protocol::run::PointInTime;
use rusqlite::Connection;

/// Records, in the order they were pushed, each of which holds for the
/// Dataset it names: the only value [`commit`] takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked(Vec<PushedRecord>);

impl Checked {
    /// Records the check found holding.
    pub(crate) fn new(records: Vec<PushedRecord>) -> Self {
        Self(records)
    }

    /// Puts `later`'s Records after these.
    pub fn append(&mut self, mut later: Self) {
        self.0.append(&mut later.0);
    }

    /// The Records, in order.
    #[must_use]
    pub fn records(&self) -> &[PushedRecord] {
        &self.0
    }
}

/// The attempt whose Records commit: its crawl request's ID and URL, and
/// the time of its commit.
#[derive(Clone, Copy, Debug)]
pub struct Committing<'a> {
    /// The crawl request's ID, its work item's.
    pub request_id: &'a str,
    /// The crawl request's URL.
    pub url: &'a str,
    /// When the attempt commits.
    pub at: &'a PointInTime,
}

/// Commits `records`, which the attempt `committing` names staged, in
/// `connection`'s transaction: each Record takes its Dataset's next number
/// and the hash of the schema the run file declares it with. Answers, per
/// Dataset in the order first pushed, how many it committed.
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
        "INSERT INTO records (dataset, number, work_item, url, committed_at, data, schema_hash)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM records WHERE dataset = ?1), ?2, ?3, ?4, ?5,
             (SELECT schema_hash FROM datasets WHERE name = ?1))",
    )?;
    let mut counted: Vec<DatasetRecords> = Vec::new();
    for record in records.records() {
        insert.execute((
            &record.dataset,
            committing.request_id,
            committing.url,
            committing.at.as_str(),
            record.data.to_string(),
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
    Ok(counted)
}

/// Which committed Records a read takes: one Dataset's, after a number.
/// A filter on what Records carry, such as the Wall marker (ADR 0052),
/// joins here, so every reader of a Dataset filters alike.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    /// The Dataset's name.
    pub dataset: &'a str,
    /// The number of the last Record already read: 0 reads from the first.
    pub after: u64,
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
    connection
        .prepare(
            "SELECT number, data, work_item, url, committed_at FROM records
             WHERE dataset = ?1 AND number > ?2 ORDER BY number LIMIT ?3",
        )?
        .query_map(
            (
                query.dataset,
                i64::try_from(query.after).unwrap_or(i64::MAX),
                i64::try_from(most).unwrap_or(i64::MAX),
            ),
            |row| {
                let data: String = row.get(1)?;
                let value = serde_json::from_str(&data).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(DatasetRecord {
                    number: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    value,
                    request_id: row.get(2)?,
                    url: row.get(3)?,
                    committed_at: PointInTime::from_rfc3339(row.get(4)?),
                })
            },
        )?
        .collect()
}
