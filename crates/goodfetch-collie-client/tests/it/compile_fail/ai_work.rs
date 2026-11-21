//! AI work's answer sits in how it ended (ADR 0117): reading an output
//! the result does not hold, or a `match` on `Ended` that skips the
//! stopped work, does not compile. Each case in `tests/ui/ai_work` must
//! fail to compile, with the error its `.stderr` file holds.
//! Covers: story:11-34

#[test]
fn reading_an_output_before_how_the_work_ended_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/ai_work/*.rs");
}
