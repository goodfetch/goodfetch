//! The inputs of the Console's sign-in endpoints (spec #592; ADR 0067): its
//! `Cookie` header, the `POST /sign-in` body and the `X-CSRF-Token` header,
//! which any page or program may send.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::console::fuzz_sign_in_endpoints(data);
});
