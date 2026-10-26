//! The SQL tab's module seam (spec #593; ADR 0062): the query connection
//! over real run files, written with the tables `goodfetch-dataset` adds
//! to every crawl's run file, read through the table functions DuckDB
//! calls, never through DuckDB's SQLite extension (ADR 0057).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod limits;
mod refusals;
mod run_files;
mod tables;
