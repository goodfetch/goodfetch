//! The extractor's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): `formats` over a 1 MiB page with every format
//! asked, held at +5 % by `xtask perf` (spec #580's performance budgets).

mod scenarios;

use std::hint::black_box;

use gungraun::Tool;
use gungraun::prelude::*;

#[library_benchmark]
#[bench::one_mebibyte_every_format(scenarios::page())]
fn every_format(html: String) -> usize {
    black_box(scenarios::read_every_format(black_box(&html)))
}

library_benchmark_group!(name = formats, benchmarks = every_format);

main!(
    config = LibraryBenchmarkConfig::default().default_tool(Tool::DHAT),
    library_benchmark_groups = formats
);
