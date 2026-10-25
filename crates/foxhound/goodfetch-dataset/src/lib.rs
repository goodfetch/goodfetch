//! A Foxhound crawl's Datasets and their Records (ADRs 0082, 0087 and
//! 0103): the Datasets a crawl declares at its start, each with its JSON
//! Schema and its key, refused with `dataset_duplicate` or
//! `dataset_key_invalid` before the Run begins and kept in its run file;
//! the check a pushed Record passes before it is staged with its attempt
//! (`dataset_unknown`, `record_invalid`), which only a [`Checked`] value
//! proves; and each Dataset's numbering of the Records an attempt commits,
//! from 1, in commit order, with no gaps. The crawl's delivery (area 3)
//! stages and commits; this crate says what may be committed and numbers
//! it. Its own crate under `crates/foxhound/`, since only Foxhound links it
//! (STYLE.md, reason 2).

mod declaration;
mod records;

use goodfetch_engine::Contribution;
use goodfetch_engine::run_state::RunStateTables;

pub use declaration::Datasets;
pub use records::{Checked, commit};

/// The tables of a crawl's Datasets in every run file: `datasets`, each
/// declared Dataset in the order declared, with its JSON Schema and that
/// schema's hash (none for a Dataset without one) and its key as a JSON
/// array; and `records`, each committed Record by its Dataset and its
/// number, with the crawl request whose attempt committed it.
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
            data TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT",
    ],
};

/// What Datasets add to the engine: [`TABLES`].
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-dataset").with_run_state_tables(TABLES)
}
