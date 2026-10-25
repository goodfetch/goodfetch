//! The Frontier's instruction counts, under gungraun's Callgrind (ADR 0071):
//! admission through the per-domain queues, and a request's identity: the
//! URL key, and `admit` against the visited set in a run file in memory.
//! The scenarios allocate through the system allocator; mimalloc, the
//! allocator the binary ships, joins the Frontier's scenarios once its
//! other parts arrive.

mod scenarios;

use std::hint::black_box;

use goodfetch_frontier::Url;
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

#[library_benchmark]
#[bench::no_urls(0)]
#[bench::hundred_thousand_urls(100_000)]
fn enqueue(urls: usize) -> usize {
    black_box(scenarios::enqueue(black_box(urls)))
}

library_benchmark_group!(name = frontier, benchmarks = enqueue);

main!(library_benchmark_groups = [admission, identity, frontier]);
