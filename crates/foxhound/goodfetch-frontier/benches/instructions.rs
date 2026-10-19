//! The Frontier's instruction counts, under gungraun's Callgrind (ADR 0071).
//! The fixture scenario allocates through the system allocator; the
//! Frontier's real scenarios run with mimalloc as the global allocator, as
//! the binary ships, once area 3 adds them.

mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;

#[library_benchmark]
#[bench::thousand_urls(scenarios::synthetic_urls(1_000))]
fn queue_unique(urls: Vec<String>) -> usize {
    black_box(scenarios::queue_unique(black_box(urls)))
}

library_benchmark_group!(name = frontier, benchmarks = queue_unique);

main!(library_benchmark_groups = frontier);
