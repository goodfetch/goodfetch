//! A setting's value is typed by its key (ADR 0083): a number of
//! milliseconds for a duration, `None` for a setting that cannot be
//! cleared, a field a protocol shape lacks, or untyped JSON in place of
//! that shape, does not compile. Each case in `tests/ui/settings` must fail
//! to compile, with the error its `.stderr` file holds.

#[test]
fn a_value_of_the_wrong_type_for_its_key_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/settings/*.rs");
}
