//! `dataset.erase` (ADR 0031; spec #1937, story 214) and `run.erase`
//! (ADR 0016; spec #565, story 4-120): person-only answers that apply at once
//! with no user code, on a run no engine runs. The engine answers either only
//! to a person's surface, before these handlers run. Each erase writes a
//! `run_data_erased` Audit log entry without repeating any erased personal
//! value.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use goodfetch_engine::audit::Origin;
use goodfetch_engine::durable;
use goodfetch_engine::response_cache::ResponseCacheError;
use goodfetch_engine::run::{format_refusal, run_error};
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::formats::RESPONSE_CACHE;
use goodfetch_protocol::foxhound::erase::{
    DATASET_ERASE, DatasetEraseParams, DatasetErased, RUN_DATA_ERASED, RUN_ERASE, RunEraseParams,
    RunErased,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::open::open_dataset;

/// The handler of `dataset.erase`.
pub(crate) fn handler() -> Handler {
    Handler::new(&DATASET_ERASE, erase)
}

/// The handler of `run.erase`.
pub(crate) fn run_handler() -> Handler {
    Handler::new(&RUN_ERASE, erase_run)
}

async fn erase(
    params: DatasetEraseParams,
    context: CallContext,
) -> Result<DatasetErased, CallError> {
    drop(open_dataset(&context, params.run_id.clone(), &params.dataset).await?);
    let dataset = params.dataset.clone();
    let (records, rejected_records) = context
        .runs()
        .edit(params.run_id.clone(), move |connection| {
            erase_records(connection, &dataset)
        })
        .await
        .map_err(|error| run_error(error, &context))?;
    record_audit(
        &context,
        &params.run_id,
        json!({
            "operation": DATASET_ERASE.name,
            "dataset": params.dataset,
            "records": records,
            "rejected_records": rejected_records,
        }),
    )
    .await;
    Ok(DatasetErased {
        run_id: params.run_id,
        dataset: params.dataset,
        records,
        rejected_records,
    })
}

/// Criteria for `run.erase`.
#[derive(Clone, Debug)]
struct MatchCriteria {
    value: Option<String>,
    key: Option<String>,
}

impl MatchCriteria {
    fn from_params(params: &RunEraseParams) -> Result<Self, CallError> {
        let value = params
            .value
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        let key = params
            .key
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        if value.is_none() && key.is_none() {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "run.erase requires a non-empty `value`, a non-empty `key`, or both",
            ));
        }
        Ok(Self { value, key })
    }

    /// Initial plain-text value needles from `value` alone (never the field
    /// name `key`, so `key`-only erasure never rewrites JSON property names or
    /// over-deletes unrelated rows).
    fn initial_value_needles(&self) -> BTreeSet<String> {
        let mut needles = BTreeSet::new();
        if let Some(value) = &self.value {
            needles.insert(value.clone());
        }
        needles
    }

    /// Whether a JSON Record (`data`) or its extra text matches these criteria.
    /// For `key`-only erasure, collects the actual erased personal scalar
    /// values found at `key` into `collected` so subsequent text/cache/file
    /// passes scrub those personal values rather than the field name itself.
    fn matches_record(&self, data: &str, extra: &[&str], collected: &mut BTreeSet<String>) -> bool {
        match (&self.key, &self.value) {
            (Some(key), Some(value)) => serde_json::from_str::<Value>(data).is_ok_and(|parsed| {
                value_at_path(&parsed, key).is_some_and(|v| json_contains(v, value))
            }),
            (Some(key), None) => serde_json::from_str::<Value>(data).is_ok_and(|parsed| {
                let Some(found) = value_at_path(&parsed, key) else {
                    return false;
                };
                collect_json_scalars(found, collected);
                true
            }),
            (None, Some(value)) => {
                data.contains(value.as_str())
                    || extra.iter().any(|text| text.contains(value.as_str()))
            }
            (None, None) => false,
        }
    }
}

