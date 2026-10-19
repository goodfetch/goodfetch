//! The resident-memory check (ADRs 0071, 0094 and 0134): `xtask rss check`
//! holds each scenario's peak, measured on one Product's executable on one
//! platform, to the committed ceiling for that Product, platform and
//! scenario, and reports without gating while no ceilings file exists;
//! `xtask rss propose` turns 20 runs into ceilings at 1.5 times the highest
//! peak, rounded up to a whole MiB. Peaks and ceilings are fixtures here.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const MIB: u64 = 1024 * 1024;

/// The ceilings file the check reads by default.
const CEILINGS: &str = "ci/rss-ceilings.toml";

/// One peak of the peaks file, as `xtask rss measure` writes it.
fn peak(product: &str, platform: &str, scenario: &str, bytes: u64) -> String {
    format!(
        "{{\"product\":\"{product}\",\"platform\":\"{platform}\",\"scenario\":\"{scenario}\",\"bytes\":{bytes}}}\n"
    )
}

fn write_peaks(workspace: &Workspace, peaks: &[String]) {
    workspace.write("target/rss/peaks.jsonl", peaks.concat().as_bytes());
}

fn check(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["rss", "check", "--peaks", "target/rss/peaks.jsonl"])
}

#[rstest]
#[case::under_its_ceiling(5 * MIB)]
#[case::at_its_ceiling(6 * MIB)]
fn a_peak_at_or_under_its_ceiling_passes(#[case] bytes: u64) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.macos-arm64]\nidle-attached-engine = 6\n",
    );
    write_peaks(
        &workspace,
        &[peak("collie", "macos-arm64", "idle-attached-engine", bytes)],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_peak_over_its_ceiling_fails() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.macos-arm64]\nidle-attached-engine = 6\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            "collie",
            "macos-arm64",
            "idle-attached-engine",
            6 * MIB + 1,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "rss-ceiling",
            "collie idle-attached-engine macos-arm64"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}
