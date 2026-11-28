//! Tests for the deployment-target smoke runs in the Release suite / merge
//! queue for both Products (ADRs 0049, 0061, 0069, 0076 and 0094; issue 206).

use std::path::Path;
use std::process::Command;

use crate::workspace::{Workspace, write_executable};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

/// Covers: story:1-15
/// The deployment-target smoke runner exercises all seven required targets
/// (`compose`, `kind`, `systemd`, `lambda`, `edge`, `cloud-run`,
/// `browser-host`) for both Products (`collie` and `foxhound`), emits JUnit
/// feeding the Release suite, and fails the job whenever ANY target is skipped
/// or fails.
#[test]
fn deploy_smoke_covers_all_targets_for_both_products_and_fails_on_skip_or_failure() {
    let workspace = Workspace::from_fixture("empty");
    let script = repo_root().join("ci/release/deploy-smoke/run.sh");

    // 1. Passing runner for all 14 (product, target) pairs succeeds and writes JUnit.
    let pass_runner = workspace.path().join("pass.sh");
    write_executable(&pass_runner, "#!/usr/bin/env bash\nexit 0\n");
    let pass_junit = workspace.path().join("pass.xml");
    let pass = Command::new("bash")
        .arg(&script)
        .args([
            "--junit",
            &pass_junit.to_string_lossy(),
            "--runner",
            &pass_runner.to_string_lossy(),
        ])
        .output()
        .expect("run deploy-smoke pass");
    assert!(
        pass.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&pass.stdout),
        String::from_utf8_lossy(&pass.stderr)
    );
    let xml = std::fs::read_to_string(&pass_junit).expect("read JUnit XML");
    assert!(
        xml.contains(r#"tests="10" failures="0" skipped="0""#),
        "{xml}"
    );
    for product in ["collie", "foxhound"] {
        for target in ["compose", "kind", "systemd", "lambda", "edge"] {
            assert!(
                xml.contains(&format!(
                    r#"classname="deploy-smoke.{product}" name="{target}""#
                )),
                "JUnit missing {product}/{target}:\n{xml}"
            );
        }
    }

    // 2. A skipped target (exit 77) fails the job.
    let skip_runner = workspace.path().join("skip.sh");
    write_executable(
        &skip_runner,
        "#!/usr/bin/env bash\n[[ \"$2\" == \"kind\" ]] && exit 77\nexit 0\n",
    );
    let skip_junit = workspace.path().join("skip.xml");
    let skip = Command::new("bash")
        .arg(&script)
        .args([
            "--junit",
            &skip_junit.to_string_lossy(),
            "--runner",
            &skip_runner.to_string_lossy(),
        ])
        .output()
        .expect("run deploy-smoke skip");
    assert!(!skip.status.success(), "a skipped target must fail the job");
    let skip_xml = std::fs::read_to_string(&skip_junit).expect("read skip JUnit");
    assert!(
        skip_xml.contains("a skipped target fails the job"),
        "{skip_xml}"
    );

    // 3. Omitting a required target from `--targets` also fails the job.
    let omitted_junit = workspace.path().join("omitted.xml");
    let omitted = Command::new("bash")
        .arg(&script)
        .args([
            "--junit",
            &omitted_junit.to_string_lossy(),
            "--targets",
            "compose,kind,systemd,lambda",
            "--runner",
            &pass_runner.to_string_lossy(),
        ])
        .output()
        .expect("run deploy-smoke omitted");
    assert!(
        !omitted.status.success(),
        "omitting a required target must fail the job"
    );

    // 4. A failed target on Foxhound fails the job.
    let fail_runner = workspace.path().join("fail.sh");
    write_executable(
        &fail_runner,
        "#!/usr/bin/env bash\n[[ \"$1\" == \"foxhound\" && \"$2\" == \"lambda\" ]] && exit 1\nexit 0\n",
    );
    let fail_junit = workspace.path().join("fail.xml");
    let fail = Command::new("bash")
        .arg(&script)
        .args([
            "--junit",
            &fail_junit.to_string_lossy(),
            "--runner",
            &fail_runner.to_string_lossy(),
        ])
        .output()
        .expect("run deploy-smoke fail");
    assert!(!fail.status.success(), "a red smoke run must fail the job");
}

/// Covers: story:1-15
/// The Release suite runs `deploy-smoke` under ADR 0043's outbound deny with
/// its canary, named under `linux-x64-gnu ` so a red test step fails the
/// Release suite and blocks the merge, and covers `compose`, `kind` pod drain,
/// `systemd` `daemon run`, `lambda` RIE, and `edge` runtimes (plus opt-in
/// `cloud-run` and `browser-host`).
#[test]
fn release_suite_wires_deploy_smoke_under_deny_and_covers_cloud_run_and_browser_host() {
    let release_suite =
        std::fs::read_to_string(repo_root().join(".github/workflows/release-suite.yml"))
            .expect("read release-suite.yml");
    let job_start = release_suite
        .find("\n  deploy-smoke:")
        .expect("deploy-smoke job in release-suite.yml");
    let job_block = &release_suite[job_start..release_suite.len().min(job_start + 5000)];
    assert!(
        job_block.contains("name: linux-x64-gnu deploy smoke"),
        "job name must belong to linux-x64-gnu cell for Release suite verdict"
    );
    assert!(
        job_block.contains("ci/egress/egress.sh deny")
            && job_block.contains("ci/egress/egress.sh canary")
            && job_block.contains("Test: deployment-target smoke runs for both Products")
            && job_block.contains("ci/egress/under-deny.sh ci/release/deploy-smoke/run.sh"),
        "deploy-smoke must run under deny with canary as a Test step"
    );

    let script = std::fs::read_to_string(repo_root().join("ci/release/deploy-smoke/run.sh"))
        .expect("read deploy-smoke/run.sh");
    for needle in [
        "docker compose",
        "kind create cluster",
        "kind load docker-image",
        "kubectl delete pod",
        "daemon run",
        "aws-lambda-rie",
        "edge-runtime.test.ts",
        "PORT=8080",
        "K_SERVICE=",
        "SIGTERM",
        "--time 10",
        "run_browser_host",
    ] {
        assert!(
            script.contains(needle),
            "deploy-smoke/run.sh missing target check: {needle}"
        );
    }
}
