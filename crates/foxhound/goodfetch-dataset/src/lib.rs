//! A Foxhound crawl's Datasets and their Records (ADRs 0082, 0087 and
//! 0103): the Datasets a crawl declares at its start, each with its JSON
//! Schema and its key, refused with `dataset_duplicate` or
//! `dataset_key_invalid` before the Run begins and kept in its run file;
//! the check a pushed Record passes before it is staged with its attempt
//! (`dataset_unknown`, `record_invalid`), which only a [`Checked`] value
//! proves; and each Dataset's numbering of the Records an attempt commits,
//! from 1, in commit order, with no gaps; `run.records`, the cursor that
//! reads them back, live, without slowing the crawl; and each Dataset's
//! rejection table, which a Record the check refused at push enters with
//! its attempt, and `run.rejected_records`, its reader. The crawl's
//! delivery (area 3) stages and commits; this crate says what may be
//! committed, numbers it and reads it. A resume checks the crawl's
//! declarations against those the Run began with, and a changed schema is
//! recorded for the Records committed from then on, each keeping its
//! schema's hash. Its own crate under `crates/foxhound/`, since only
//! Foxhound links it
//! (STYLE.md, reason 2).

mod declaration;
mod encoding;
mod export;
mod open;
mod page;
mod pipeline;
mod reader;
mod records;
mod rejected_records;
mod rejections;
mod resume;
mod run_tables;

use goodfetch_engine::Contribution;
use goodfetch_engine::run_state::RunStateTables;

pub use declaration::{Datasets, Declarations, Refused, counted_validator};
pub use encoding::ExportOptions;
pub use open::open_run;
pub use pipeline::write_export;
pub use records::{Checked, Committing, Query, commit, read};
pub use rejections::{Rejection, RequestUrl, fuzz_rejection};
pub use resume::Began;
pub use run_tables::{RUN_FILE_TABLES, RunFileTable};

/// The tables of a crawl's Datasets in every run file: `datasets`, each
/// declared Dataset in the order declared, with its JSON Schema and that
/// schema's hash (none for a Dataset without one) and its key as a JSON
/// array; and `records`, each committed Record by its Dataset and its
/// number, with the crawl request whose attempt committed it, that
/// request's URL, the time of the commit, the hash of the schema it was
/// committed under (none for a Dataset without one) and its Provenance as
/// JSON; and
/// `rejected_records`, each Dataset's rejection table: each rejected Record
/// by its Dataset and its number there, with the crawl request whose
/// attempt staged it, the Record as JSON and, as a JSON object, why
/// (its reason, its issues and the Hook, when one rejected it).
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-dataset",
    statements: &[
        "CREATE TABLE IF NOT EXISTS datasets (
            name TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL,
            schema TEXT,
            schema_hash TEXT,
            key TEXT NOT NULL
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            url TEXT NOT NULL,
            committed_at TEXT NOT NULL,
            data TEXT NOT NULL,
            schema_hash TEXT,
            provenance TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS rejected_records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            rejection TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT",
    ],
};

/// What Datasets add to the engine: [`TABLES`], `run.records`,
/// `run.rejected_records`, which reads each entry's URL through
/// `request_url`, the crawl's reader of its work items, and `run.export`
/// with `export.save_as` and `export.stream`.
pub fn contribution(request_url: RequestUrl) -> Contribution {
    export::handlers().into_iter().fold(
        Contribution::new("goodfetch-dataset")
            .with_run_state_tables(TABLES)
            .with_handler(reader::handler())
            .with_handler(page::handler())
            .with_handler(rejected_records::handler(request_url)),
        Contribution::with_handler,
    )
}
