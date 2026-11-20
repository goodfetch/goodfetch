//! A program's own error is a `BoxError`, `Send + Sync + 'static` (ADR
//! 0109): an error that is not `Send` does not compile into one. Each case in
//! `tests/ui/error` must fail to compile, with the error its `.stderr` file
//! holds.
//! Covers: story:11-36

#[test]
fn an_error_that_is_not_send_does_not_compile_into_a_box_error() {
    trybuild::TestCases::new().compile_fail("tests/ui/error/*.rs");
}
