//! The fixture-size check: no fixture file exceeds 256 KiB (ADR 0062).
//! Covers: story:1-107

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// TESTING.md's limit, 256 KiB.
const LIMIT: usize = 262_144;

#[rstest]
#[case::beside_a_crates_tests("crates/goodfetch-engine/tests/it/fixtures/page.html")]
#[case::in_shared_test_support("test-support/fixtures/corpus/page.html")]
// ci/proofs/ holds the static checks' fixtures, though no folder there is
// named `fixtures`.
#[case::of_a_static_checks_proof("ci/proofs/insta/missing/src/lib.rs")]
fn fixture_over_256_kib_fails(#[case] path: &str) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(path, &vec![b'x'; LIMIT + 1]);

    let outcome = workspace.xtask(&["fixture-size"]);

    assert_eq!(outcome.findings(), vec![finding("oversized-fixture", path)]);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::fixture_of_exactly_256_kib("test-support/fixtures/page.html", LIMIT)]
#[case::large_file_outside_a_fixtures_folder("crates/goodfetch-engine/src/table.bin", LIMIT * 2)]
#[case::frozen_research("rnd/lab/fixtures/page.html", LIMIT * 2)]
#[case::vitest_reference_screenshot(
    "test-support/fixtures/pages/__screenshots__/page.visual.test.ts/page-chromium.png",
    LIMIT * 2
)]
#[case::playwright_reference_screenshot(
    "test-support/fixtures/pages/page.visual.spec.ts-snapshots/page-visual.png",
    LIMIT * 2
)]
fn file_within_the_rule_passes(#[case] path: &str, #[case] size: usize) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(path, &vec![b'x'; size]);

    let outcome = workspace.xtask(&["fixture-size"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}
