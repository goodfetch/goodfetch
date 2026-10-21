//! The Frontier's heap metrics, under gungraun's DHAT with the system
//! allocator (ADR 0071): DHAT counts allocations by replacing `malloc`.

mod scenarios;

use std::hint::black_box;

use goodfetch_frontier::Url;
use gungraun::Tool;
use gungraun::prelude::*;

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

main!(
    config = LibraryBenchmarkConfig::default().default_tool(Tool::DHAT),
    library_benchmark_groups = [admission, identity]
);
