//! A Label's handle carries its Payload's type (ADRs 0082 and 0093): a
//! misspelled handle, a Payload of another type than its Label's, and a
//! Payload given to a Label that has none are type errors, found before a
//! Run starts. Each case in `tests/ui/payloads` must fail to compile, with
//! the error its `.stderr` file holds.
//! Covers: story:11-1017

#[test]
fn a_payload_wired_to_the_wrong_label_does_not_build() {
    trybuild::TestCases::new().compile_fail("tests/ui/payloads/*.rs");
}