/// Whether `text` contains any needle in `needles`.
fn matches_any_needle(text: &str, needles: &BTreeSet<String>) -> bool {
    needles
        .iter()
        .any(|needle| !needle.is_empty() && text.contains(needle.as_str()))
}

/// Whether raw `bytes` contain any needle in `needles`.
fn bytes_contain_any_needle(bytes: &[u8], needles: &BTreeSet<String>) -> bool {
    needles.iter().any(|needle| {
        let nb = needle.as_bytes();
        !nb.is_empty() && bytes.windows(nb.len()).any(|window| window == nb)
    })
}

/// Collects non-empty string and numeric scalar values inside `value` as
/// personal value needles for `key`-only erasure.
fn collect_json_scalars(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(text) if !text.is_empty() => {
            out.insert(text.clone());
        }
        Value::Number(number) => {
            out.insert(number.to_string());
        }
        Value::Array(items) => {
            for item in items {
                collect_json_scalars(item, out);
            }
        }
        Value::Object(map) => {
            for entry in map.values() {
                collect_json_scalars(entry, out);
            }
        }
        Value::String(_) | Value::Bool(_) | Value::Null => {}
    }
}

/// Looks up a field by name or dotted path (`a.b.c`) inside a JSON object.
fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

/// Whether any scalar inside `value` (or its JSON encoding) contains `needle`.
fn json_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Number(number) => number.to_string().contains(needle),
        Value::Bool(flag) => flag.to_string() == needle,
        Value::Array(items) => items.iter().any(|item| json_contains(item, needle)),
        Value::Object(map) => map.values().any(|item| json_contains(item, needle)),
        Value::Null => false,
    }
}

/// Counts and collected personal values from erasing one SQLite database
/// (`run.sqlite` or `checkpoint.sqlite`).
struct DbErased {
    records: u64,
    rejected_records: u64,
    baseline_rows: u64,
    other_entries: u64,
    value_needles: BTreeSet<String>,
}

impl DbErased {
    fn total(&self) -> u64 {
        self.records + self.rejected_records + self.baseline_rows + self.other_entries
    }
}

fn response_cache_error(error: ResponseCacheError) -> CallError {
    match error {
        ResponseCacheError::Format {
            found,
            compatibility,
        } => format_refusal(&RESPONSE_CACHE, &found, compatibility),
        other => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the Response cache could not be erased: {other}"),
        ),
    }
}

async fn erase_run(params: RunEraseParams, context: CallContext) -> Result<RunErased, CallError> {
    let criteria = MatchCriteria::from_params(&params)?;
    let run_criteria = criteria.clone();
    let checkpoint_criteria = criteria.clone();
    let folder_criteria = criteria.clone();
    let (db_erased, checkpoint_entries) = context
        .runs()
        .edit_with_checkpoint(
            params.run_id.clone(),
            move |connection| {
                let mut needles = run_criteria.initial_value_needles();
                erase_database(connection, &run_criteria, &mut needles)
            },
            move |connection, erased| {
                erase_database(connection, &checkpoint_criteria, &mut erased.value_needles)
                    .map(|ckpt| ckpt.total())
            },
            move |folder, erased| {
                scrub_run_folder_files(folder, &folder_criteria, &mut erased.value_needles)
            },
        )
        .await
        .map_err(|error| run_error(error, &context))?;
    let needles: Vec<String> = db_erased.value_needles.into_iter().collect();
    let response_cache_entries = context
        .engine()
        .response_cache()
        .erase_matching(&params.run_id, &needles)
        .await
        .map_err(response_cache_error)?;
    record_audit(
        &context,
        &params.run_id,
        json!({
            "operation": RUN_ERASE.name,
            "by_value": criteria.value.is_some(),
            "by_key": criteria.key.is_some(),
            "records": db_erased.records,
            "rejected_records": db_erased.rejected_records,
            "baseline_rows": db_erased.baseline_rows,
            "checkpoint_entries": checkpoint_entries,
            "response_cache_entries": response_cache_entries,
        }),
    )
    .await;
    Ok(RunErased {
        run_id: params.run_id,
        records: db_erased.records,
        rejected_records: db_erased.rejected_records,
        baseline_rows: db_erased.baseline_rows,
        checkpoint_entries,
        response_cache_entries,
    })
}

