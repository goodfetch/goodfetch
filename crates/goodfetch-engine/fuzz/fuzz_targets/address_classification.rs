//! The resolved-address classification: the addresses a lookup answers and
//! the hosts of URLs a page names, which an attacker may choose (ADR 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::egress::fuzz_address_classification(data);
});
