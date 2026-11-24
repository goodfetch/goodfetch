//! `xtask chromium expectations regenerate` (ADR 0074; TESTING.md, The
//! Goodfetch Chromium): the generated `upstream-flaky` files, written from
//! LUCI Analysis's answers, recorded here, each with the command that made
//! it and its tracking issue, opened through a stand-in of GitHub's
//! interface. The local tooling declares no Feature.

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace};

const LINUX: &str = "chromium/expectations/gtest/generated/unit_tests-linux.filter";
const MAC: &str = "chromium/expectations/gtest/generated/unit_tests-mac.filter";
const WINDOWS: &str = "chromium/expectations/gtest/generated/unit_tests-windows.filter";

const SUMMARIES: &str = "/prpc/luci.analysis.v1.Clusters/QueryClusterSummaries";
const FAILURES: &str = "/prpc/luci.analysis.v1.Clusters/QueryClusterFailures";
const STATS: &str = "/prpc/luci.analysis.v1.TestHistory/QueryStats";
const VARIANTS: &str = "/prpc/luci.analysis.v1.TestHistory/QueryVariants";
const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";

/// The clusters, as LUCI Analysis names them: one test of the suite, one of
/// another suite, a failure reason, and a parameterised family.
const CLUSTERS: &str = r#")]}'
{"clusterSummaries": [
  {"clusterId": {"algorithm": "testname-v4", "id": "alpha"}, "title": "://chrome/test\\\\:unit_tests!gtest::AlphaTest#Flakes", "metrics": {"failures": {"value": "40"}}},
  {"clusterId": {"algorithm": "testname-v4", "id": "gamma"}, "title": "://chrome/test\\\\:browser_tests!gtest::GammaTest#Other", "metrics": {"failures": {"value": "30"}}},
  {"clusterId": {"algorithm": "reason-v6", "id": "reason"}, "title": "clipboard.cc(%): Check failed", "metrics": {"failures": {"value": "20"}}},
  {"clusterId": {"algorithm": "testname-v4", "id": "family"}, "title": "://chrome/test\\\\\\\\:unit\\\\_tests!gtest::BetaTest#Case%", "metrics": {"failures": {"value": "10"}}}
]}"#;

const FAMILY: &str = r#")]}'
{"failures": [
  {"testId": "://chrome/test\\:unit_tests!gtest::BetaTest#Case/All.Value", "count": 3},
  {"testId": "://chrome/test\\:browser_tests!gtest::BetaTest#Case/All.Value", "count": 1}
]}"#;

/// `AlphaTest` flakes in 2% of its Linux verdicts, under 1% on Windows,
/// and often on a `ChromeOS` bot, a configuration Goodfetch never builds.
const ALPHA: &str = r#")]}'
{"groups": [
  {"partitionTime": "2026-11-22T00:00:00Z", "variantHash": "lin", "verdictCounts": {"passed": 50, "flaky": 1}},
  {"partitionTime": "2026-11-23T00:00:00Z", "variantHash": "lin", "verdictCounts": {"passed": 48, "flaky": 1}},
  {"partitionTime": "2026-11-23T00:00:00Z", "variantHash": "win", "verdictCounts": {"passed": 999, "failed": 1}},
  {"partitionTime": "2026-11-23T00:00:00Z", "variantHash": "chromeos", "verdictCounts": {"flaky": 50}}
]}"#;

/// `BetaTest` flakes in 10% of 10 verdicts on macOS, and fails on Linux in
/// too few verdicts to count, skipped ones never counting.
const BETA: &str = r#")]}'
{"groups": [
  {"partitionTime": "2026-11-23T00:00:00Z", "variantHash": "mac", "verdictCounts": {"passed": 9, "flaky": 1}},
  {"partitionTime": "2026-11-23T00:00:00Z", "variantHash": "lin", "verdictCounts": {"passed": 5, "failed": 1, "skipped": 4}}
]}"#;

const BOTS: &str = r#")]}'
{"variants": [
  {"variantHash": "lin", "variant": {"def": {"bucket": "ci", "builder": "Linux Tests", "os": "Ubuntu-22.04", "test_suite": "unit_tests"}}},
  {"variantHash": "win", "variant": {"def": {"bucket": "ci", "builder": "Win11 Tests x64", "os": "Windows-11", "test_suite": "unit_tests"}}},
  {"variantHash": "mac", "variant": {"def": {"bucket": "ci", "builder": "Mac15 Tests", "os": "Mac-15", "test_suite": "unit_tests"}}},
  {"variantHash": "chromeos", "variant": {"def": {"bucket": "ci", "builder": "linux-chromeos-rel", "os": "Ubuntu-22.04", "test_suite": "unit_tests"}}}
]}"#;

