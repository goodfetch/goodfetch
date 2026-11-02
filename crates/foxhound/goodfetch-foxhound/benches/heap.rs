//! Foxhound's crawl heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.
//!
//! ADR 0134's bytes allocated per crawled page: `xtask perf` divides
//! `crawl::in_memory_pages`' total bytes by the body bytes
//! `pages.source.toml` records and holds the quotient at 2× or under. The
//! whole process counts, as the Frontier's benchmark does: an allocation of
//! SQLite's page cache, or of a page's parse deep in the crawl's tasks, sits
//! deeper than DHAT's 25 callers below the benchmark function, where the
//! default entry point would leave it out. The benchmark takes no
//! argument, so its ID is the one the ceiling reads, and its site's bodies
//! are borrowed from a static fixture, so serving them allocates nothing.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]

#[expect(
    dead_code,
    reason = "the counts cell's outcomes, which this benchmark's crawl does not meet"
)]
mod scenarios;

use std::hint::black_box;

use gungraun::prelude::*;
use gungraun::{Dhat, EntryPoint, Tool};

#[library_benchmark(
    config = LibraryBenchmarkConfig::default()
        .tool(Dhat::default().entry_point(EntryPoint::None))
)]
fn in_memory_pages() -> usize {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let crawled = scenarios::crawl(home.path(), std::time::UNIX_EPOCH, 0);
    assert_eq!(
        crawled.body_bytes,
        scenarios::body_bytes(),
        "invariant: the site sends each body once"
    );
    black_box(crawled.records.len())
}

library_benchmark_group!(name = crawl, benchmarks = in_memory_pages);

// BoringSSL, which goodfetch-page's egress relay links, brings libstdc++,
// whose emergency exception pool is allocated while the loader starts the
// process, on a stack DHAT unwinds into frames with no location, which
// gungraun cannot read: the pool is left out, as no benchmark throws a C++
// exception.
main!(
    config = LibraryBenchmarkConfig::default()
        .default_tool(Tool::DHAT)
        .env("GLIBCXX_TUNABLES", "glibcxx.eh_pool.obj_count=0"),
    library_benchmark_groups = crawl
);
