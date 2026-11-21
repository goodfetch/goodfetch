//! AI work's answer sits in how it ended (ADR 0117): reading an output
//! the result does not hold, a `match` on `Ended` that skips the stopped
//! work, or a stopped task's partial answer taken for the declared output
//! (ADR 0079), does not compile. Each case in `tests/ui/ai_work` must fail
//! to compile, with the error its `.stderr` file holds.
//! Covers: story:11-34, story:11-35

#[test]
fn taking_an_answer_without_how_the_work_ended_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/ai_work/*.rs");
}