/// Appends a `run_data_erased` Audit log entry for `run_id`.
async fn record_audit(context: &CallContext, run_id: &RunId, details: Value) {
    let origin = Origin {
        actor: Actor::Person {
            name: context.engine().person().to_owned(),
        },
        surface: Some(context.surface()),
        layer: None,
        run_id: Some(run_id.clone()),
    };
    if let Err(error) = context
        .engine()
        .audit()
        .record(
            &origin,
            &RUN_DATA_ERASED,
            details,
            context.engine().clock().timestamp(),
        )
        .await
    {
        tracing::error!(
            %error,
            %run_id,
            "the erasure was not recorded in the Audit log"
        );
    }
}

/// Whether a table named `table` exists in `connection`.
fn has_table(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")?
        .exists([table])
}

/// Erases every matching Record, rejected Record, Baseline row and run-state
/// field from `connection` (`run.sqlite` or `checkpoint.sqlite`), updating
/// `value_needles` with any erased personal values collected under `key`.
fn erase_database(
    connection: &Connection,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> rusqlite::Result<DbErased> {
    let has_search = has_table(connection, "record_search")?;
    let mut secured_search = false;
    let mut records = 0_u64;
    if has_table(connection, "records")? {
        let matched: Vec<(String, i64)> = {
            let mut select = connection
                .prepare_cached("SELECT dataset, number, url, data, provenance FROM records")?;
            let rows = select.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            let mut matched = Vec::new();
            for row in rows {
                let (dataset, number, url, data, provenance) = row?;
                if criteria.matches_record(&data, &[&url, &provenance], value_needles) {
                    matched.push((dataset, number));
                }
            }
            matched
        };
        if !matched.is_empty() {
            if has_search && !secured_search {
                connection
                    .prepare_cached(
                        "INSERT INTO record_search(record_search, rank) VALUES ('secure-delete', 1)",
                    )?
                    .execute([])?;
                secured_search = true;
            }
            let mut delete_record = connection
                .prepare_cached("DELETE FROM records WHERE dataset = ?1 AND number = ?2")?;
            let mut delete_search = if has_search {
                Some(connection.prepare_cached(
                    "DELETE FROM record_search WHERE dataset = ?1 AND number = ?2",
                )?)
            } else {
                None
            };
            for (dataset, number) in matched {
                if let Some(delete_search) = &mut delete_search {
                    delete_search.execute((&dataset, number))?;
                }
                records += count(delete_record.execute((&dataset, number))?);
            }
        }
    }
    let _ = secured_search;

    let mut rejected_records = 0_u64;
    if has_table(connection, "rejected_records")? {
        let matched: Vec<(String, i64)> = {
            let mut select = connection
                .prepare_cached("SELECT dataset, number, data, rejection FROM rejected_records")?;
            let rows = select.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            let mut matched = Vec::new();
            for row in rows {
                let (dataset, number, data, rejection) = row?;
                if criteria.matches_record(&data, &[&rejection], value_needles) {
                    matched.push((dataset, number));
                }
            }
            matched
        };
        if !matched.is_empty() {
            let mut delete = connection.prepare_cached(
                "DELETE FROM rejected_records WHERE dataset = ?1 AND number = ?2",
            )?;
            for (dataset, number) in matched {
                rejected_records += count(delete.execute((&dataset, number))?);
            }
        }
    }

    let baseline_rows = erase_baselines(connection, criteria, value_needles)?;
    let other_entries = scrub_run_tables(connection, criteria, value_needles)?;
    Ok(DbErased {
        records,
        rejected_records,
        baseline_rows,
        other_entries,
        value_needles: value_needles.clone(),
    })
}

/// Erases matching rows from any `baselines` table if present.
fn erase_baselines(
    connection: &Connection,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> rusqlite::Result<u64> {
    if !has_table(connection, "baselines")? {
        return Ok(0);
    }
    let columns: Vec<String> = {
        let mut stmt = connection.prepare("PRAGMA table_info(baselines)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if columns.is_empty() {
        return Ok(0);
    }
    let select_cols = columns
        .iter()
        .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!("SELECT rowid, {select_cols} FROM baselines");
    let matched: Vec<i64> = {
        let mut stmt = connection.prepare(&select_sql)?;
        let mut rows = stmt.query([])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let mut hit = false;
            for idx in 1..=columns.len() {
                if let Ok(Some(text)) = row.get::<_, Option<String>>(idx)
                    && (criteria.matches_record(&text, &[], value_needles)
                        || matches_any_needle(&text, value_needles))
                {
                    hit = true;
                    break;
                }
            }
            if hit {
                ids.push(rowid);
            }
        }
        ids
    };
    let mut deleted = 0_u64;
    if !matched.is_empty() {
        let mut delete = connection.prepare_cached("DELETE FROM baselines WHERE rowid = ?1")?;
        for rowid in matched {
            deleted += count(delete.execute([rowid])?);
        }
    }
    Ok(deleted)
}

fn scrub_key_values(
    connection: &Connection,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> rusqlite::Result<u64> {
    if !has_table(connection, "key_values")? {
        return Ok(0);
    }
    let matched: Vec<String> = {
        let mut stmt = connection.prepare_cached("SELECT key, value, content FROM key_values")?;
        let rows = stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let value: Option<String> = row.get(1)?;
            let content: Option<Vec<u8>> = row.get(2)?;
            Ok((key, value, content))
        })?;
        let mut keys = Vec::new();
        for row in rows {
            let (key, value, content) = row?;
            let exact_key = criteria.key.as_deref() == Some(key.as_str());
            if exact_key
                && criteria.value.is_none()
                && let Some(val_text) = value.as_deref()
                && !val_text.is_empty()
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(val_text) {
                    collect_json_scalars(&parsed, value_needles);
                } else {
                    value_needles.insert(val_text.to_owned());
                }
            }
            let hit = exact_key
                || matches_any_needle(&key, value_needles)
                || value.as_deref().is_some_and(|t| {
                    criteria.matches_record(t, &[], value_needles)
                        || matches_any_needle(t, value_needles)
                })
                || content
                    .as_deref()
                    .is_some_and(|bytes| bytes_contain_any_needle(bytes, value_needles));
            if hit {
                keys.push(key);
            }
        }
        keys
    };
    let mut scrubbed = 0_u64;
    if !matched.is_empty() {
        let mut delete = connection.prepare_cached("DELETE FROM key_values WHERE key = ?1")?;
        for key in matched {
            scrubbed += count(delete.execute([&key])?);
        }
    }
    Ok(scrubbed)
}

