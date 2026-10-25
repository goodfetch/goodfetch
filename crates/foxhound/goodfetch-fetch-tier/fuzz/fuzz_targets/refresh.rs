//! The refresh-stub parser over arbitrary bytes, as a `Refresh` header's
//! value and as a document holding `<meta http-equiv="refresh">` (ADR
//! 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_fetch_tier::fuzz_refresh(data);
});
