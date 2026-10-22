//! Certificate and key files, which a person names in `tls.trusted_cas`
//! and, for a client certificate, its key, and a pin row's key bytes in the
//! known-certificates store (ADRs 0042 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::tls::fuzz_certificate_file(data);
});
