//! ADR 0043's deny where a job runs on its runner's own system rather than in
//! the owner's machine's Linux VM: the Release suite's Windows cell, under
//! firewall rules that block the product's programs, whose dropped
//! connections the firewall log records.
//! Covers: story:1-68

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
fn windows_rules_block_everything_but_the_resolved_named_endpoints_and_loopback() {
    let workspace = Workspace::from_fixture("egress");
    workspace.append(
        "ci/egress/endpoints.toml",
        "[[named]]\n\
         job = \"nightly.yml#sinks\"\n\
         exception = \"saas-sinks\"\n\
         endpoints = [\"db.example.com:5432\", \"objects.example.com:443\"]\n",
    );
    workspace.write(
        "resolved.txt",
        b"db.example.com 5432 203.0.113.10\nobjects.example.com 443 2001:db8::20\n",
    );

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "nightly.yml#sinks",
        "--resolved",
        "resolved.txt",
        "--windows",
    ]);

    assert_eq!(
        outcome.stdout,
        "any 0.0.0.0-126.255.255.255,128.0.0.0-203.0.113.9,203.0.113.11-255.255.255.255,::,::2-2001:db8::1f,2001:db8::21-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff *\n\
         tcp 203.0.113.10 1-5431,5433-65535\n\
         udp 203.0.113.10 *\n\
         tcp 2001:db8::20 1-442,444-65535\n\
         udp 2001:db8::20 *\n"
    );
    assert_eq!(outcome.code, 0);
}
