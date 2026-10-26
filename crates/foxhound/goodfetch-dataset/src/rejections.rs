//! Each Dataset's rejection table (ADR 0103): the Records rejected rather
//! than committed, kept in the run file's `rejected_records` table under
//! their Dataset and their number, which counts from 1 per Dataset, in
//! commit order, with no gaps, beside the crawl request whose attempt
//! staged them, the Record as JSON and why it was rejected. A rejection is
//! staged with its attempt and written inside the write that commits it,
//! so a discarded attempt's rejections never enter the table.

use goodfetch_protocol::foxhound::dataset::RecordIssue;
use goodfetch_protocol::foxhound::outcomes::DatasetRecords;
use goodfetch_protocol::foxhound::rejections::{REJECTED_RECORDS_PAGE, RejectedRecord};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A Record rejected rather than committed, staged with its attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    /// The Dataset it was pushed into.
    pub(crate) dataset: String,
    /// The Record as pushed.
    pub(crate) data: Value,
    /// Why.
    pub(crate) why: Why,
}

/// Why a Record was rejected, as the `rejection` column keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Why {
    /// The reason, as text.
    pub(crate) reason: String,
    /// Each issue by its path.
    pub(crate) issues: Vec<RecordIssue>,
    /// The Hook that rejected it; none for the engine's own check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hook: Option<String>,
}

/// Writes `rejections`, which the attempt of the crawl request `work_item`
/// staged, in `connection`'s transaction: each takes its Dataset's next
/// number, its Record and why written as `kept` encodes them. Counts each
/// in `counted`, adding a Dataset not yet counted.
///
/// # Errors
///
/// The run file's, when a row cannot be written.
pub(crate) fn write(
    connection: &Connection,
    work_item: &str,
    rejections: &[Rejection],
    kept: &mut impl FnMut(Value) -> String,
    counted: &mut Vec<DatasetRecords>,
) -> rusqlite::Result<()> {
    if rejections.is_empty() {
        return Ok(());
    }
    let mut insert = connection.prepare(
        "INSERT INTO rejected_records (dataset, number, work_item, data, rejection)
         VALUES (?1, (SELECT coalesce(max(number), 0) + 1 FROM rejected_records WHERE dataset = ?1), ?2, ?3, ?4)",
    )?;
    for rejection in rejections {
        let why = serde_json::to_value(&rejection.why).expect("invariant: a rejection encodes");
        insert.execute((
            &rejection.dataset,
            work_item,
            kept(rejection.data.clone()),
            kept(why),
        ))?;
        match counted
            .iter_mut()
            .find(|counted| counted.dataset == rejection.dataset)
        {
            Some(counted) => counted.rejected += 1,
            None => counted.push(DatasetRecords {
                dataset: rejection.dataset.clone(),
                committed: 0,
                dropped: 0,
                rejected: 1,
            }),
        }
    }
    Ok(())
}

/// One page of a Dataset's rejection table as read: its entries after a
/// number, and the number the next page starts after.
pub(crate) struct Page {
    pub(crate) records: Vec<RejectedRecord>,
    pub(crate) next: u64,
}

/// The entries of `dataset`'s rejection table after `after`, at most a
/// page, in number order, read in one transaction. Each entry's URL is its
/// crawl request's, read from the crawl's `work_items` table.
///
/// # Errors
///
/// The run file's, when the rows cannot be read or an entry no longer
/// decodes.
pub(crate) fn page(connection: &Connection, dataset: &str, after: u64) -> rusqlite::Result<Page> {
    let transaction = connection.unchecked_transaction()?;
    let records = transaction
        .prepare(
            "SELECT rejected.number, rejected.data, rejected.rejection, rejected.work_item,
                    coalesce(work_items.url, '')
             FROM rejected_records AS rejected
             LEFT JOIN work_items ON work_items.id = rejected.work_item
             WHERE rejected.dataset = ?1 AND rejected.number > ?2
             ORDER BY rejected.number LIMIT ?3",
        )?
        .query_map(
            (
                dataset,
                integer(after),
                integer(u64::try_from(REJECTED_RECORDS_PAGE).unwrap_or(u64::MAX)),
            ),
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?
        .map(|row| {
            row.and_then(|(number, data, rejection, request_id, url)| {
                let (data, why) = decode(&data, &rejection).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(RejectedRecord {
                    number: u64::try_from(number).unwrap_or_default(),
                    data,
                    reason: why.reason,
                    issues: why.issues,
                    hook: why.hook,
                    request_id,
                    url,
                })
            })
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let last: i64 = transaction.query_row(
        "SELECT coalesce(max(number), 0) FROM rejected_records WHERE dataset = ?1",
        [dataset],
        |row| row.get(0),
    )?;
    transaction.finish()?;
    let next = match records.last() {
        Some(entry) if records.len() == REJECTED_RECORDS_PAGE => entry.number,
        _ => u64::try_from(last).unwrap_or_default().max(after),
    };
    Ok(Page { records, next })
}

/// An entry's Record and why it was rejected, from its `data` and
/// `rejection` columns.
fn decode(data: &str, rejection: &str) -> serde_json::Result<(Value, Why)> {
    Ok((
        serde_json::from_str(data)?,
        serde_json::from_str(rejection)?,
    ))
}

/// The fuzz target's body for the rejection table's JSON (ADR 0067):
/// reads `data` as an entry's `rejection` column, as the reader decodes
/// it; one that decodes encodes back to JSON that decodes to itself. It
/// panics only on a bug.
pub fn fuzz_rejection(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let Ok((_, why)) = decode("null", &text) else {
        return;
    };
    let encoded = serde_json::to_string(&why).expect("invariant: a rejection encodes");
    let (_, again) = decode("null", &encoded).expect("invariant: an encoded rejection decodes");
    assert_eq!(again, why, "a rejection survives its own encoding");
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
