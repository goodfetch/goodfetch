//! The Frontier's instruction counts, under gungraun's Callgrind (ADR 0071):
//! admission through the per-domain queues. The scenario allocates through
//! the system allocator; mimalloc, the allocator the binary ships, joins
//! the Frontier's scenarios once its other parts arrive.

mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;

#[library_benchmark]
#[bench::one_heavy_domain_and_a_thousand_small(scenarios::requests(64, 1_000))]
fn admit(requests: Vec<(String, i64)>) -> usize {
    black_box(scenarios::admit_all(black_box(requests)))
}

library_benchmark_group!(name = admission, benchmarks = admit);

main!(library_benchmark_groups = admission);