/// Scrubs matching fields/rows across all other run-state and Checkpoint
/// tables (`key_values`, `run_file_chunks`, `run`, `work_items`, `outcomes`,
/// `seeds`, `visited`, `warnings`, etc.) so no copy of an erased value remains
/// in `run.sqlite` or `checkpoint.sqlite`.
fn scrub_run_tables(
    connection: &Connection,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> rusqlite::Result<u64> {
    if value_needles.is_empty() && criteria.key.is_none() {
        return Ok(0);
    }
    let mut scrubbed = scrub_key_values(connection, criteria, value_needles)?;
    if !value_needles.is_empty() && has_table(connection, "run_file_chunks")? {
        let matched: Vec<(String, i64)> = {
            let mut stmt =
                connection.prepare_cached("SELECT sha256, offset, data FROM run_file_chunks")?;
            let mapped = stmt.query_map([], |row| {
                let sha256: String = row.get(0)?;
                let offset: i64 = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let hit = bytes_contain_any_needle(&data, value_needles);
                Ok(hit.then_some((sha256, offset)))
            })?;
            let mut hits = Vec::new();
            for row in mapped {
                if let Some(hit) = row? {
                    hits.push(hit);
                }
            }
            hits
        };
        if !matched.is_empty() {
            let mut delete = connection
                .prepare_cached("DELETE FROM run_file_chunks WHERE sha256 = ?1 AND offset = ?2")?;
            for (sha256, offset) in matched {
                scrubbed += count(delete.execute((&sha256, offset))?);
            }
        }
    }
    let tables: Vec<String> = {
        let mut stmt = connection.prepare_cached(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND sql NOT LIKE 'CREATE VIRTUAL TABLE%'",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for table in tables {
        if table.starts_with("sqlite_")
            || table.starts_with("record_search")
            || matches!(
                table.as_str(),
                "format"
                    | "run_key"
                    | "datasets"
                    | "records"
                    | "rejected_records"
                    | "baselines"
                    | "key_values"
                    | "run_file_chunks"
            )
        {
            continue;
        }
        scrubbed += scrub_generic_table(connection, &table, criteria, value_needles)?;
    }
    Ok(scrubbed)
}

struct TableCol {
    name: String,
    is_text: bool,
    pk: i64,
}

struct RowUpdate {
    pk_vals: Vec<rusqlite::types::Value>,
    updates: Vec<(usize, String)>,
}

fn load_table_cols(connection: &Connection, table: &str) -> rusqlite::Result<Vec<TableCol>> {
    let mut stmt = connection.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(1)?;
        let col_type: String = row.get(2)?;
        let pk: i64 = row.get(5)?;
        Ok(TableCol {
            name,
            is_text: col_type.eq_ignore_ascii_case("TEXT"),
            pk,
        })
    })?;
    rows.collect()
}

