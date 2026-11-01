//! The fetch tiers' heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.

mod scenarios;

use std::hint::black_box;

use gungraun::Tool;
use gungraun::prelude::*;

#[library_benchmark]
#[bench::seven_documents(scenarios::corpus())]
fn decoding_order(corpus: Vec<(Vec<u8>, Option<&'static str>)>) -> (usize, usize) {
    black_box(scenarios::decode_all(black_box(&corpus)))
}

library_benchmark_group!(name = decoding, benchmarks = decoding_order);

// BoringSSL, which the crate's transports link, brings libstdc++, whose
// emergency exception pool is allocated while the loader starts the
// process, on a stack DHAT unwinds into frames with no location, which
// gungraun cannot read: the pool is left out, as no benchmark throws a C++
// exception.
main!(
    config = LibraryBenchmarkConfig::default()
        .default_tool(Tool::DHAT)
        .env("GLIBCXX_TUNABLES", "glibcxx.eh_pool.obj_count=0"),
    library_benchmark_groups = decoding
);
