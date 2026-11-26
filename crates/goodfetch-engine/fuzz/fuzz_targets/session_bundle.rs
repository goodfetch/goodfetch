//! A session export bundle opened and read as an import reads it: a file
//! from another machine (ADRs 0015 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::session_bundle::fuzz_session_bundle(data);
});
