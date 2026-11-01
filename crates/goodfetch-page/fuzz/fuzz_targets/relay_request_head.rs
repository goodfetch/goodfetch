//! The egress relay's request heads, which any local process can send to
//! a Session's relay before its credentials are checked (ADRs 0017 and
//! 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_page::fuzz_relay_request_head(data);
});
