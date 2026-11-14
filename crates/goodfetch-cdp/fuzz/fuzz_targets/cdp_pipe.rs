//! The CDP pipe as the browser writes it: its NUL-delimited framing and each
//! message's decoding into an answer or an event (ADRs 0017 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_cdp::fuzz_pipe(data);
});
