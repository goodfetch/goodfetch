//! The settings files, an Operator policy and user and project
//! configuration, which a person or another Product's engine wrote (ADR
//! 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::settings::fuzz_settings_file(data);
});
