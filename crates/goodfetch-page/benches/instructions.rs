//! The page crate's instruction counts, under gungraun's Callgrind with
//! mimalloc as the global allocator, as the executables ship (ADR 0071):
//! the guard's check-and-arm round trip (ADR 0019's speed target) and the
//! interception's per-request pipeline (spec #577).

mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[library_benchmark]
#[bench::hundred_round_trips(100)]
fn check_and_arm(count: usize) -> usize {
    black_box(scenarios::check_and_arm(black_box(count)))
}

library_benchmark_group!(name = guard, benchmarks = check_and_arm);

#[library_benchmark]
#[bench::thousand_requests_with_filter_lists(args = (1000), setup = scenarios::filter_lists)]
fn intercept((count, blocking): (usize, scenarios::steps::Blocking)) -> usize {
    black_box(scenarios::intercept(black_box(count), &blocking))
}

library_benchmark_group!(name = interception, benchmarks = intercept);

main!(library_benchmark_groups = [guard, interception]);
