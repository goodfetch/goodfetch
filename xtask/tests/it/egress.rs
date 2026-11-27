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

#[rstest]
#[case::subdomain("tunnel.goodfetch.dev:443")]
#[case::trailing_dot("tunnel.goodfetch.dev.:443")]
fn integration_account_job_naming_a_goodfetch_dev_tunnel_host_fails(#[case] endpoint: &str) {
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
        &format!(
            "\n[[named]]\njob = \"live.yml#integration-accounts\"\nexception = \"integration-account\"\nendpoints = [{endpoint:?}]\n"
        ),
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

#[cfg(unix)]
mod integration_accounts_script {
    use std::path::Path;
    use std::process::Command;

    use crate::workspace::{Workspace, write_executable};

    fn script_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/integration-accounts.sh")
    }

    #[test]
    fn starts_daemon_and_quick_tunnel_exports_env_and_cleans_up_on_exit() {
        let workspace = Workspace::from_fixture("egress");
        let root = workspace.path();
        let bin = root.join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        workspace.write(
            "ci/live-checks.toml",
            b"[[live-check]]\nid = \"demo\"\narea = 11\njob = \"live.yml#integration-accounts\"\nrun = \"true\"\n",
        );

        let daemon = bin.join("daemon");
        write_executable(
            &daemon,
            r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "daemon" && "${2:-}" == "status" ]]; then
  echo '{"major":0}'
  exit 0
fi
if [[ "${1:-}" == "daemon" && "${2:-}" == "run" ]]; then
  echo "test-daemon-token" > "$GOODFETCH_HOME/0/daemon.token"
  echo "$$" > "$RECORD_DIR/daemon.pid"
  echo '{"major":0,"address":"127.0.0.1:43210"}'
  exec sleep 300
fi
exit 2
"#,
        );

        let cloudflared = bin.join("cloudflared");
        write_executable(
            &cloudflared,
            r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" > "$RECORD_DIR/cloudflared.args"
echo "$$" > "$RECORD_DIR/tunnel.pid"
echo "https://demo-quick-tunnel.trycloudflare.com" >&2
exec sleep 300
"#,
        );

        let xtask = bin.join("xtask");
        write_executable(
            &xtask,
            r#"#!/usr/bin/env bash
set -euo pipefail
printf 'URL=%s\nTOKEN=%s\n' "$GOODFETCH_INTEGRATION_TUNNEL_URL" "$GOODFETCH_INTEGRATION_DAEMON_TOKEN" > "$RECORD_DIR/env.txt"
"#,
        );

        let record = root.join("record");
        std::fs::create_dir_all(&record).unwrap();

        let output = Command::new(script_path())
            .arg("run")
            .current_dir(root)
            .env("DAEMON_BIN", &daemon)
            .env("CLOUDFLARED_BIN", &cloudflared)
            .env("XTASK_BIN", &xtask)
            .env("RECORD_DIR", &record)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "expected success, got {:?}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(record.join("env.txt")).unwrap(),
            "URL=https://demo-quick-tunnel.trycloudflare.com\nTOKEN=test-daemon-token\n"
        );
        let args = std::fs::read_to_string(record.join("cloudflared.args")).unwrap();
        assert!(args.contains("--no-autoupdate"), "{args}");
        assert!(args.contains("--origin-ca-pool"), "{args}");
        assert!(args.contains("--url https://127.0.0.1:43210"), "{args}");

        for pid_file in ["daemon.pid", "tunnel.pid"] {
            let pid = std::fs::read_to_string(record.join(pid_file)).unwrap();
            let alive = Command::new("kill")
                .args(["-0", pid.trim()])
                .status()
                .unwrap()
                .success();
            assert!(!alive, "{pid_file} ({}) was leaked after exit", pid.trim());
        }
    }

    #[test]
    fn fails_and_cleans_up_daemon_when_cloudflared_exits_without_a_url() {
        let workspace = Workspace::from_fixture("egress");
        let root = workspace.path();
        let bin = root.join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        workspace.write(
            "ci/live-checks.toml",
            b"[[live-check]]\nid = \"demo\"\narea = 11\njob = \"live.yml#integration-accounts\"\nrun = \"true\"\n",
        );

        let daemon = bin.join("daemon");
        write_executable(
            &daemon,
            r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "daemon" && "${2:-}" == "status" ]]; then
  echo '{"major":0}'
  exit 0
fi
echo "test-daemon-token" > "$GOODFETCH_HOME/0/daemon.token"
echo "$$" > "$RECORD_DIR/daemon.pid"
echo '{"major":0,"address":"127.0.0.1:43210"}'
exec sleep 300
"#,
        );

        let cloudflared = bin.join("cloudflared");
        write_executable(
            &cloudflared,
            "#!/usr/bin/env bash\necho 'tunnel failed' >&2\nexit 1\n",
        );

        let xtask = bin.join("xtask");
        write_executable(&xtask, "#!/usr/bin/env bash\nexit 0\n");

        let record = root.join("record");
        std::fs::create_dir_all(&record).unwrap();

        let output = Command::new(script_path())
            .arg("run")
            .current_dir(root)
            .env("DAEMON_BIN", &daemon)
            .env("CLOUDFLARED_BIN", &cloudflared)
            .env("XTASK_BIN", &xtask)
            .env("RECORD_DIR", &record)
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("cloudflared exited"), "{stderr}");
        let pid = std::fs::read_to_string(record.join("daemon.pid")).unwrap();
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "daemon ({}) was leaked after failure", pid.trim());
    }
}
