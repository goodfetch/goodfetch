//! The plugin protocol's messages, one a line, as a Plugin's stdout carries
//! them, with the `hello` result held to an approved manifest (ADRs 0033
//! and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_protocol::plugin::fuzz_plugin_messages(data);
});
