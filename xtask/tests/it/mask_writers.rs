//! The Mask writers check (spec #564): every writer the Products' crates hold
//! is listed with how it meets the `Mask`, so a new writer cannot skip it
//! unseen.
//! Covers: story:4-56

use rstest::rstest;

use crate::workspace::{Workspace, finding};

const LIST: &str = "crates/goodfetch-engine/src/mask/writers.toml";
const WRITER: &str = "crates/goodfetch-engine/src/report.rs";

/// A file that writes one whole.
const ONE_WRITE: &str =
    "fn keep(path: &Path) {\n    std::fs::write(path, \"report\").unwrap();\n}\n";

/// A file that writes two wholes.
const TWO_WRITES: &str = "fn keep(path: &Path) {\n    std::fs::write(path, \"report\").unwrap();\n    std::fs::write(path, \"again\").unwrap();\n}\n";

/// A list holding `path`, whose `count` writers hold no value.
fn listing(path: &str, count: usize) -> String {
    format!("[[writer]]\npath = \"{path}\"\ncount = {count}\nhow = \"holds no value: a report\"\n")
}

fn workspace(list: &str, files: &[(&str, &str)]) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(LIST, list.as_bytes());
    for (path, source) in files {
        workspace.write(path, source.as_bytes());
    }
    workspace
}

/// Any crate adding a file write fails the check until the list says how it
/// meets the `Mask`.
#[test]
fn a_writer_the_list_does_not_hold_fails() {
    let workspace = workspace("writer = []\n", &[(WRITER, ONE_WRITE)]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("mask-writer-unlisted", WRITER)]
    );
    assert_eq!(outcome.code, 1);
}

/// A new writer in a file the list already holds, which is the one a count
/// alone can see.
#[test]
fn a_file_with_more_writers_than_the_list_says_fails() {
    let workspace = workspace(&listing(WRITER, 1), &[(WRITER, TWO_WRITES)]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("mask-writer-count", WRITER)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_file_with_fewer_writers_than_the_list_says_fails() {
    let workspace = workspace(&listing(WRITER, 2), &[(WRITER, ONE_WRITE)]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("mask-writer-count", WRITER)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_listed_file_that_holds_no_writer_now_fails() {
    let workspace = workspace(&listing(WRITER, 1), &[(WRITER, "fn nothing() {}\n")]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("mask-writer-gone", WRITER)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_listed_writer_that_does_not_say_how_it_meets_the_mask_fails() {
    let list = format!("[[writer]]\npath = \"{WRITER}\"\ncount = 1\nhow = \" \"\n");
    let workspace = workspace(&list, &[(WRITER, ONE_WRITE)]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("mask-writer-unexplained", WRITER)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_listed_writer_passes() {
    let workspace = workspace(&listing(WRITER, 1), &[(WRITER, ONE_WRITE)]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(outcome.findings(), Vec::new());
    assert_eq!(outcome.code, 0);
}

/// What writes no Product output is left out, and so is a comment that names
/// a writer.
#[rstest]
#[case::a_test_file("crates/goodfetch-engine/src/report/tests.rs", ONE_WRITE)]
#[case::an_integration_test("crates/goodfetch-engine/tests/it/report.rs", ONE_WRITE)]
#[case::a_benchmark("crates/goodfetch-engine/benches/report.rs", ONE_WRITE)]
#[case::an_example("crates/goodfetch-engine/examples/report.rs", ONE_WRITE)]
#[case::a_build_script("crates/goodfetch-engine/build.rs", ONE_WRITE)]
#[case::test_support("crates/goodfetch-test-support/src/report.rs", ONE_WRITE)]
#[case::a_comment(WRITER, "// std::fs::write(path, bytes)\nfn nothing() {}\n")]
fn what_writes_no_product_output_is_left_out(#[case] path: &str, #[case] source: &str) {
    let workspace = workspace("writer = []\n", &[(path, source)]);

    let outcome = workspace.xtask(&["mask-writers"]);

    assert_eq!(outcome.findings(), Vec::new());
    assert_eq!(outcome.code, 0);
}
