//! Checkpoint files, which a Client hands an engine to resume a Run from, a
//! copy another machine wrote (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::checkpoint::fuzz_checkpoint(data);
});
