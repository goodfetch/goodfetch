//! The fetch tiers' instruction counts, under gungraun's Callgrind (ADR
//! 0071): the decoding order of ADR 0119 over a pinned corpus, held at
//! +5 % by `xtask perf` (spec #579's performance budgets). The scenarios
//! allocate through the system allocator.

mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;

#[library_benchmark]
#[bench::seven_documents(scenarios::corpus())]
fn decoding_order(corpus: Vec<(Vec<u8>, Option<&'static str>)>) -> (usize, usize) {
    black_box(scenarios::decode_all(black_box(&corpus)))
}

library_benchmark_group!(name = decoding, benchmarks = decoding_order);

main!(library_benchmark_groups = decoding);
