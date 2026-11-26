//! JSON-RPC framing and decoding, the engine's first decoder of input it did
//! not write (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::framing::fuzz_framing(data);
});
