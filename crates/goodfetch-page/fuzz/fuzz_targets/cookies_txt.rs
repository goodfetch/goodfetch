//! Netscape `cookies.txt` parser fuzz target (ADR 0067; ticket #735).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_page::fuzz_cookies_txt(data);
});
