//! A Schedule's cron trigger expression, which `croner` parses (ADRs 0036
//! and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_schedule::fuzz_cron_expression(data);
});