fn luci() -> GitHub {
    GitHub::serving(&[
        (SUMMARIES, CLUSTERS),
        (
            &format!("POST {FAILURES} clusters/testname-v4/family/failures"),
            FAMILY,
        ),
        (&format!("POST {STATS} AlphaTest#Flakes\""), ALPHA),
        (&format!("POST {STATS} BetaTest#Case/All.Value\""), BETA),
        (VARIANTS, BOTS),
    ])
}

fn github() -> GitHub {
    GitHub::serving(&[
        (
            &format!("POST {ISSUES} unit_tests-linux.filter"),
            r#"{"number": 41}"#,
        ),
        (
            &format!("POST {ISSUES} unit_tests-mac.filter"),
            r#"{"number": 42}"#,
        ),
    ])
}

fn regenerate(workspace: &Workspace, luci: &GitHub, github: &GitHub) -> Outcome {
    workspace.xtask_with_env(
        &[
            "chromium",
            "expectations",
            "regenerate",
            "--today",
            "2026-11-24",
            "--luci-url",
            luci.url(),
            "--api-url",
            github.url(),
            "unit_tests",
        ],
        &[("GITHUB_TOKEN", "a-write-token".as_ref())],
    )
}

/// The titles of the issues opened.
fn opened(github: &GitHub) -> Vec<String> {
    github
        .writes()
        .iter()
        .map(|request| {
            assert_eq!(
                (request.method.as_str(), request.path.as_str()),
                ("POST", ISSUES)
            );
            let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
            body["title"].as_str().unwrap().to_owned()
        })
        .collect()
}

fn generated(issue: u32, entry: &str) -> String {
    format!(
        "# Generated by: xtask chromium expectations regenerate --days 30 --min-flake-rate 1 unit_tests\n# Tracking issue: #{issue}\n# Do not edit: the command above rewrites this file from LUCI Analysis.\n\n{entry}"
    )
}

#[test]
fn each_platforms_file_is_written_with_its_command_and_its_tracking_issue() {
    let (luci, github) = (luci(), github());
    let workspace = Workspace::from_fixture("empty");

    let outcome = regenerate(&workspace, &luci, &github);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        workspace.read_text(LINUX),
        generated(
            41,
            "# upstream-flaky rate=2.0% passed=98 failed=0 flaky=2\n-AlphaTest.Flakes\n"
        )
    );
    assert_eq!(
        workspace.read_text(MAC),
        generated(
            42,
            "# upstream-flaky rate=10.0% passed=9 failed=0 flaky=1\n-All/BetaTest.Case/Value\n"
        )
    );
    assert_eq!(workspace.read(WINDOWS), None);
    assert_eq!(
        opened(&github),
        [
            "Upstream-flaky exclusions: unit_tests on linux",
            "Upstream-flaky exclusions: unit_tests on mac"
        ]
    );
    let checked = workspace.xtask(&["chromium", "expectations", "check", "--today", "2026-11-24"]);
    assert_eq!(checked.code, 0, "{}{}", checked.stdout, checked.stderr);
}

#[test]
fn a_regenerated_file_keeps_its_issue_and_a_platform_left_without_entries_loses_its_file() {
    let (luci, github) = (luci(), github());
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        LINUX,
        generated(
            7,
            "# upstream-flaky rate=3.0% passed=97 failed=3 flaky=0\n-OldTest.Gone\n",
        )
        .as_bytes(),
    );
    workspace.write(
        WINDOWS,
        generated(
            8,
            "# upstream-flaky rate=1.0% passed=99 failed=1 flaky=0\n-OldTest.Gone\n",
        )
        .as_bytes(),
    );

    let outcome = regenerate(&workspace, &luci, &github);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        workspace.read_text(LINUX),
        generated(
            7,
            "# upstream-flaky rate=2.0% passed=98 failed=0 flaky=2\n-AlphaTest.Flakes\n"
        )
    );
    assert_eq!(workspace.read(WINDOWS), None);
    assert_eq!(
        opened(&github),
        ["Upstream-flaky exclusions: unit_tests on mac"]
    );
}

#[test]
fn an_error_from_luci_analysis_writes_nothing_and_opens_nothing() {
    let luci = GitHub::serving(&[(SUMMARIES, CLUSTERS), (FAILURES, FAMILY), (VARIANTS, BOTS)]);
    let github = github();
    let workspace = Workspace::from_fixture("empty");

    let outcome = regenerate(&workspace, &luci, &github);

    assert_eq!(outcome.code, 2, "{}{}", outcome.stdout, outcome.stderr);
    assert!(outcome.stderr.contains(STATS), "{}", outcome.stderr);
    assert_eq!(workspace.read(LINUX), None);
    assert_eq!(workspace.read(MAC), None);
    assert_eq!(opened(&github), Vec::<String>::new());
}
