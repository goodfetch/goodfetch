//! The Records a crawl commits (ADRs 0008 and 0103): each kept in the run
//! file's `records` table under its Dataset and its number, which counts
//! from 1 per Dataset, in commit order, with no gaps, beside the crawl
//! request whose attempt committed it. Only Records that passed their
//! Dataset's check, a [`Checked`] value, are committed, inside the write
//! that commits their attempt.

use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::outcomes::DatasetRecords;
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

/// Commits `records`, which the attempt of the crawl request `work_item`
/// staged, in `connection`'s transaction: each Record takes its Dataset's
/// next number and the hash of the schema the run file declares it with. Answers, per Dataset in the order first pushed, how many
/// it committed.
///
/// # Errors
///
/// The run file's, when a row cannot be written.
pub fn commit(
    connection: &Connection,
    work_item: &str,
    records: &Checked,
) -> rusqlite::Result<Vec<DatasetRecords>> {
    let mut insert = connection.prepare(
        "INSERT INTO records (dataset, number, work_item, data, schema_hash)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM records WHERE dataset = ?1), ?2, ?3,
             (SELECT schema_hash FROM datasets WHERE name = ?1))",
    )?;
    let mut counted: Vec<DatasetRecords> = Vec::new();
    for record in records.records() {
        insert.execute((&record.dataset, work_item, record.data.to_string()))?;
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
