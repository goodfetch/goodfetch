//! Reference screenshots (ADR 0073): `screenshots fetch` applies only a run of
//! the `update-screenshots` workflow made in the pinned image on the branch's
//! head, and `screenshots orphans` refuses a reference no visual test read.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

/// The fixture workspace's pinned digest, in `ci/visual/image.toml`.
const PINNED: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

const OTHER_DIGEST: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";

const OTHER_COMMIT: &str = "3333333333333333333333333333333333333333";

const VITEST_REFERENCE: &str =
    "console/src/button/__screenshots__/button.visual.test.tsx/primary-chromium.png";

const PLAYWRIGHT_REFERENCE: &str = "website/tests/home.visual.spec.ts-snapshots/home-visual.png";

/// Writes a downloaded `update-screenshots` artifact at `run/` and fetches it.
fn fetch(workspace: &Workspace, digest: &str, commit: &str, files: &[(&str, &[u8])]) -> Outcome {
    workspace.write(
        "run/provenance.json",
        format!(r#"{{ "digest": "{digest}", "commit": "{commit}", "run": 4242 }}"#).as_bytes(),
    );
    for (path, contents) in files {
        workspace.write(&format!("run/screenshots/{path}"), contents);
    }
    workspace.xtask(&["screenshots", "fetch", "run"])
}

#[test]
fn run_in_the_pinned_image_on_the_head_applies_its_screenshots() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write(VITEST_REFERENCE, b"old");
    let head = workspace.commit_all();

    let outcome = fetch(
        &workspace,
        PINNED,
        &head,
        &[(VITEST_REFERENCE, b"new"), (PLAYWRIGHT_REFERENCE, b"added")],
    );

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
    assert_eq!(workspace.read(VITEST_REFERENCE), Some(b"new".to_vec()));
    assert_eq!(
        workspace.read(PLAYWRIGHT_REFERENCE),
        Some(b"added".to_vec())
    );
}

#[test]
fn run_that_changed_no_screenshot_applies_nothing() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write(VITEST_REFERENCE, b"old");
    let head = workspace.commit_all();

    let outcome = fetch(&workspace, PINNED, &head, &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
    assert_eq!(workspace.read(VITEST_REFERENCE), Some(b"old".to_vec()));
}

#[test]
fn run_in_another_image_is_refused_and_applies_nothing() {
    let workspace = Workspace::from_fixture("screenshots");
    let head = workspace.commit_all();

    let outcome = fetch(
        &workspace,
        OTHER_DIGEST,
        &head,
        &[(VITEST_REFERENCE, b"new")],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding("image-not-pinned", OTHER_DIGEST)]
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(workspace.read(VITEST_REFERENCE), None);
}

#[rstest]
#[case::workflow(".github/workflows/pull-request.yml")]
#[case::source_file("console/src/button/button.tsx")]
#[case::image_outside_a_reference_folder("console/src/button/icon.png")]
#[case::snapshot_beside_a_test("console/src/button/__snapshots__/button.test.ts.snap")]
fn run_carrying_a_file_that_is_not_a_reference_screenshot_is_refused(#[case] path: &str) {
    let workspace = Workspace::from_fixture("screenshots");
    let head = workspace.commit_all();

    let outcome = fetch(
        &workspace,
        PINNED,
        &head,
        &[(VITEST_REFERENCE, b"new"), (path, b"planted")],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding("not-a-reference-screenshot", path)]
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(workspace.read(VITEST_REFERENCE), None);
    assert_eq!(workspace.read(path), None);
}

#[test]
fn reference_no_visual_test_read_is_an_orphan() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write(VITEST_REFERENCE, b"png");
    workspace.write(PLAYWRIGHT_REFERENCE, b"png");
    workspace.write(
        "target/visual/read/vitest.txt",
        format!("{VITEST_REFERENCE}\n").as_bytes(),
    );

    let outcome = workspace.xtask(&["screenshots", "orphans"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "orphaned-reference-screenshot",
            PLAYWRIGHT_REFERENCE
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn references_every_visual_test_read_pass() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write(VITEST_REFERENCE, b"png");
    workspace.write(PLAYWRIGHT_REFERENCE, b"png");
    workspace.write(
        "target/visual/read/vitest.txt",
        format!("{VITEST_REFERENCE}\n").as_bytes(),
    );
    workspace.write(
        "target/visual/read/playwright-1.txt",
        format!("{PLAYWRIGHT_REFERENCE}\n").as_bytes(),
    );

    let outcome = workspace.xtask(&["screenshots", "orphans"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn orphan_check_without_a_recorded_visual_run_is_an_error_never_a_pass() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write(VITEST_REFERENCE, b"png");

    let outcome = workspace.xtask(&["screenshots", "orphans"]);

    assert_eq!(outcome.code, 2);
}

#[test]
fn run_on_a_commit_that_is_not_the_head_is_refused_and_applies_nothing() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.commit_all();

    let outcome = fetch(
        &workspace,
        PINNED,
        OTHER_COMMIT,
        &[(VITEST_REFERENCE, b"new")],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding("commit-not-head", OTHER_COMMIT)]
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(workspace.read(VITEST_REFERENCE), None);
}
