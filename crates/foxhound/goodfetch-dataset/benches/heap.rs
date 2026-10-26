//! The Datasets' heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.

mod scenarios;

use std::hint::black_box;

use goodfetch_protocol::foxhound::export::ExportFormat;
use gungraun::Tool;
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

main!(
    config = LibraryBenchmarkConfig::default().default_tool(Tool::DHAT),
    library_benchmark_groups = [dataset, export]
);
