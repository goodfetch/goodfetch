//! robots.txt files, which a site wrote (RFC 9309; ADRs 0067 and 0086).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::robots_txt::fuzz_robots_txt(data);
});
