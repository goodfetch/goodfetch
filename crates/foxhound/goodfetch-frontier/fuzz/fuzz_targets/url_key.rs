//! The URL key over arbitrary `href`s, as pages a crawl reads write them
//! (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_frontier::fuzz_url_key(data);
});
