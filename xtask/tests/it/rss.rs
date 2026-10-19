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

#[test]
fn with_no_ceilings_file_the_check_reports_and_gates_nothing() {
    let workspace = Workspace::from_fixture("empty");
    write_peaks(
        &workspace,
        &[
            peak("collie", "linux-x64", "idle-attached-engine", 3 * MIB),
            peak("foxhound", "linux-x64", "idle-attached-engine", 9 * MIB),
        ],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(outcome.stdout, @"
    ## Resident memory

    No ceilings file at `ci/rss-ceilings.toml`: the check reports and gates nothing until the owner approves the ceilings (ADR 0071).

    | Product | Scenario | Platform | Peak | Ceiling | Verdict | Performance target |
    |---|---|---|---:|---:|---|---|
    | collie | idle-attached-engine | linux-x64 | 3.00 MiB | none | not gated | 8.00 MiB: met |
    | foxhound | idle-attached-engine | linux-x64 | 9.00 MiB | none | not gated | 8.00 MiB: missed by 1.00 MiB |

    A Performance target informs the owner test and never fails a change (ADR 0134).
    ");
}

#[rstest]
#[case::collie_under_its_own_ceiling("collie", vec![])]
#[case::foxhound_over_its_own_ceiling(
    "foxhound",
    vec![finding("rss-ceiling", "foxhound idle-attached-engine macos-arm64")]
)]
fn each_peak_is_held_to_its_own_products_ceiling(
    #[case] product: &str,
    #[case] expected: Vec<crate::workspace::Finding>,
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.macos-arm64]\nidle-attached-engine = 20\n\n[foxhound.macos-arm64]\nidle-attached-engine = 4\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            product,
            "macos-arm64",
            "idle-attached-engine",
            10 * MIB,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
}

#[test]
fn a_collie_peak_is_never_judged_against_foxhounds_ceiling() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[foxhound.macos-arm64]\nidle-attached-engine = 4\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            "collie",
            "macos-arm64",
            "idle-attached-engine",
            10 * MIB,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_peak_is_held_to_the_ceiling_of_the_platform_it_was_measured_on() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.linux-x64]\nidle-attached-engine = 4\n\n[collie.windows-x64]\nidle-attached-engine = 20\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            "collie",
            "windows-x64",
            "idle-attached-engine",
            10 * MIB,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
}

#[test]
fn the_check_saves_each_peak_with_its_ceiling_for_the_drift_check() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(CEILINGS, b"[collie.linux-x64]\nidle-attached-engine = 6\n");
    write_peaks(
        &workspace,
        &[
            peak("collie", "linux-x64", "idle-attached-engine", 3 * MIB),
            peak("foxhound", "linux-x64", "idle-attached-engine", 5 * MIB),
        ],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let figures: serde_json::Value =
        serde_json::from_str(&workspace.read_text("target/junit/rss/perf-figures.json")).unwrap();
    assert_eq!(
        figures,
        serde_json::json!({
            "figures": [
                {
                    "subject": "rss/collie/idle-attached-engine",
                    "figure": "resident-memory",
                    "value": 3_145_728,
                    "ceiling": 6_291_456,
                },
                {
                    "subject": "rss/foxhound/idle-attached-engine",
                    "figure": "resident-memory",
                    "value": 5_242_880,
                },
            ],
        })
    );
}

#[test]
fn the_check_writes_each_peak_as_a_junit_test_case_with_its_figures() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(CEILINGS, b"[collie.linux-x64]\nidle-attached-engine = 6\n");
    write_peaks(
        &workspace,
        &[
            peak("collie", "linux-x64", "idle-attached-engine", 7 * MIB),
            peak("foxhound", "linux-x64", "idle-attached-engine", 5 * MIB),
        ],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    insta::assert_snapshot!(workspace.read_text("target/junit/rss.xml"), @r#"
    <?xml version="1.0" encoding="UTF-8"?>
    <testsuites name="rss" tests="2" failures="1">
      <testsuite name="rss" tests="2" failures="1">
        <testcase classname="rss" name="collie idle-attached-engine" time="0">
          <properties>
            <property name="platform" value="linux-x64"/>
            <property name="peak-bytes" value="7340032"/>
            <property name="ceiling-bytes" value="6291456"/>
            <property name="target-bytes" value="8388608"/>
          </properties>
          <failure message="peak of 7.00 MiB over its ceiling of 6.00 MiB">peak of 7.00 MiB over its ceiling of 6.00 MiB</failure>
        </testcase>
        <testcase classname="rss" name="foxhound idle-attached-engine" time="0">
          <properties>
            <property name="platform" value="linux-x64"/>
            <property name="peak-bytes" value="5242880"/>
            <property name="target-bytes" value="8388608"/>
          </properties>
        </testcase>
      </testsuite>
    </testsuites>
    "#);
}
