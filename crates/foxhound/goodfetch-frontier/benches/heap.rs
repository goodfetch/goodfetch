//! The Frontier's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.

mod scenarios;

use std::hint::black_box;

use goodfetch_frontier::Url;
use gungraun::prelude::*;
use gungraun::{Dhat, EntryPoint, Tool};

#[library_benchmark]
#[bench::one_heavy_domain_and_a_thousand_small(scenarios::requests(64, 1_000))]
fn admit(requests: Vec<(String, i64)>) -> (usize, usize) {
    black_box(scenarios::admit_all(black_box(requests)))
}

library_benchmark_group!(name = admission, benchmarks = admit);

#[library_benchmark]
#[bench::a_thousand_shop_links(scenarios::parsed(&scenarios::links(1_000)))]
fn url_key(links: Vec<Url>) -> usize {
    black_box(scenarios::key_all(black_box(&links)))
}

#[library_benchmark]
#[bench::a_thousand_shop_links(scenarios::links(1_000))]
fn admit_seeds(links: Vec<String>) -> usize {
    black_box(scenarios::admit_seeds(black_box(&links)))
}

library_benchmark_group!(name = identity, benchmarks = [url_key, admit_seeds]);

// ADR 0134's heap bytes per URL: `xtask perf` takes the bytes at the global
// maximum of the 100,000 URLs, less the same scenario with none, over
// 100,000, and holds it to its 8-byte ceiling. The whole process counts: an
// allocation of SQLite's page cache sits deeper than DHAT's 25 callers
// below the benchmark function, so the default entry point would leave the
// cache out.
#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Dhat::default().entry_point(EntryPoint::None))
)]
#[bench::no_urls(0)]
#[bench::hundred_thousand_urls(100_000)]
fn enqueue(urls: usize) -> usize {
    black_box(scenarios::enqueue(black_box(urls)))
}

library_benchmark_group!(name = frontier, benchmarks = enqueue);

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
