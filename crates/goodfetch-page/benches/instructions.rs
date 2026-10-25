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

#[library_benchmark]
#[bench::ten_connections(args = (10), setup = with_terminated)]
fn relay_terminated_connections((count, fixture): (usize, scenarios::Terminated)) -> usize {
    black_box(scenarios::relay_terminated_connections(
        black_box(count),
        &fixture,
    ))
}

#[library_benchmark]
#[bench::four_mebibytes(args = (4), setup = with_terminated)]
fn relay_terminated_tunnel((mebibytes, fixture): (usize, scenarios::Terminated)) -> usize {
    black_box(scenarios::relay_terminated_tunnel(
        black_box(mebibytes),
        &fixture,
    ))
}

/// The terminating scenarios' state, made outside the measure.
fn with_terminated(count: usize) -> (usize, scenarios::Terminated) {
    (count, scenarios::terminated())
}

library_benchmark_group!(
    name = egress_relay,
    benchmarks = [
        relay_connections,
        relay_tunnel,
        relay_terminated_connections,
        relay_terminated_tunnel
    ]
);

main!(library_benchmark_groups = [guard, interception, egress_relay]);
