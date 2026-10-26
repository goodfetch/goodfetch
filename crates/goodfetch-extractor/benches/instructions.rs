//! The extractor's instruction counts, under gungraun's Callgrind with
//! mimalloc as the global allocator, as the executables ship (ADR 0071):
//! `formats` over a 1 MiB page with every format asked, held at +5 % by
//! `xtask perf` (spec #580's performance budgets).

mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[library_benchmark]
#[bench::one_mebibyte_every_format(scenarios::page())]
fn every_format(html: String) -> usize {
    black_box(scenarios::read_every_format(black_box(&html)))
}

library_benchmark_group!(name = formats, benchmarks = every_format);

main!(library_benchmark_groups = formats);
