//! The engine's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.
#![expect(
    clippy::disallowed_methods,
    reason = "bench targets run on the test Clock and Entropy (ADR 0071)"
)]

mod scenarios;

use std::hint::black_box;

use gungraun::Tool;
use gungraun::prelude::*;

#[library_benchmark]
#[bench::hundred_requests(scenarios::engine_info_lines(100))]
fn framing(lines: Vec<u8>) -> usize {
    black_box(scenarios::frame_and_decode(black_box(&lines)))
}

#[library_benchmark]
#[bench::hundred_calls(
    args = (100),
    setup = scenarios::dispatch_input,
    teardown = scenarios::dispatch_done
)]
fn dispatch(input: (Vec<u8>, tempfile::TempDir)) -> (usize, tempfile::TempDir) {
    let (lines, home) = input;
    let replies = black_box(scenarios::dispatch(black_box(&lines), home.path()));
    (replies, home)
}

#[library_benchmark]
#[bench::hundred_records(100)]
fn run_state_writes(count: usize) -> usize {
    black_box(scenarios::run_state_writes(black_box(count)))
}

library_benchmark_group!(
    name = engine,
    benchmarks = [framing, dispatch, run_state_writes]
);

main!(
    config = LibraryBenchmarkConfig::default().default_tool(Tool::DHAT),
    library_benchmark_groups = engine
);
