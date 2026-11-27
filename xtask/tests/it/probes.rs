//! Seam A (`xtask probes generate` and `check`, ADR 0067, ticket #429):
//! one probe case per confinement rule and network level, plus the browser
//! sandbox startup probe, with positive controls and drift checks.
//! Declares no Feature (#325, Out of Scope, ambiguity 9).

use std::path::Path;

use crate::workspace::{Workspace, finding};

const SAMPLE_POLICY: &str = r#"
os = "linux"
mechanisms = ["landlock", "pid_namespace", "mount_namespace"]
signal_rule = "signals.outside"
file_rules = [
    "files.read_bundle",
    "files.read_runtime",
    "files.write_instance",
    "files.read_elsewhere",
    "files.write_elsewhere",
    "files.write_bundle",
]

[[levels]]
name = "none"

[[levels.seccomp]]
name = "brk"
call = "brk"
verdict = "allow"

[[levels.seccomp]]
name = "clone3"
call = "clone3"
verdict = "not_implemented"

[[levels.seccomp]]
name = "ptrace"
call = "ptrace"
verdict = "refuse"

[[levels.seccomp]]
name = "socket"
call = "socket"
verdict = "refuse"
socket_families = 3

[[levels]]
name = "proxy_tcp"

[[levels.seccomp]]
name = "brk"
call = "brk"
verdict = "allow"

[[levels.seccomp]]
name = "ptrace"
call = "ptrace"
verdict = "refuse"

[[deny_list]]
rule = "deny.ssh"
base = "home"
path = ".ssh"

[[deny_list]]
rule = "deny.env_file"
base = "anywhere"
path = ".env"
"#;

fn sample_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("policy.toml", SAMPLE_POLICY.as_bytes());
    workspace
}

