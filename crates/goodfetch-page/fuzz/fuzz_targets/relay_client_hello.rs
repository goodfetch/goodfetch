//! The ClientHello the egress relay reads from the browser and mirrors
//! upstream in its terminating mode (ADRs 0042, 0067 and 0124).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_page::fuzz_relay_client_hello(data);
});
