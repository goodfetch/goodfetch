//! A Dataset's handle carries its Record's type (ADR 0103): a misspelled
//! handle and a push of a Record of another type are type errors, found
//! before a Run starts. Each case in `tests/ui/datasets` must fail to
//! compile, with the error its `.stderr` file holds.
//!
//! Covers: story:11-1058

#[test]
fn a_misspelled_dataset_and_a_record_of_another_type_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/datasets/*.rs");
}
