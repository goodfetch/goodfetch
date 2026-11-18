//! A call's timeout is one duration type, `std::time::Duration` (ADR
//! 0080): a number of milliseconds, Playwright's `timeout(30_000)`, does not
//! compile. Each case in `tests/ui/duration` must fail to compile, with the
//! error its `.stderr` file holds.
//! Covers: story:11-40

#[test]
fn a_timeout_in_milliseconds_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/duration/*.rs");
}
