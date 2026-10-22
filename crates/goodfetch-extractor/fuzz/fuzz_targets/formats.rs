//! The page formats' HTML entry point, which reads a document a site wrote
//! (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_extractor::fuzz_formats(data);
});
