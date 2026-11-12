//! Reference screenshots (ADR 0073): `screenshots fetch` applies only a run of
//! the `update-screenshots` workflow made in the pinned image on the branch's
//! head, `screenshots orphans` refuses a reference no visual test read, and
//! `screenshots check` refuses a comparison tolerance set outside the shared
//! visual config and a Reference screenshot over 512 KiB.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

/// The fixture workspace's pinned digest, in `ci/visual/image.toml`.
const PINNED: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

const OTHER_DIGEST: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";

const OTHER_COMMIT: &str = "3333333333333333333333333333333333333333";

const VITEST_REFERENCE: &str =
    "website/src/button/__screenshots__/button.visual.test.tsx/primary-chromium.png";

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
#[case::source_file("website/src/button/button.tsx")]
#[case::image_outside_a_reference_folder("website/src/button/icon.png")]
#[case::snapshot_beside_a_test("website/src/button/__snapshots__/button.test.ts.snap")]
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

/// ADR 0073's limit for one Reference screenshot, 512 KiB.
const REFERENCE_LIMIT: u64 = 524_288;

#[rstest]
#[case::max_diff_pixels("max-diff-pixels.spec.ts", "website/tests/docs.visual.spec.ts", 4)]
#[case::max_diff_pixel_ratio(
    "max-diff-pixel-ratio.spec.ts",
    "website/tests/docs.visual.spec.ts",
    4
)]
#[case::threshold_in_a_test("threshold.spec.ts", "website/tests/docs.visual.spec.ts", 4)]
#[case::allowed_mismatched_pixels(
    "allowed-mismatched-pixels.test.ts",
    "website/src/button/button.visual.test.ts",
    7
)]
#[case::allowed_mismatched_pixel_ratio(
    "allowed-mismatched-pixel-ratio.test.ts",
    "website/src/button/button.visual.test.ts",
    7
)]
#[case::threshold_in_comparator_options(
    "threshold-in-comparator-options.test.ts",
    "website/src/button/button.visual.test.ts",
    7
)]
#[case::threshold_in_the_runner_config("threshold-in-runner-config.ts", "playwright.config.ts", 6)]
// In a visual test, `threshold` counts wherever it is written, so a variable
// passed as the options does not hide it.
#[case::threshold_in_a_variable_of_a_visual_test(
    "threshold-in-a-variable.spec.ts",
    "website/tests/docs.visual.spec.ts",
    4
)]
// A helper module in a fixtures folder, which a visual spec may import.
#[case::in_a_fixtures_folder("max-diff-pixels.spec.ts", "website/tests/fixtures/loose.ts", 4)]
// Test support is not the shared config: only its one file is.
#[case::in_test_support_beside_the_shared_config(
    "max-diff-pixels.spec.ts",
    "test-support/typescript/src/screenshot.ts",
    4
)]
fn tolerance_outside_the_shared_config_fails(
    #[case] fixture: &str,
    #[case] path: &str,
    #[case] line: u32,
) {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.add_fixture_file(&format!("screenshots-cases/{fixture}"), path);

    let outcome = workspace.xtask(&["screenshots", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "tolerance-outside-shared-config",
            &format!("{path}:{line}")
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn workspace_whose_tolerances_are_all_in_the_shared_config_passes() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write_sized(VITEST_REFERENCE, 4096);

    let outcome = workspace.xtask(&["screenshots", "check"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
// This check's own cases break the rule on purpose.
#[case::in_the_checks_own_cases("xtask/tests/it/fixtures/screenshots-cases/loose.spec.ts")]
// Outside a visual test, `threshold` in a variable is a common word's.
#[case::threshold_in_a_variable_outside_a_visual_test("website/src/observer.ts")]
#[case::in_frozen_research("rnd/lab/loose.spec.ts")]
fn tolerance_where_no_suite_runs_passes(#[case] path: &str) {
    let workspace = Workspace::from_fixture("screenshots");
    let fixture = if path.ends_with("observer.ts") {
        "screenshots-cases/threshold-in-a-variable.spec.ts"
    } else {
        "screenshots-cases/max-diff-pixels.spec.ts"
    };
    workspace.add_fixture_file(fixture, path);

    let outcome = workspace.xtask(&["screenshots", "check"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::vitest(VITEST_REFERENCE)]
#[case::playwright(PLAYWRIGHT_REFERENCE)]
fn reference_screenshot_over_512_kib_fails(#[case] path: &str) {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write_sized(path, REFERENCE_LIMIT + 1);

    let outcome = workspace.xtask(&["screenshots", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("oversized-reference-screenshot", path)]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn reference_screenshot_of_exactly_512_kib_passes() {
    let workspace = Workspace::from_fixture("screenshots");
    workspace.write_sized(PLAYWRIGHT_REFERENCE, REFERENCE_LIMIT);

    let outcome = workspace.xtask(&["screenshots", "check"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}