#[test]
fn a_sample_policy_generates_one_case_per_rule_and_network_level_and_passes_check() {
    let workspace = sample_workspace();

    let generated = workspace.xtask(&[
        "probes",
        "generate",
        "--policy",
        "policy.toml",
        "--out",
        "probes",
    ]);
    assert_eq!(
        generated.code, 0,
        "{}{}",
        generated.stdout, generated.stderr
    );

    let none_text = std::fs::read_to_string(workspace.path().join("probes/linux/none.toml"))
        .expect("linux/none.toml generated");
    let proxy_text = std::fs::read_to_string(workspace.path().join("probes/linux/proxy_tcp.toml"))
        .expect("linux/proxy_tcp.toml generated");

    assert!(none_text.contains(r#"rule = "brk""#), "{none_text}");
    assert!(none_text.contains(r#"rule = "clone3""#), "{none_text}");
    assert!(none_text.contains(r#"rule = "ptrace""#), "{none_text}");
    assert!(
        none_text.contains(r#"rule = "signals.outside""#)
            && none_text.contains(r#"pid_namespace = "ESRCH""#),
        "{none_text}"
    );
    assert!(none_text.contains(r#"rule = "deny.ssh""#), "{none_text}");
    assert!(
        none_text.contains(r#"rule = "deny.env_file""#),
        "{none_text}"
    );
    assert!(
        none_text.contains(r#"rule = "files.write_bundle""#),
        "{none_text}"
    );
    assert_eq!(
        none_text.matches(r#"rule = "socket""#).count(),
        3,
        "one case per declared socket family"
    );
    assert!(proxy_text.contains(r#"rule = "brk""#), "{proxy_text}");
    assert!(proxy_text.contains(r#"rule = "ptrace""#), "{proxy_text}");
    assert!(
        !proxy_text.contains(r#"rule = "socket""#),
        "proxy_tcp has no socket refusal in the sample policy"
    );

    let checked = workspace.xtask(&[
        "probes",
        "check",
        "--policy",
        "policy.toml",
        "--probes",
        "probes",
    ]);
    assert_eq!(checked.code, 0, "{}{}", checked.stdout, checked.stderr);
    assert_eq!(checked.findings(), []);
}

#[test]
fn a_rule_removed_from_the_sample_policy_leaves_an_orphaned_probe_and_check_fails() {
    let workspace = sample_workspace();
    let generated = workspace.xtask(&[
        "probes",
        "generate",
        "--policy",
        "policy.toml",
        "--out",
        "probes",
    ]);
    assert_eq!(
        generated.code, 0,
        "{}{}",
        generated.stdout, generated.stderr
    );

    workspace.edit(
        "policy.toml",
        "[[levels.seccomp]]\nname = \"clone3\"\ncall = \"clone3\"\nverdict = \"not_implemented\"\n",
        "",
    );
    workspace.edit(
        "policy.toml",
        "[[deny_list]]\nrule = \"deny.ssh\"\nbase = \"home\"\npath = \".ssh\"\n",
        "",
    );

    let checked = workspace.xtask(&[
        "probes",
        "check",
        "--policy",
        "policy.toml",
        "--probes",
        "probes",
    ]);
    assert_eq!(checked.code, 1, "{}{}", checked.stdout, checked.stderr);
    let findings = checked.findings();
    assert!(
        findings.contains(&finding("probe-without-rule", "linux/none:clone3")),
        "expected orphaned seccomp probe finding, got {findings:?}"
    );
    assert!(
        findings.contains(&finding("probe-without-rule", "linux/none:deny.ssh")),
        "expected orphaned deny-list probe finding, got {findings:?}"
    );
}

#[test]
fn a_rule_added_without_regenerating_fails_check() {
    let workspace = sample_workspace();
    let generated = workspace.xtask(&[
        "probes",
        "generate",
        "--policy",
        "policy.toml",
        "--out",
        "probes",
    ]);
    assert_eq!(
        generated.code, 0,
        "{}{}",
        generated.stdout, generated.stderr
    );

    let mut policy = std::fs::read_to_string(workspace.path().join("policy.toml")).unwrap();
    policy.push_str("\n[[deny_list]]\nrule = \"deny.aws\"\nbase = \"home\"\npath = \".aws\"\n");
    policy = policy.replace(
        "name = \"proxy_tcp\"\n",
        "name = \"proxy_tcp\"\n\n[[levels.seccomp]]\nname = \"unshare\"\ncall = \"unshare\"\nverdict = \"refuse\"\n",
    );
    workspace.write("policy.toml", policy.as_bytes());

    let checked = workspace.xtask(&[
        "probes",
        "check",
        "--policy",
        "policy.toml",
        "--probes",
        "probes",
    ]);
    assert_eq!(checked.code, 1, "{}{}", checked.stdout, checked.stderr);
    let findings = checked.findings();
    assert!(
        findings.contains(&finding("rule-without-probe", "linux/proxy_tcp:unshare")),
        "expected missing seccomp probe finding, got {findings:?}"
    );
    assert!(
        findings.contains(&finding("rule-without-probe", "linux/none:deny.aws")),
        "expected missing deny-list probe finding, got {findings:?}"
    );
}

#[test]
fn a_refusal_probe_without_a_positive_control_fails_check() {
    let workspace = sample_workspace();
    let generated = workspace.xtask(&[
        "probes",
        "generate",
        "--policy",
        "policy.toml",
        "--out",
        "probes",
    ]);
    assert_eq!(
        generated.code, 0,
        "{}{}",
        generated.stdout, generated.stderr
    );

    workspace.edit(
        "probes/linux/none.toml",
        r#"{ rule = "ptrace", call = "ptrace", args = [16, 0, 0, 0], unconfined = "ESRCH", confined = "EPERM" }"#,
        r#"{ rule = "ptrace", call = "ptrace", args = [16, 0, 0, 0], unconfined = "EPERM", confined = "EPERM" }"#,
    );

    let checked = workspace.xtask(&[
        "probes",
        "check",
        "--policy",
        "policy.toml",
        "--probes",
        "probes",
    ]);
    assert_eq!(checked.code, 1, "{}{}", checked.stdout, checked.stderr);
    assert!(
        checked
            .findings()
            .contains(&finding("no-positive-control", "linux/none:ptrace")),
        "expected no-positive-control finding, got {:?}",
        checked.findings()
    );
}

#[test]
fn the_workspace_s_committed_confinement_and_browser_sandbox_probes_pass_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits under the workspace root");
    let workspace = Workspace::from_fixture("empty");
    // Run against the real workspace root through GOODFETCH_WORKSPACE_ROOT? No,
    // xtask runs in its cwd, so run the binary directly in `root`.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(root)
        .args(["probes", "check"])
        .output()
        .expect("running xtask probes check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let _ = workspace;
}
