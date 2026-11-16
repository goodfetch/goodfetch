//! The engine's instruction counts, under gungraun's Callgrind with mimalloc
//! as the global allocator, as the executables ship (ADR 0071).
#![expect(
    clippy::disallowed_methods,
    reason = "bench targets run on the test Clock and Entropy (ADR 0071); its SQL statements are its own, on no Shared path (ADR 0140)"
)]

#[expect(
    dead_code,
    reason = "what only the counts cell and its test read of the scenario"
)]
mod messages;
mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

#[library_benchmark(
    config = LibraryBenchmarkConfig::default().pass_through_env(messages::PLANT_VARIABLE)
)]
#[bench::no_messages(
    args = (messages::NO_MESSAGES, messages::Logs::On),
    setup = messages::input,
    teardown = messages::done
)]
#[bench::small_params(
    args = (messages::SMALL_PARAMS, messages::Logs::On),
    setup = messages::input,
    teardown = messages::done
)]
#[bench::large_params(
    args = (messages::LARGE_PARAMS, messages::Logs::On),
    setup = messages::input,
    teardown = messages::done
)]
#[bench::no_messages_log_off(
    args = (messages::NO_MESSAGES, messages::Logs::Off),
    setup = messages::input,
    teardown = messages::done
)]
#[bench::small_params_log_off(
    args = (messages::SMALL_PARAMS, messages::Logs::Off),
    setup = messages::input,
    teardown = messages::done
)]
#[bench::large_params_log_off(
    args = (messages::LARGE_PARAMS, messages::Logs::Off),
    setup = messages::input,
    teardown = messages::done
)]
fn dispatch_messages(input: messages::Input) -> (usize, messages::Input) {
    let outgoing = black_box(messages::dispatched(black_box(&input)));
    (outgoing, input)
}

#[library_benchmark]
#[bench::hundred_records(100)]
fn run_state_writes(count: usize) -> usize {
    black_box(scenarios::run_state_writes(black_box(count)))
}

#[library_benchmark]
#[bench::thousand_entries(scenarios::egress_policy(1000))]
fn egress_judge(input: (goodfetch_engine::egress::Policy, Vec<String>)) -> usize {
    let (policy, urls) = input;
    black_box(scenarios::judge_all(black_box(&policy), black_box(&urls)))
}

#[library_benchmark]
#[bench::thousand_entries_hundred_ranges(scenarios::egress_policy_with_private_ranges(1000, 100))]
fn egress_judge_private_ranges(
    input: (
        goodfetch_engine::egress::Policy,
        Vec<(String, Vec<std::net::SocketAddr>)>,
    ),
) -> usize {
    let (policy, requests) = input;
    black_box(scenarios::judge_all_with_addresses(
        black_box(&policy),
        black_box(&requests),
    ))
}

#[library_benchmark]
#[bench::fifty_values_one_mib(
    args = (50, 1 << 20),
    setup = scenarios::mask_input
)]
fn mask_stream(
    input: (
        goodfetch_engine::mask::Masks,
        goodfetch_engine::mask::Mask,
        Vec<u8>,
    ),
) -> (usize, goodfetch_engine::mask::Mask) {
    let (masks, mask, stream) = input;
    let written = black_box(scenarios::mask_stream(
        black_box(&masks),
        black_box(&stream),
    ));
    (written, mask)
}

#[library_benchmark]
#[bench::thousand_joins_of_one_value(1000)]
fn mask_joins(joins: usize) -> usize {
    black_box(scenarios::mask_joins_of_one_value(black_box(joins)))
}

library_benchmark_group!(
    name = engine,
    benchmarks = [
        framing,
        dispatch,
        dispatch_messages,
        run_state_writes,
        egress_judge,
        egress_judge_private_ranges,
        mask_stream,
        mask_joins
    ]
);

main!(library_benchmark_groups = engine);
