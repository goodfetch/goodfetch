//! Exported Audit logs, the JSONL `audit export` writes, which
//! `audit verify --file` reads from wherever the copy was kept (ADRs 0016
//! and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::audit::fuzz_audit_jsonl(data);
});
