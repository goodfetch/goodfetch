//! A setting only Foxhound carries has no constant in the Collie client
//! (ADR 0095): naming `politeness.domain.concurrency` in a Collie Run's
//! settings does not compile. Each case in `tests/ui/foxhound_settings`
//! must fail to compile, with the error its `.stderr` file holds.
//!
//! Covers: story:11-1092

#[test]
fn a_key_only_foxhound_carries_does_not_compile_in_a_collie_run() {
    trybuild::TestCases::new().compile_fail("tests/ui/foxhound_settings/*.rs");
}
