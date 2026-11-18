//! The forms the catalog's members take in Rust (ADR 0099), which no
//! program can misuse: a boolean is never a positional argument, on a
//! round trip, on a close or on a blocking twin, and a `Query` has no
//! `timeout`. Each case in `tests/ui/forms` must fail to compile, with the
//! error its `.stderr` file holds.
//! Covers: story:11-178

#[test]
fn a_boolean_is_never_positional_and_a_query_takes_no_timeout() {
    trybuild::TestCases::new().compile_fail("tests/ui/forms/*.rs");
}
