//! Real run files for the tests: the tables `goodfetch-dataset` adds to a
//! crawl's run file, holding Records, a rejection and a declared Dataset
//! as a crawl commits them, and a `run` table whose inputs hold a Secret, as the
//! engine's run table does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_sql_tab::{Limits, OpenRun, Sandbox};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

/// The default limits: 512 MiB and 10,000 rows.
pub(crate) const LIMITS: Limits = Limits {
    memory: 512 * 1024 * 1024,
    rows: 10_000,
};

/// A run file of `records`, each a `books` Record, at `folder/<run>.sqlite`,
/// whose run table's inputs hold `secret`.
pub(crate) fn run_file(folder: &Path, run: &str, records: &[Value], secret: &str) -> PathBuf {
    let path = folder.join(format!("{run}.sqlite"));
    let mut connection = Connection::open(&path).expect("the run file opens");
    let connection = connection.transaction().expect("one transaction writes it");
    for statement in goodfetch_dataset::TABLES.statements {
        connection
            .execute_batch(statement)
            .expect("the Datasets' tables are created");
    }
    connection
        .execute_batch("CREATE TABLE run (id TEXT NOT NULL, inputs TEXT NOT NULL)")
        .expect("the run table is created");
    connection
        .execute(
            "INSERT INTO run (id, inputs) VALUES (?1, ?2)",
            (run, json!({ "proxy": { "password": secret } }).to_string()),
        )
        .expect("the run row is written");
    connection
        .execute(
            "INSERT INTO datasets (name, position, schema, schema_hash, key) VALUES ('books', 0, NULL, ?1, '[]')",
            [secret],
        )
        .expect("the Dataset is declared");
    for (index, value) in records.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO records (dataset, number, work_item, url, committed_at, data, schema_hash, provenance) \
                 VALUES ('books', ?1, ?2, ?3, '2026-10-26T12:00:00Z', ?4, ?5, ?6)",
                (
                    i64::try_from(index + 1).expect("a small number"),
                    format!("request-{}", index + 1),
                    format!("https://shop.test/{}", index + 1),
                    value.to_string(),
                    secret,
                    json!({ "request_id": format!("request-{}", index + 1) }).to_string(),
                ),
            )
            .expect("the Record is committed");
    }
    connection
        .execute(
            "INSERT INTO rejected_records (dataset, number, work_item, data, rejection) VALUES ('books', 1, 'request-9', '{\"title\": 1}', '{\"reason\": \"schema\"}')",
            [],
        )
        .expect("a rejection is written");
    connection.commit().expect("the run file is committed");
    path
}

/// What opens `path` read-only, as the engine's Run reader does.
pub(crate) fn opener(path: &Path) -> OpenRun {
    let path = path.to_owned();
    Arc::new(move || {
        Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map(Some)
    })
}

/// A query connection over `runs`, each a Run ID and its run file.
pub(crate) fn sandbox(runs: &[(&str, &Path)], limits: Limits) -> Sandbox {
    Sandbox::open(
        runs.iter()
            .map(|(run, path)| ((*run).to_owned(), opener(path))),
        limits,
    )
    .expect("the query connection opens")
}

/// The `books` Record of page `page`.
pub(crate) fn book(page: u64) -> Value {
    json!({ "page": page, "title": format!("Café {page}") })
}
