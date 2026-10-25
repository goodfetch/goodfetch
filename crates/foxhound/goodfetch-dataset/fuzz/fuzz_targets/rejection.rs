//! The rejection table's JSON, as a run file holds it, which a run file
//! imported from elsewhere may hold in any form (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_dataset::fuzz_rejection(data);
});
