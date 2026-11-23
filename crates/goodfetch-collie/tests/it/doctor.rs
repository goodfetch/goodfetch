//! `doctor` through Collie's executable (spec #608, ADR 0043): one finding
//! for each part of the installation, with a fix for each that is not in
//! order, and exit `0` whatever it finds.

use goodfetch_protocol::doctor::{DoctorReport, FindingId, FindingStatus};
use goodfetch_test_support::{TestEnv, assert_cli_snapshot, browser_cache_in_home, cli};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The settings of a home whose Runs launch Chrome for Testing and may
/// download nothing.
const OFFLINE_FULL_BUILD: &str = "browser.headless_shell = false\negress.offline = true\n";

/// Each finding's part and status.
fn statuses(report: &DoctorReport) -> Vec<(FindingId, FindingStatus)> {
    report
        .findings
        .iter()
        .map(|finding| (finding.id, finding.status))
        .collect()
}

/// A home whose Browser cache lacks the build a Run launches, with
/// `egress.offline` on, has a failing `browsers` finding with its fix;
/// `doctor` still exits `0`, under `--json` and for people.
///
/// Covers: story:11-212, command:doctor
#[test]
fn doctor_exits_0_and_prints_a_fix_for_each_failing_finding() {
    let env = TestEnv::new();
    env.user_configuration(OFFLINE_FULL_BUILD);

    let checked = cli(&env, COLLIE)
        .args(["doctor", "--json"])
        .json("DoctorReport")
        .unwrap();
    let human = cli(&env, COLLIE).args(["doctor"]).run().unwrap();

    let report: DoctorReport = serde_json::from_value(checked.documents[0].clone()).unwrap();
    let unfixed: Vec<FindingId> = report
        .findings
        .iter()
        .filter(|finding| finding.status != FindingStatus::Ok && finding.fix.is_none())
        .map(|finding| finding.id)
        .collect();
    assert_eq!(
        (checked.output.exit, human.exit, statuses(&report), unfixed),
        (
            Some(0),
            Some(0),
            vec![
                (FindingId::Home, FindingStatus::Ok),
                (FindingId::Executables, FindingStatus::Ok),
                (FindingId::Daemon, FindingStatus::Ok),
                (FindingId::VersionSkew, FindingStatus::Ok),
                (FindingId::Browsers, FindingStatus::Failing),
            ],
            vec![],
        )
    );
    assert_cli_snapshot!("doctor_json", checked.output);
    assert_cli_snapshot!("doctor_human", human);
}

/// The build a Run launches, laid out in the home's Browser cache, makes
/// the `browsers` finding `ok`, with no fix.
///
/// Covers: story:11-212, command:doctor
#[test]
fn doctor_finds_the_build_a_run_launches_in_the_browser_cache() {
    let env = TestEnv::new();
    env.user_configuration(OFFLINE_FULL_BUILD);
    browser_cache_in_home(&env).unwrap();

    let checked = cli(&env, COLLIE)
        .args(["doctor", "--json"])
        .json("DoctorReport")
        .unwrap();

    let report: DoctorReport = serde_json::from_value(checked.documents[0].clone()).unwrap();
    let browsers = report
        .findings
        .iter()
        .find(|finding| finding.id == FindingId::Browsers)
        .map(|finding| (finding.status, finding.fix.clone()));
    assert_eq!(browsers, Some((FindingStatus::Ok, None)));
}
