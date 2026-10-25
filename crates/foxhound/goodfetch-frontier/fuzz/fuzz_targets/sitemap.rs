//! Sitemap files, plain XML or gzip, as sites serve them (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_frontier::sitemap::fuzz_sitemap(data);
});
