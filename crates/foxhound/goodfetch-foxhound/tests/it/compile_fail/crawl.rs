//! A crawl's mistakes the compiler catches (ADRs 0081 and 0093): a
//! handler's future is `Send`, so one holding an `Rc` across an await
//! does not compile, and a Label is a handle the program declares, so a
//! misspelled one is an unknown name. Each case in `tests/ui/crawl` must
//! fail to compile, with the error its `.stderr` file holds.
//!
//! Covers: story:11-1016, story:11-1101

#[test]
fn a_non_send_handler_and_a_misspelled_label_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/crawl/*.rs");
}
