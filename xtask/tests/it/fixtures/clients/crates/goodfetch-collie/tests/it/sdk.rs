//! The Rust SDK's tests on a Linked engine: a test carries a case on the
//! `sdk-rust` surface by the `ClientCaseRun::start` call in its body.

use goodfetch_conformance::{ClientCaseRun, RustMode};

#[test]
fn refuses_a_newer_major() {
    let _case = ClientCaseRun::start(None, "hello-refused", RustMode::Linked).unwrap();
}

/// A helper's call is no test's, so it carries nothing.
fn started(case: &str) -> ClientCaseRun {
    ClientCaseRun::start(None, case, RustMode::Linked).unwrap()
}

#[test]
fn starts_a_run() {
    let _case = started("start-a-run");
}
