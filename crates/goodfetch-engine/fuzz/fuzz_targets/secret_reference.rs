//! Secret references, which a person wrote in a settings file or a program
//! passed as a credential setting's value (ADRs 0015 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::secrets::fuzz_secret_reference(data);
});
