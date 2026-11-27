//! A Live view viewer's messages, the channel's decoder of input it did not
//! write (ADRs 0067 and 0149).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::live_view::fuzz_viewer_message(data);
});
