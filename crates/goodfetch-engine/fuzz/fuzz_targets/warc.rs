//! WARC files, a Response recording's, which a user hands the engine
//! (ADRs 0067 and 0105).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::warc::fuzz_warc(data);
});
