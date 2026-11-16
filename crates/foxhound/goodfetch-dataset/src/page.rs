//! `dataset.page` (spec #593, story 205; ADR 0031): one page of a
//! Dataset's committed Records, paged by key, with filtering (the `wall`
//! marker of ADR 0052 included), sorting and column selection done here in
//! the engine, so a Client reads one page whatever the Dataset's size. Read
//! from the crawl's committed state through the engine's Run reader, never
//! its writer. Every page reads the Records numbered through the `through`
//! its first page answered, and goes on after the previous page's last
//! Record by its sort key, which never changes once committed, so Records
//! committed meanwhile never make a page repeat or skip one.

use std::fmt::Write as _;

use goodfetch_engine::run::{run_error, through_owner};
use goodfetch_engine::runs::RunError;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::dataset::DatasetRecord;
use goodfetch_protocol::foxhound::dataset_view::{
    DATASET_PAGE, DATASET_PAGE_DEFAULT, DATASET_PAGE_MOST, DatasetPage, DatasetPageParams,
    FieldCondition, RecordFilter, RecordSort, SortDirection, WallFilter,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use rusqlite::Connection;
use rusqlite::types::Value as Sql;
use serde_json::Value;

use crate::open::open_dataset;
use crate::records::{RECORD_COLUMNS, dataset_record, last_number, wall_condition};

/// The handler of `dataset.page`.
pub(crate) fn handler() -> Handler {
    Handler::new(&DATASET_PAGE, page)
}

async fn page(params: DatasetPageParams, context: CallContext) -> Result<DatasetPage, CallError> {
    if let Some(answer) = through_owner(&context, &DATASET_PAGE, &params.run_id, &params).await? {
        return Ok(answer);
    }
    let limit = params.limit.unwrap_or(DATASET_PAGE_DEFAULT);
    if !(1..=DATASET_PAGE_MOST).contains(&limit) {
        return Err(refused(format!(
            "`limit` is {limit}: a page holds from 1 to {DATASET_PAGE_MOST} Records"
        )));
    }
    let filter = params.filter.unwrap_or_default();
    let named = filter
        .fields
        .iter()
        .map(|condition| condition.field.as_str())
        .chain(params.sort.iter().map(|sort| sort.field.as_str()));
    for field in named {
        if field_path(field).is_none() {
            return Err(refused(format!(
                "the field `{field}` holds `\"`, which a filter or a sort cannot name"
            )));
        }
    }
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = open_dataset(&context, params.run_id.clone(), &params.dataset).await?;
    let (dataset, after, through, sort) =
        (params.dataset, params.after, params.through, params.sort);
    let (through, mut records) = reader
        .read(move |connection| {
            let through = match through {
                Some(through) => through,
                None => last_number(connection, &dataset)?,
            };
            let paging = Paging {
                dataset: &dataset,
                after,
                through,
                filter: &filter,
                sort: sort.as_ref(),
                limit,
            };
            Ok((through, read_page(connection, paging)?))
        })
        .await
        .map_err(failed)?;
    if let Some(columns) = &params.columns {
        for record in &mut records {
            keep_columns(record, columns);
        }
    }
    let full = records.len() == usize::try_from(limit).unwrap_or(usize::MAX);
    let next = records.last().filter(|_| full).map(|record| record.number);
    Ok(DatasetPage {
        records,
        through,
        next,
    })
}

/// `invalid_params`, saying why.
fn refused(message: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, message)
}

/// Keeps only `columns` of `record`'s value, when it is a JSON object.
fn keep_columns(record: &mut DatasetRecord, columns: &[String]) {
    if let Value::Object(fields) = &mut record.value {
        fields.retain(|field, _| columns.iter().any(|column| column == field));
    }
}

/// One page of a Dataset's committed Records: those numbered through
/// `through` that meet `filter`, in the order of `sort`, after the Record
/// numbered `after` in that order.
#[derive(Clone, Copy, Debug)]
struct Paging<'a> {
    dataset: &'a str,
    after: Option<u64>,
    through: u64,
    filter: &'a RecordFilter,
    sort: Option<&'a RecordSort>,
    limit: u32,
}

/// The JSON path of the top-level field `field`, for SQLite's
/// `json_extract`; none for a name holding `"`, which no path quotes.
fn field_path(field: &str) -> Option<String> {
    (!field.contains('"')).then(|| format!("$.\"{field}\""))
}

