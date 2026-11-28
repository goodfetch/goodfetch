//! The peer link's framing and answers, the lines another Group member of a
//! Daemon group sends (ADRs 0055 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::group::fuzz_peer_framing(data);
});
