//! The Egress canary's verdict, and the end-of-job check that the deny
//! refused nothing but the canary (ADR 0043), read from the deny's nftables
//! counters and kernel log as the job's VM writes them.
//! Covers: story:1-68

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// `nft --json list counters table inet goodfetch_egress`, as nftables 1.0 writes it.
fn counters(canary: u64, denied: u64) -> String {
    format!(
        r#"{{"nftables": [{{"metainfo": {{"version": "1.0.9", "release_name": "Old Doc Yak #3", "json_schema_version": 1}}}}, {{"counter": {{"family": "inet", "name": "canary", "table": "goodfetch_egress", "handle": 1, "packets": {canary}, "bytes": 60}}}}, {{"counter": {{"family": "inet", "name": "denied", "table": "goodfetch_egress", "handle": 2, "packets": {denied}, "bytes": 60}}}}, {{"counter": {{"family": "inet", "name": "background", "table": "goodfetch_egress", "handle": 3, "packets": 4, "bytes": 240}}}}]}}"#
    )
}

fn verdict(workspace: &Workspace) -> crate::workspace::Outcome {
    workspace.xtask(&[
        "egress",
        "canary",
        "verdict",
        "--outcome",
        "outcome.txt",
        "--before",
        "before.json",
        "--after",
        "after.json",
    ])
}

#[rstest]
#[case::refused_by_the_deny("refused")]
#[case::timed_out_after_the_deny_counted_it("timed-out")]
fn canary_the_deny_counted_and_that_did_not_connect_passes(#[case] outcome: &str) {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("outcome.txt", outcome.as_bytes());
    workspace.write("before.json", counters(0, 0).as_bytes());
    workspace.write("after.json", counters(1, 1).as_bytes());

    let outcome = verdict(&workspace);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn canary_that_connected_fails() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("outcome.txt", b"connected");
    workspace.write("before.json", counters(0, 0).as_bytes());
    workspace.write("after.json", counters(0, 0).as_bytes());

    let outcome = verdict(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("canary-reached", "1.1.1.1:443")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn canary_refused_before_it_met_the_deny_fails() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("outcome.txt", b"unreachable");
    workspace.write("before.json", counters(1, 3).as_bytes());
    workspace.write("after.json", counters(1, 3).as_bytes());

    let outcome = verdict(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("canary-unseen", "1.1.1.1:443")]
    );
    assert_eq!(outcome.code, 1);
}

fn report(workspace: &Workspace) -> crate::workspace::Outcome {
    workspace.xtask(&[
        "egress",
        "report",
        "--counters",
        "after.json",
        "--log",
        "kernel.log",
    ])
}

const CANARY_LOG: &str = "goodfetch-egress-denied: IN= OUT=eth0 SRC=192.168.5.15 DST=1.1.1.1 LEN=60 TOS=0x00 PREC=0x00 TTL=64 ID=48213 DF PROTO=TCP SPT=41822 DPT=443 WINDOW=64240 RES=0x00 SYN URGP=0\n";

#[test]
fn job_whose_only_refused_connection_was_the_canary_passes() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("after.json", counters(1, 1).as_bytes());
    workspace.write("kernel.log", CANARY_LOG.as_bytes());

    let outcome = report(&workspace);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn each_other_refused_destination_fails_the_job_once() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("after.json", counters(1, 5).as_bytes());
    workspace.write(
        "kernel.log",
        format!(
            "{CANARY_LOG}\
             [  812.441503] goodfetch-egress-denied: IN= OUT=eth0 SRC=192.168.5.15 DST=93.184.215.14 LEN=60 TOS=0x00 PREC=0x00 TTL=64 ID=1 DF PROTO=TCP SPT=40100 DPT=443 WINDOW=64240 RES=0x00 SYN URGP=0\n\
             [  812.442017] goodfetch-egress-denied: IN= OUT=eth0 SRC=192.168.5.15 DST=8.8.8.8 LEN=72 TOS=0x00 PREC=0x00 TTL=64 ID=2 PROTO=UDP SPT=53122 DPT=53 LEN=52\n\
             [  813.442017] goodfetch-egress-denied: IN= OUT=eth0 SRC=192.168.5.15 DST=8.8.8.8 LEN=72 TOS=0x00 PREC=0x00 TTL=64 ID=3 PROTO=UDP SPT=53122 DPT=53 LEN=52\n\
             [  814.000001] goodfetch-egress-denied: IN= OUT=eth0 SRC=fd00::15 DST=2606:4700:4700:0000:0000:0000:0000:1111 LEN=80 TC=0 HOPLIMIT=64 FLOWLBL=0 PROTO=TCP SPT=40200 DPT=443 WINDOW=64800 RES=0x00 SYN URGP=0\n"
        )
        .as_bytes(),
    );

    let outcome = report(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("egress-denied", "tcp 93.184.215.14:443"),
            finding("egress-denied", "udp 8.8.8.8:53"),
            finding("egress-denied", "tcp [2606:4700:4700::1111]:443"),
        ]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn refused_connection_missing_from_the_log_still_fails_the_job() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("after.json", counters(1, 2).as_bytes());
    workspace.write("kernel.log", CANARY_LOG.as_bytes());

    let outcome = report(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("egress-denied-unlogged", "counter denied")]
    );
    assert_eq!(outcome.code, 1);
}
