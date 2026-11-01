//! Chrome's `<meta>` prescan, the XML declaration and the decoding order
//! over arbitrary bytes, as a crawl's documents hold them (ADRs 0067 and
//! 0119).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_fetch_tier::fuzz_prescan(data);
});