/// Scrubs matching text columns in `table` (updating non-PK text columns in
/// place, or deleting rows whose primary key itself matches a value needle).
fn scrub_generic_table(
    connection: &Connection,
    table: &str,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> rusqlite::Result<u64> {
    let cols = load_table_cols(connection, table)?;
    let text_cols: Vec<&TableCol> = cols.iter().filter(|c| c.is_text).collect();
    if text_cols.is_empty() {
        return Ok(0);
    }
    let mut pk_cols: Vec<&TableCol> = cols.iter().filter(|c| c.pk > 0).collect();
    pk_cols.sort_by_key(|c| c.pk);
    let use_rowid = pk_cols.is_empty();
    let pk_names: Vec<&str> = if use_rowid {
        vec!["rowid"]
    } else {
        pk_cols.iter().map(|c| c.name.as_str()).collect()
    };
    let non_pk_text: Vec<&str> = text_cols
        .iter()
        .filter(|c| c.pk == 0)
        .map(|c| c.name.as_str())
        .collect();
    let quote = |name: &str| {
        if name == "rowid" && use_rowid {
            "rowid".to_owned()
        } else {
            format!("\"{name}\"")
        }
    };
    let quoted_select = pk_names
        .iter()
        .chain(non_pk_text.iter())
        .map(|name| quote(name))
        .collect::<Vec<_>>()
        .join(", ");
    let where_pk = pk_names
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{} = ?{}", quote(name), i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let pk_len = pk_names.len();

    let mut to_delete: Vec<Vec<rusqlite::types::Value>> = Vec::new();
    let mut to_update: Vec<RowUpdate> = Vec::new();
    {
        let mut stmt = connection.prepare(&format!("SELECT {quoted_select} FROM \"{table}\""))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let mut pk_vals = Vec::with_capacity(pk_len);
            let mut pk_matched = false;
            for i in 0..pk_len {
                let val: rusqlite::types::Value = row.get(i)?;
                if let rusqlite::types::Value::Text(ref text) = val
                    && matches_any_needle(text, value_needles)
                {
                    pk_matched = true;
                }
                pk_vals.push(val);
            }
            if pk_matched && table != "run" {
                to_delete.push(pk_vals);
                continue;
            }
            let mut updates = Vec::new();
            for (offset, _) in non_pk_text.iter().enumerate() {
                if let Ok(Some(text)) = row.get::<_, Option<String>>(pk_len + offset)
                    && let Some(scrubbed) = scrub_text_cell(&text, criteria, value_needles)
                    && scrubbed != text
                {
                    updates.push((offset, scrubbed));
                }
            }
            if !updates.is_empty() {
                to_update.push(RowUpdate { pk_vals, updates });
            }
        }
    }
    apply_generic_scrub(
        connection,
        table,
        &where_pk,
        &non_pk_text,
        to_delete,
        to_update,
    )
}

