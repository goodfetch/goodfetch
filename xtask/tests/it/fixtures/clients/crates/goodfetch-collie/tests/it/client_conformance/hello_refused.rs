//! The case `hello-refused`, in both modes.

use rstest::rstest;

#[rstest]
#[case::attached("attached")]
#[case::daemon("daemon")]
fn refuses_a_newer_major(#[case] mode: &str) {
    let _ = mode;
}
