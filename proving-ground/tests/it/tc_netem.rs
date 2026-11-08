//! `tc netem` on Linux, for published figures (ADR 0025): with `--shaper
//! netem`, `serve` sets the declared profile's `netem` parameters on the
//! loopback interface for its own port, and removes them when it stops. The
//! test reads the parameters back from the kernel, never the elapsed time
//! (ADR 0064). It needs `CAP_NET_ADMIN` in its network namespace and
//! iproute2's `tc`: the `netem` OS service, which `xtask os-services check
//! --netem` confirms, and a local `just test` skips this module without.
//! Covers: story:2-44

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};

use goodfetch_test_support::{LOOPBACK_PORT_0, serve_arguments};
use serde_json::Value;

/// Where `tc` lives, for the Proving ground's cleared environment.
const PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// What `tc` shows of the loopback interface's queueing disciplines.
fn loopback_queueing() -> String {
    let output = Command::new("tc")
        .args(["qdisc", "show", "dev", "lo"])
        .env("PATH", PATH)
        .output()
        .expect("`tc` runs");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("`tc` prints UTF-8")
}

#[test]
fn serve_sets_the_declared_netem_parameters_until_it_stops() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
        .args(serve_arguments(160, LOOPBACK_PORT_0, "rtt-20"))
        .args(["--loss", "1", "--shaper", "netem"])
        .env_clear()
        .env("PATH", PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the Proving ground starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout is piped"))
        .read_line(&mut line)
        .expect("stdout is readable");
    let network =
        serde_json::from_str::<Value>(&line).expect("the line is JSON")["network"].clone();
    let shaped = loopback_queueing();
    let stopped = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("`kill` runs");
    let exited = child.wait().expect("the Proving ground exits");

    assert_eq!(network["shaper"], "netem", "{line}");
    let netem = shaped
        .lines()
        .find(|qdisc| qdisc.starts_with("qdisc netem 40: parent 1:4 "))
        .unwrap_or_else(|| panic!("no netem on lo's fourth band: {shaped}"));
    assert!(
        netem.contains(" delay 10ms") && netem.contains(" loss 1%"),
        "{netem}"
    );
    assert!(stopped.success() && exited.success(), "{exited}");
    assert!(!loopback_queueing().contains("netem"));
}
