//! Foxhound's crawl instruction counts, under gungraun's Callgrind with
//! mimalloc as the global allocator, as the executable ships (ADR 0071).
//!
//! `admit_seeds`: a crawl started with a thousand seeds, which the Run's
//! start admits through the crawl's Run state as it makes the run file
//! (#2167). The count is bounded to that admission, the Run state's
//! never-inlined `admit_seeds`, so the engine's start, the Connection and the
//! abort that ends the Run are left out.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]

#[expect(
    dead_code,
    reason = "the fixture's crawls, which the heap and counts benchmarks run and this one does not"
)]
mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;
use gungraun::{Callgrind, EntryPoint};

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The Run state's seed admission, which every seed's admission ends in.
const ADMISSION: &str = "goodfetch_crawl::run_state::admission::admit_seeds";

#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Callgrind::default().entry_point(EntryPoint::Custom(ADMISSION.to_owned())))
)]
#[bench::a_thousand_shop_links(scenarios::shop_links(1_000))]
fn admit_seeds(links: Vec<String>) -> usize {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    black_box(scenarios::start_with_seeds(
        home.path(),
        std::time::UNIX_EPOCH,
        0,
        black_box(&links),
    ))
}

library_benchmark_group!(name = seeds, benchmarks = admit_seeds);

main!(library_benchmark_groups = seeds);
