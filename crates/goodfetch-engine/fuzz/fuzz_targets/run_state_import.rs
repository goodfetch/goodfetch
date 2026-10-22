//! Run-state import: a per-run file another machine or the other layout
//! wrote, which `run.resume` makes a Run's run file (ADRs 0002 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::runs::fuzz_run_state_import(data);
});
