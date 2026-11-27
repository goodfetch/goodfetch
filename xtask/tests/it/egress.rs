//! Which jobs run under ADR 0043's outbound-traffic deny in CI: every job is
//! declared under it or outside it, and a job under it denies before its
//! first test step, runs the Egress canary right after, and checks the
//! deny's log at its end.
//! Covers: story:1-69

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn declared_jobs_with_the_deny_before_their_tests_pass() {
    let workspace = Workspace::from_fixture("egress");

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn job_not_declared_under_or_outside_the_deny_fails() {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file(
        "egress-cases/undeclared-job.yml",
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("undeclared-job", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::without_the_deny("quick-without-deny.yml", "missing-deny", "pull-request.yml#quick")]
#[case::test_step_before_the_deny(
    "quick-test-before-deny.yml",
    "test-before-deny",
    "pull-request.yml#quick"
)]
#[case::canary_not_right_after_the_deny(
    "quick-canary-not-after-deny.yml",
    "missing-canary",
    "pull-request.yml#quick"
)]
#[case::log_check_skipped_on_failure(
    "quick-check-not-always.yml",
    "missing-egress-check",
    "pull-request.yml#quick"
)]
#[case::test_step_run_outside_the_deny(
    "quick-test-outside-deny.yml",
    "test-outside-deny",
    "pull-request.yml#quick"
)]
#[case::outside_job_running_the_deny(
    "all-green-with-deny.yml",
    "deny-outside-rule",
    "pull-request.yml#all-green"
)]
fn job_breaking_the_deny_order_fails(
    #[case] workflow: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file(
        &format!("egress-cases/{workflow}"),
        ".github/workflows/pull-request.yml",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::declared_twice(
    "[[outside]]\njob = \"pull-request.yml#quick\"\nreason = \"It runs none of the product.\"\n",
    "declared-twice",
    "pull-request.yml#quick"
)]
#[case::declared_without_a_job(
    "[[deny]]\njob = \"release.yml#test\"\n",
    "unknown-job",
    "release.yml#test"
)]
fn declaration_that_matches_no_single_job_fails(
    #[case] declaration: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("egress");
    workspace.append("ci/egress/endpoints.toml", declaration);

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}

/// An `integration-account` job with a `vars.<NAME> == 'true'` gate and Quick
/// Tunnel endpoints passes; dropping or bypassing the variable gate, or naming
/// a `goodfetch.dev` tunnel host, fails (ADRs 0043, 0070 and 0076; issue 428).
#[test]
fn integration_account_job_gated_by_a_repository_variable_passes() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write(
        ".github/workflows/live.yml",
        br#"name: live
on:
  workflow_dispatch:
permissions: {}
jobs:
  integration-accounts:
    if: vars.GOODFETCH_INTEGRATION_ACCOUNTS_ENABLED == 'true' && (github.event_name == 'schedule' || inputs.checks == 'integration-accounts')
    runs-on: ubuntu-24.04
    steps:
      - name: Deny outbound traffic
        run: ci/egress/egress.sh deny
      - name: Refuse the Egress canary
        run: ci/egress/egress.sh canary
      - name: "Test: the job's Live checks"
        run: ci/egress/under-deny.sh ci/integration-accounts.sh run
      - name: Check that the deny refused nothing else
        if: always()
        run: ci/egress/egress.sh check
"#,
    );
    workspace.append(
        "ci/egress/endpoints.toml",
        "\n[[named]]\njob = \"live.yml#integration-accounts\"\nexception = \"integration-account\"\nendpoints = [\n  \"api.trycloudflare.com:443\",\n  \"region1.v2.argotunnel.com:7844\",\n  \"region2.v2.argotunnel.com:7844\",\n]\n",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::no_condition("")]
#[case::no_repository_variable("    if: inputs.checks == 'integration-accounts'\n")]
#[case::or_bypasses_variable(
    "    if: vars.GOODFETCH_INTEGRATION_ACCOUNTS_ENABLED == 'true' || inputs.checks == 'integration-accounts'\n"
)]
#[case::negated_variable("    if: vars.GOODFETCH_INTEGRATION_ACCOUNTS_ENABLED != 'false'\n")]
fn integration_account_job_not_gated_by_a_repository_variable_fails(#[case] condition: &str) {
    let workspace = Workspace::from_fixture("egress");
    workspace.write(
        ".github/workflows/live.yml",
        format!(
            "name: live\non:\n  workflow_dispatch:\npermissions: {{}}\njobs:\n  integration-accounts:\n{condition}    runs-on: ubuntu-24.04\n    steps:\n      - name: Deny outbound traffic\n        run: ci/egress/egress.sh deny\n      - name: Refuse the Egress canary\n        run: ci/egress/egress.sh canary\n      - name: \"Test: the job's Live checks\"\n        run: ci/egress/under-deny.sh ci/integration-accounts.sh run\n      - name: Check that the deny refused nothing else\n        if: always()\n        run: ci/egress/egress.sh check\n"
        )
        .as_bytes(),
    );
    workspace.append(
        "ci/egress/endpoints.toml",
        "\n[[named]]\njob = \"live.yml#integration-accounts\"\nexception = \"integration-account\"\nendpoints = [\"api.trycloudflare.com:443\"]\n",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "integration-account-unprotected",
            "live.yml#integration-accounts"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn integration_account_job_naming_a_goodfetch_dev_tunnel_host_fails() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write(
        ".github/workflows/live.yml",
        br#"name: live
on:
  workflow_dispatch:
permissions: {}
jobs:
  integration-accounts:
    if: vars.GOODFETCH_INTEGRATION_ACCOUNTS_ENABLED == 'true'
    runs-on: ubuntu-24.04
    steps:
      - name: Deny outbound traffic
        run: ci/egress/egress.sh deny
      - name: Refuse the Egress canary
        run: ci/egress/egress.sh canary
      - name: "Test: the job's Live checks"
        run: ci/egress/under-deny.sh ci/integration-accounts.sh run
      - name: Check that the deny refused nothing else
        if: always()
        run: ci/egress/egress.sh check
"#,
    );
    workspace.append(
        "ci/egress/endpoints.toml",
        "\n[[named]]\njob = \"live.yml#integration-accounts\"\nexception = \"integration-account\"\nendpoints = [\"tunnel.goodfetch.dev:443\"]\n",
    );

    let outcome = workspace.xtask(&["egress", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "named-tunnel-host",
            "live.yml#integration-accounts"
        )]
    );
    assert_eq!(outcome.code, 1);
}
