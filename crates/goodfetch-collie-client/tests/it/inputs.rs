//! Inputs stay strict (ADR 0082): where a result ignores a field it does
//! not know, an input's builder has a setter only for the fields its
//! operation takes, so one given an unknown field does not compile. Each
//! case in `tests/ui/inputs` must fail to compile, with the error its
//! `.stderr` file holds.
//! Covers: story:11-68

#[test]
fn an_input_builder_given_an_unknown_field_does_not_build() {
    trybuild::TestCases::new().compile_fail("tests/ui/inputs/*.rs");
}
