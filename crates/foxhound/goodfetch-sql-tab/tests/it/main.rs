//! The SQL tab's module seam (spec #593; ADR 0062): the query connection
//! over real run files, written with the tables `goodfetch-dataset` adds
//! to every crawl's run file, read through the table functions DuckDB
//! calls, never through DuckDB's SQLite extension (ADR 0057).
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests write run files and a file outside the runs, watch a loopback listener no query may reach, and stop a query from a thread of their own, outside any async runtime"
)]

mod limits;
mod refusals;
mod run_files;
mod tables;
