//! The page crate's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): the guard's check-and-arm round trip and the
//! interception's per-request pipeline.

mod scenarios;

use std::hint::black_box;

use gungraun::Tool;
use gungraun::prelude::*;

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

#[library_benchmark]
#[bench::hundred_connections(100)]
fn relay_connections(count: usize) -> usize {
    black_box(scenarios::relay_connections(black_box(count)))
}

#[library_benchmark]
#[bench::four_mebibytes(4)]
fn relay_tunnel(mebibytes: usize) -> usize {
    black_box(scenarios::relay_tunnel(black_box(mebibytes)))
}

library_benchmark_group!(
    name = egress_relay,
    benchmarks = [relay_connections, relay_tunnel]
);

main!(
    config = LibraryBenchmarkConfig::default().default_tool(Tool::DHAT),
    library_benchmark_groups = [guard, interception, egress_relay]
);
