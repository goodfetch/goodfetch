//! The Frontier's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.

mod scenarios;

use std::hint::black_box;

use goodfetch_frontier::Url;
use goodfetch_protocol::run::durability::MEMORY_BUDGET;
use gungraun::prelude::*;
use gungraun::{Dhat, EntryPoint, Tool};

#[library_benchmark]
#[bench::one_heavy_domain_and_a_thousand_small(scenarios::admission(64, 1_000))]
fn admit(requests: Vec<(String, i64)>) -> (usize, usize) {
    black_box(scenarios::admit_all(black_box(requests)))
}

library_benchmark_group!(name = admission, benchmarks = admit);

#[library_benchmark]
#[bench::a_thousand_shop_links(scenarios::parsed(&scenarios::links(1_000)))]
fn url_key(links: Vec<Url>) -> usize {
    black_box(scenarios::key_all(black_box(&links)))
}

library_benchmark_group!(name = identity, benchmarks = url_key);

// The Frontier's heap rows of the Quality register (ADRs 0134 and 0142),
// each read at the default Durability level, `batch`: `xtask perf` takes
// the bytes at the global maximum of a case, less those of its benchmark's
// smaller case, and holds them to their ceiling. The whole process counts:
// an allocation of SQLite's page cache sits deeper than DHAT's 25 callers
// below the benchmark function, so the default entry point would leave the
// cache out.
//
// `enqueue`: at `run_state.memory.budget: 0`, at most 8 bytes per URL at
// 100,000 URLs (ADR 0134, kept at budget 0 by ADR 0142).
#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Dhat::default().entry_point(EntryPoint::None))
)]
#[bench::no_urls(0)]
#[bench::hundred_thousand_urls(100_000)]
fn enqueue(urls: usize) -> usize {
    black_box(scenarios::enqueue(black_box(urls), Some(0)))
}

/// ADR 0142's test budget, 1 MiB, which 100,000 URLs already spill past.
const TEST_BUDGET: u64 = 1024 * 1024;

// `grow`: at the 1 MiB test budget, at most 8 bytes per URL from 100,000 to
// 1,000,000 URLs: past the spill, memory stays within its budget.
#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Dhat::default().entry_point(EntryPoint::None))
)]
#[bench::hundred_thousand_urls(100_000)]
#[bench::million_urls(1_000_000)]
fn grow(urls: usize) -> usize {
    black_box(scenarios::enqueue(black_box(urls), Some(TEST_BUDGET)))
}

// `hold`: at the default budget, before any spill, at most 95 bytes per URL
// at 100,000 URLs, the visits memory holds included.
#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Dhat::default().entry_point(EntryPoint::None))
)]
#[bench::no_urls(0)]
#[bench::hundred_thousand_urls(100_000)]
fn hold(urls: usize) -> usize {
    black_box(scenarios::enqueue(black_box(urls), Some(MEMORY_BUDGET)))
}

// `spill`: at the default budget and 1,000,000 URLs, past the spill, at
// most the budget plus 10 %.
#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Dhat::default().entry_point(EntryPoint::None))
)]
#[bench::no_urls(0)]
#[bench::million_urls(1_000_000)]
fn spill(urls: usize) -> usize {
    black_box(scenarios::enqueue(black_box(urls), Some(MEMORY_BUDGET)))
}

library_benchmark_group!(name = frontier, benchmarks = [enqueue, grow, hold, spill]);

// An attempt's staged enqueues (ADR 0093): what the attempt keeps in memory
// beside its staging grows with what it added, by the same amount each.
#[library_benchmark]
#[bench::a_hundred_enqueues(100)]
#[bench::a_thousand_enqueues(1_000)]
fn stage(enqueues: usize) -> usize {
    black_box(scenarios::stage(black_box(enqueues)))
}

library_benchmark_group!(name = staging, benchmarks = stage);

// BoringSSL, which goodfetch-page's egress relay links, brings libstdc++,
// whose emergency exception pool is allocated while the loader starts the
// process, on a stack DHAT unwinds into frames with no location, which
// gungraun cannot read: the pool is left out, as no benchmark throws a C++
// exception.
main!(
    config = LibraryBenchmarkConfig::default()
        .default_tool(Tool::DHAT)
        .env("GLIBCXX_TUNABLES", "glibcxx.eh_pool.obj_count=0"),
    library_benchmark_groups = [admission, identity, frontier, staging]
);
