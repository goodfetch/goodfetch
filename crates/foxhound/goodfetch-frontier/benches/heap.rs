//! The Frontier's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.

mod scenarios;

use std::hint::black_box;

use gungraun::Tool;
use gungraun::prelude::*;

#[library_benchmark]
#[bench::thousand_urls(scenarios::synthetic_urls(1_000))]
fn queue_unique(urls: Vec<String>) -> usize {
    black_box(scenarios::queue_unique(black_box(urls)))
}

library_benchmark_group!(name = frontier, benchmarks = queue_unique);

main!(
    config = LibraryBenchmarkConfig::default().default_tool(Tool::DHAT),
    library_benchmark_groups = frontier
);
