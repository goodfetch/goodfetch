//! A Plugin's bundle, a tar archive a registry served, unpacked as `plugin
//! add` unpacks it (ADRs 0033 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::plugin_bundle::fuzz_plugin_bundle(data);
});
