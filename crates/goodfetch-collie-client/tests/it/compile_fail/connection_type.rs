//! The Connection both Products' connections implement (ADRs 0095 and
//! 0099), which a helper a library serving both Products takes: no other
//! value fits it, async or blocking. Each case in `tests/ui/connection`
//! must fail to compile, with the error its `.stderr` file holds; Foxhound's
//! `tests/it/sdk.rs` shows a `Collie` and a `Foxhound` fit it.
//! Covers: story:11-179

#[test]
fn a_helper_typed_for_connection_takes_no_other_value() {
    trybuild::TestCases::new().compile_fail("tests/ui/connection/*.rs");
}