/// The page `paging` names, in its order. The sort key of the previous
/// page's last Record is read from that Record, so the page goes on from
/// it by key; Records that lack the sort's field come last, then by number.
fn read_page(connection: &Connection, paging: Paging<'_>) -> rusqlite::Result<Vec<DatasetRecord>> {
    let number = |value: u64| Sql::Integer(i64::try_from(value).unwrap_or(i64::MAX));
    let path = |field: &str| Sql::Text(field_path(field).unwrap_or_default());
    let mut bound = vec![Sql::Text(paging.dataset.to_owned()), number(paging.through)];
    let mut sql =
        format!("SELECT {RECORD_COLUMNS} FROM records WHERE dataset = ?1 AND number <= ?2");
    bound.push(match paging.filter.wall {
        Some(WallFilter::With) => Sql::Integer(1),
        Some(WallFilter::Without) => Sql::Integer(0),
        None => Sql::Null,
    });
    write!(sql, " AND {}", wall_condition(bound.len()))
        .expect("invariant: a String takes any text");
    for condition in &paging.filter.fields {
        bound.push(path(&condition.field));
        let field = bound.len();
        match &condition.condition {
            FieldCondition::Equals { value } => {
                bound.push(Sql::Text(value.to_string()));
                let value = bound.len();
                write!(
                    sql,
                    " AND json_extract(data, ?{field}) = json_extract(?{value}, '$')"
                )
                .expect("invariant: a String takes any text");
            }
            FieldCondition::Contains { text } => {
                bound.push(Sql::Text(text.clone()));
                let text = bound.len();
                write!(sql, " AND json_type(data, ?{field}) = 'text' AND instr(json_extract(data, ?{field}), ?{text}) > 0").expect("invariant: a String takes any text");
            }
        }
    }
    let order = match paging.sort {
        None => {
            if let Some(after) = paging.after {
                bound.push(number(after));
                write!(sql, " AND number > ?{}", bound.len())
                    .expect("invariant: a String takes any text");
            }
            "number".to_owned()
        }
        Some(sort) => {
            bound.push(path(&sort.field));
            let key = format!("json_extract(data, ?{})", bound.len());
            let (beyond, direction) = match sort.direction {
                SortDirection::Ascending => (">", "ASC"),
                SortDirection::Descending => ("<", "DESC"),
            };
            if let Some(after) = paging.after {
                bound.push(number(after));
                let after = bound.len();
                let last =
                    format!("(SELECT {key} FROM records WHERE dataset = ?1 AND number = ?{after})");
                write!(sql, " AND CASE WHEN {last} IS NULL THEN ({key} IS NULL AND number > ?{after}) \
                     ELSE ({key} IS NULL OR {key} {beyond} {last} OR ({key} = {last} AND number > ?{after})) END").expect("invariant: a String takes any text");
            }
            format!("({key} IS NULL), {key} {direction}, number")
        }
    };
    bound.push(Sql::Integer(i64::from(paging.limit)));
    write!(sql, " ORDER BY {order} LIMIT ?{}", bound.len())
        .expect("invariant: a String takes any text");
    connection
        .prepare_cached(&sql)?
        .query_map(rusqlite::params_from_iter(bound), dataset_record)?
        .collect()
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
mod tests {
    use goodfetch_protocol::foxhound::dataset_view::{RecordFilter, WallFilter};
    use rusqlite::Connection;

    use super::{Paging, read_page};

    /// A run file holding `pages`' Records 1 to 3, the second read past a
    /// paywall: no crawl marks a Wall yet (ADR 0052), so the filter is
    /// checked on the run file's rows.
    fn run_file() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        for statement in crate::TABLES.statements {
            connection.execute(statement, []).unwrap();
        }
        for (number, provenance) in [
            (1, r#"{"request_id":"1"}"#),
            (2, r#"{"request_id":"2","wall":{"kind":"paywall"}}"#),
            (3, r#"{"request_id":"3"}"#),
        ] {
            connection
                .execute(
                    "INSERT INTO records VALUES ('pages', ?1, ?1, 'https://shop.test/', '2026-01-05T09:00:00Z', '{}', NULL, ?2)",
                    (number, provenance),
                )
                .unwrap();
        }
        connection
    }

    /// The numbers of the page that `wall` filters.
    fn numbers(wall: Option<WallFilter>) -> Vec<u64> {
        let filter = RecordFilter {
            fields: Vec::new(),
            wall,
        };
        let paging = Paging {
            dataset: "pages",
            after: None,
            through: 3,
            filter: &filter,
            sort: None,
            limit: 10,
        };
        read_page(&run_file(), paging)
            .unwrap()
            .iter()
            .map(|record| record.number)
            .collect()
    }

    /// The `wall` filter takes only the Records whose Provenance carries
    /// the Wall marker, or only those whose does not.
    ///
    /// Covers: story:12-205
    #[test]
    fn the_wall_filter_takes_records_by_the_wall_marker_their_provenance_carries() {
        assert_eq!(
            (
                numbers(Some(WallFilter::With)),
                numbers(Some(WallFilter::Without)),
                numbers(None)
            ),
            (vec![2], vec![1, 3], vec![1, 2, 3])
        );
    }
}
