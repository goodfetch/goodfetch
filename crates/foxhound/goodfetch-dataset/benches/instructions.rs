//! The Datasets' instruction counts, under gungraun's Callgrind (ADR
//! 0071): a Dataset's commit and an Export's writing, held at +5 % by
//! `xtask perf` (spec #582's performance budgets). The scenarios allocate
//! through the system allocator.

mod scenarios;

use std::hint::black_box;

use goodfetch_protocol::foxhound::export::ExportFormat;
use gungraun::prelude::*;
use rusqlite::Connection;
use serde_json::Value;

#[library_benchmark]
#[bench::a_thousand_books(scenarios::records(1_000))]
fn commit(values: Vec<Value>) -> Connection {
    black_box(scenarios::commit_all(black_box(values)))
}

library_benchmark_group!(name = dataset, benchmarks = commit);

#[library_benchmark]
#[bench::json_of_a_thousand_books(
    scenarios::commit_all(scenarios::records(1_000)),
    ExportFormat::Json
)]
#[bench::jsonl_of_a_thousand_books(
    scenarios::commit_all(scenarios::records(1_000)),
    ExportFormat::Jsonl
)]
fn write(connection: Connection, format: ExportFormat) -> usize {
    black_box(scenarios::export(black_box(&connection), format))
}

library_benchmark_group!(name = export, benchmarks = write);

main!(library_benchmark_groups = [dataset, export]);
