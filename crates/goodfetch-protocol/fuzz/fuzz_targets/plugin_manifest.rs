//! A Plugin's manifest, which the engine reads before it approves or starts
//! the Plugin (ADRs 0033 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_protocol::plugin::fuzz_plugin_manifest(data);
});
