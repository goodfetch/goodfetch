//! `DnsNamespace` (ADR 0067, Linux): a child in its network and mount
//! namespace resolves names through the Proving ground's DNS fixture, and
//! the host's resolver is left as it was. The child is glibc's `getent`,
//! which resolves through `/etc/nsswitch.conf` and `/etc/resolv.conf` as
//! any program does, and exits 2 when a name does not resolve.
//!
//! Scheduled on Linux only (TESTING.md, Trust boundaries: rebinding end to
//! end on Linux only): the module is compiled for Linux alone, so another
//! OS never runs it, and on Linux nothing skips it.
#![expect(
    clippy::disallowed_types,
    reason = "a test takes an address nothing listens on by binding a UDP socket and letting it go"
)]

use std::net::SocketAddr;
use std::process::Output;

use goodfetch_test_support::{DnsNamespace, ProvingGround, TestEnv, command};

/// A name the DNS fixture answers with a public-looking address, then a
/// loopback one, then the first again: one rebinding.
const REBINDING: &str = "93-184-215-14.127-0-0-1.alternating.dns.test";

/// `getent`'s exit status when a name does not resolve.
const NOT_FOUND: i32 = 2;

/// A shell script that resolves its first argument's IPv4 addresses twice,
/// printing a line `--` between the two answers.
const RESOLVE_TWICE: &str = r#"getent ahostsv4 "$1"; echo --; getent ahostsv4 "$1""#;

fn dns(proving_ground: &ProvingGround) -> SocketAddr {
    proving_ground.description().fixtures.dns.parse().unwrap()
}

/// The first address of each answer `RESOLVE_TWICE` printed.
fn first_addresses(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .split("--\n")
        .map(|answer| {
            answer
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[test]
fn a_child_in_the_namespace_resolves_the_fixtures_alternating_answers_in_turn() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, env.seed()).unwrap();
    let namespace = DnsNamespace::enter(&env, dns(&proving_ground)).unwrap();

    let output = namespace
        .output(&env, "/bin/sh", ["-c", RESOLVE_TWICE, "sh", REBINDING])
        .unwrap();

    assert!(
        output.status.success(),
        "{:?}: {output:?}",
        namespace.mechanism()
    );
    assert_eq!(first_addresses(&output), ["93.184.215.14", "127.0.0.1"]);
}

#[test]
fn a_child_in_the_namespace_leaves_the_hosts_resolver_as_it_was() {
    let env = TestEnv::new();
    let host_files = || {
        ["/etc/resolv.conf", "/etc/nsswitch.conf"].map(|path| std::fs::read_to_string(path).ok())
    };
    let before = host_files();
    let proving_ground = ProvingGround::start(&env, env.seed()).unwrap();
    let namespace = DnsNamespace::enter(&env, dns(&proving_ground)).unwrap();

    let inside = namespace
        .output(&env, "/usr/bin/getent", ["ahostsv4", REBINDING])
        .unwrap();
    let outside = command(&env, "/usr/bin/getent")
        .args(["ahostsv4", REBINDING])
        .output()
        .unwrap();

    assert!(inside.status.success(), "{inside:?}");
    assert_eq!(host_files(), before);
    assert_eq!(outside.status.code(), Some(NOT_FOUND), "{outside:?}");
}

#[test]
fn a_child_in_a_namespace_with_no_fixture_at_its_address_resolves_nothing() {
    let env = TestEnv::new();
    // An address nothing listens on: a port bound, then let go.
    let nothing = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let namespace = DnsNamespace::enter(&env, nothing).unwrap();

    let output = namespace
        .output(&env, "/usr/bin/getent", ["ahostsv4", REBINDING])
        .unwrap();

    assert_eq!(output.status.code(), Some(NOT_FOUND), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}
