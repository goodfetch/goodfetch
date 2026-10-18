//! ADR 0043's deny where a job runs on its runner's own system rather than in
//! the owner's machine's Linux VM: the Release suite's macOS cells, under a
//! sandbox profile that refuses every connection but loopback, and its
//! Windows cell, under firewall rules that block the product's programs,
//! whose dropped connections the firewall log records.
//! Covers: story:1-68

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

/// The Windows firewall log as `Set-NetFirewallProfile -LogBlocked True`
/// writes it, with the canary's drop and an inbound drop no job made.
const FIREWALL_LOG: &str = "#Version: 1.5\r\n\
#Software: Microsoft Windows Firewall\r\n\
#Time Format: Local\r\n\
#Fields: date time action protocol src-ip dst-ip src-port dst-port size tcpflags tcpsyn tcpack tcpwin icmptype icmpcode info path pid\r\n\
\r\n\
2026-10-18 10:00:01 DROP TCP 10.1.0.4 1.1.1.1 50123 443 0 - 0 0 0 - - - SEND 4242\r\n\
2026-10-18 10:00:02 DROP UDP 10.1.0.9 10.1.0.4 5353 5353 0 - - - - - - - RECEIVE 4\r\n";

fn windows_verdict(workspace: &Workspace, outcome: &str) -> Outcome {
    workspace.write("outcome.txt", outcome.as_bytes());
    workspace.xtask(&[
        "egress",
        "canary",
        "verdict",
        "--outcome",
        "outcome.txt",
        "--firewall-log",
        "pfirewall.log",
    ])
}

#[test]
fn windows_canary_the_firewall_dropped_passes() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("pfirewall.log", FIREWALL_LOG.as_bytes());

    let outcome = windows_verdict(&workspace, "denied");

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn windows_canary_missing_from_the_firewall_log_fails() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write(
        "pfirewall.log",
        FIREWALL_LOG
            .replace("1.1.1.1 50123 443", "9.9.9.9 50123 443")
            .as_bytes(),
    );

    let outcome = windows_verdict(&workspace, "denied");

    assert_eq!(
        outcome.findings(),
        vec![finding("canary-unseen", "1.1.1.1:443")]
    );
    assert_eq!(outcome.code, 1);
}

fn windows_report(workspace: &Workspace, log: &str) -> Outcome {
    workspace.write("pfirewall.log", log.as_bytes());
    workspace.xtask(&["egress", "report", "--firewall-log", "pfirewall.log"])
}

#[test]
fn windows_job_whose_only_drop_was_the_canary_passes() {
    let workspace = Workspace::from_fixture("egress");

    let outcome = windows_report(&workspace, FIREWALL_LOG);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn windows_job_each_other_dropped_destination_fails_once() {
    let workspace = Workspace::from_fixture("egress");
    let log = format!(
        "{FIREWALL_LOG}\
         2026-10-18 10:01:00 DROP TCP 10.1.0.4 93.184.215.14 50200 443 0 - 0 0 0 - - - SEND 5100\r\n\
         2026-10-18 10:01:01 DROP TCP 10.1.0.4 93.184.215.14 50201 443 0 - 0 0 0 - - - SEND 5100\r\n\
         2026-10-18 10:01:02 DROP UDP 10.1.0.4 8.8.8.8 53122 53 0 - - - - - - - SEND 5100\r\n"
    );

    let outcome = windows_report(&workspace, &log);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("egress-denied", "tcp 93.184.215.14:443"),
            finding("egress-denied", "udp 8.8.8.8:53"),
        ]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn windows_rules_block_everything_but_the_resolved_mirror_and_loopback() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write(
        "resolved.txt",
        b"github.com 443 140.82.121.4\nrelease-assets.githubusercontent.com 443 2606:50c0:8000::154\n",
    );

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "pull-request.yml#quick",
        "--resolved",
        "resolved.txt",
        "--windows",
    ]);

    assert_eq!(
        outcome.stdout,
        "any 0.0.0.0-126.255.255.255,128.0.0.0-140.82.121.3,140.82.121.5-255.255.255.255,::,::2-2606:50c0:8000::153,2606:50c0:8000::155-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff *\n\
         tcp 140.82.121.4 1-442,444-65535\n\
         udp 140.82.121.4 *\n\
         tcp 2606:50c0:8000::154 1-442,444-65535\n\
         udp 2606:50c0:8000::154 *\n"
    );
    assert_eq!(outcome.code, 0);
}

fn sandbox_verdict(workspace: &Workspace, outcome: &str) -> Outcome {
    workspace.write("outcome.txt", outcome.as_bytes());
    workspace.xtask(&[
        "egress",
        "canary",
        "verdict",
        "--outcome",
        "outcome.txt",
        "--sandbox",
    ])
}

#[rstest]
#[case::denied_by_the_sandbox("denied", vec![])]
#[case::timed_out_somewhere_else("timed-out", vec![finding("canary-unseen", "1.1.1.1:443")])]
#[case::connected("connected", vec![finding("canary-reached", "1.1.1.1:443")])]
fn macos_canary_passes_only_when_the_sandbox_refused_it(
    #[case] ended: &str,
    #[case] expected: Vec<crate::workspace::Finding>,
) {
    let workspace = Workspace::from_fixture("egress");

    let outcome = sandbox_verdict(&workspace, ended);

    let code = i32::from(!expected.is_empty());
    assert_eq!(outcome.findings(), expected);
    assert_eq!(outcome.code, code);
}
