//! Playwright `storageState` JSON parser fuzz target (ADR 0067; ticket #735).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_page::fuzz_storage_state(data);
});