fn apply_generic_scrub(
    connection: &Connection,
    table: &str,
    where_pk: &str,
    non_pk_text: &[&str],
    to_delete: Vec<Vec<rusqlite::types::Value>>,
    to_update: Vec<RowUpdate>,
) -> rusqlite::Result<u64> {
    let mut changed = 0_u64;
    if !to_delete.is_empty() {
        let mut delete =
            connection.prepare(&format!("DELETE FROM \"{table}\" WHERE {where_pk}"))?;
        for pk_vals in to_delete {
            let params: Vec<&dyn rusqlite::ToSql> =
                pk_vals.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
            changed += count(delete.execute(params.as_slice())?);
        }
    }
    for RowUpdate { pk_vals, updates } in to_update {
        let pk_len = pk_vals.len();
        let set_clause = updates
            .iter()
            .enumerate()
            .map(|(idx, (col_offset, _))| {
                format!("\"{}\" = ?{}", non_pk_text[*col_offset], pk_len + idx + 1)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("UPDATE \"{table}\" SET {set_clause} WHERE {where_pk}");
        let mut stmt = connection.prepare(&sql)?;
        let mut params: Vec<&dyn rusqlite::ToSql> =
            pk_vals.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        for (_, new_val) in &updates {
            params.push(new_val);
        }
        changed += count(stmt.execute(params.as_slice())?);
    }
    Ok(changed)
}

/// Scrubs any erased values or fields from auxiliary files inside the Run's
/// folder (`trace/`, `files/`, and `bodies/`), using atomic replacement while
/// the Run's writer lock is held and propagating any I/O failure.
fn scrub_run_folder_files(
    folder: &Path,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> io::Result<()> {
    if value_needles.is_empty() && criteria.key.is_none() {
        return Ok(());
    }
    for sub in ["trace", "files", "bodies"] {
        scrub_dir_recursive(&folder.join(sub), criteria, value_needles)?;
    }
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "runs under the Run writer lock on the engine owner's blocking policy, outside any Shared path"
)]
fn scrub_dir_recursive(
    dir: &Path,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scrub_dir_recursive(&path, criteria, value_needles)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(&path)?;
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if let Some(scrubbed) = scrub_utf8_document(text, criteria, value_needles)
                    && scrubbed != text
                {
                    durable::replace(&path, scrubbed.as_bytes())?;
                }
            } else if !value_needles.is_empty() && bytes_contain_any_needle(&bytes, value_needles) {
                std::fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

/// Scrubs a UTF-8 file (whole JSON document, JSONL lines, or plain text),
/// returning `Some(updated)` only when content changed. Never rewrites JSON
/// property names.
fn scrub_utf8_document(
    text: &str,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> Option<String> {
    if let Ok(mut parsed) = serde_json::from_str::<Value>(text) {
        return scrub_json_value(&mut parsed, criteria, value_needles)
            .then(|| serde_json::to_string(&parsed).ok())
            .flatten();
    }
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for chunk in text.split_inclusive('\n') {
        let (line, ending) = if let Some(stripped) = chunk.strip_suffix("\r\n") {
            (stripped, "\r\n")
        } else if let Some(stripped) = chunk.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (chunk, "")
        };
        if let Some(scrubbed) = scrub_text_cell(line, criteria, value_needles) {
            if scrubbed != line {
                changed = true;
            }
            out.push_str(&scrubbed);
        } else {
            out.push_str(line);
        }
        out.push_str(ending);
    }
    changed.then_some(out)
}

/// Scrubs one text cell or JSONL line: parses JSON structurally (removing
/// matching dotted `key` paths and scrubbing `value_needles` inside string
/// values without touching property names), or scrubs `value_needles` in
/// plain text when the cell is not JSON.
fn scrub_text_cell(
    text: &str,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> Option<String> {
    if let Ok(mut parsed) = serde_json::from_str::<Value>(text) {
        return scrub_json_value(&mut parsed, criteria, value_needles)
            .then(|| serde_json::to_string(&parsed).ok())
            .flatten();
    }
    if matches_any_needle(text, value_needles) {
        Some(scrub_plain_text(text, value_needles))
    } else {
        None
    }
}

/// Removes the field at dotted `segments` inside `value` when it matches
/// `filter_value` (or unconditionally when `filter_value` is `None`,
/// collecting any scalar values removed into `value_needles`).
fn remove_at_path(
    value: &mut Value,
    segments: &[&str],
    filter_value: Option<&str>,
    value_needles: &mut BTreeSet<String>,
) -> bool {
    let Some((first, rest)) = segments.split_first() else {
        return false;
    };
    let Some(map) = value.as_object_mut() else {
        return false;
    };
    if rest.is_empty() {
        let should_remove = map
            .get(*first)
            .is_some_and(|existing| filter_value.is_none_or(|val| json_contains(existing, val)));
        if should_remove && let Some(removed) = map.remove(*first) {
            if filter_value.is_none() {
                collect_json_scalars(&removed, value_needles);
            }
            return true;
        }
        return false;
    }
    let Some(child) = map.get_mut(*first) else {
        return false;
    };
    remove_at_path(child, rest, filter_value, value_needles)
}

fn scrub_json_value(
    value: &mut Value,
    criteria: &MatchCriteria,
    value_needles: &mut BTreeSet<String>,
) -> bool {
    match value {
        Value::Object(_) => {
            let mut changed = false;
            if let Some(key) = &criteria.key {
                let segments: Vec<&str> = key.split('.').collect();
                if remove_at_path(value, &segments, criteria.value.as_deref(), value_needles) {
                    changed = true;
                }
            }
            if let Value::Object(map) = value {
                for entry in map.values_mut() {
                    if scrub_json_value(entry, criteria, value_needles) {
                        changed = true;
                    }
                }
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                if scrub_json_value(item, criteria, value_needles) {
                    changed = true;
                }
            }
            changed
        }
        Value::String(text) if matches_any_needle(text, value_needles) => {
            *text = scrub_plain_text(text, value_needles);
            true
        }
        _ => false,
    }
}

fn scrub_plain_text(text: &str, needles: &BTreeSet<String>) -> String {
    let mut out = text.to_owned();
    for needle in needles {
        if !needle.is_empty() {
            out = out.replace(needle.as_str(), "[erased]");
        }
    }
    out
}

/// Deletes the Records of `dataset`, committed and rejected, and its rows
/// of the full-text index, and answers how many of each it deleted.
/// The index's `secure-delete` option, which persists in the run file,
/// has its deleted rows' tokens removed rather than marked.
fn erase_records(connection: &Connection, dataset: &str) -> rusqlite::Result<(u64, u64)> {
    connection
        .prepare_cached(
            "INSERT INTO record_search(record_search, rank) VALUES ('secure-delete', 1)",
        )?
        .execute([])?;
    connection
        .prepare_cached("DELETE FROM record_search WHERE dataset = ?1")?
        .execute([dataset])?;
    let records = connection
        .prepare_cached("DELETE FROM records WHERE dataset = ?1")?
        .execute([dataset])?;
    let rejected = connection
        .prepare_cached("DELETE FROM rejected_records WHERE dataset = ?1")?
        .execute([dataset])?;
    Ok((count(records), count(rejected)))
}

/// A count of rows SQLite answers, which fits.
fn count(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
