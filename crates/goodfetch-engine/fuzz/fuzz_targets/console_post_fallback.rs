//! The inputs of the Console's `POST` fallback (spec #592; ADR 0067): the
//! path naming its event stream and the body carrying the page's messages,
//! which any page or program may send.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::console::fuzz_post_fallback(data);
});
