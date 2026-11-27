//! A Plugin's OCI image index and image manifest, and the reference `plugin
//! add` names, which a registry and a person wrote (ADRs 0033 and 0067).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_engine::plugin_oci::fuzz_oci_image_index(data);
});
