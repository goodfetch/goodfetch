//! What a job under ADR 0043's deny may reach, and the rules its VM loads:
//! the pinned artifact mirror (Goodfetch's own releases and the test V8
//! build included) and a named-endpoint job's own list, nothing else.
//! Covers: story:1-68

use rstest::rstest;

use crate::workspace::Workspace;

#[test]
fn job_under_the_deny_reaches_the_pinned_artifact_mirror_only() {
    let workspace = Workspace::from_fixture("egress");

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", "pull-request.yml#quick"]);

    assert_eq!(
        outcome.stdout,
        "github.com 443\n\
         release-assets.githubusercontent.com 443\n\
         storage.googleapis.com 443\n"
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn mirror_accepts_the_releases_compat_lists() {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file("egress-cases/releases.toml", "compat/releases.toml");

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", "pull-request.yml#quick"]);

    assert_eq!(
        outcome.stdout,
        "artifacts.example.org 8443\n\
         ghcr.io 443\n\
         github.com 443\n\
         release-assets.githubusercontent.com 443\n\
         storage.googleapis.com 443\n"
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn named_endpoint_job_also_reaches_its_own_list() {
    let workspace = Workspace::from_fixture("egress");
    workspace.append(
        "ci/egress/endpoints.toml",
        "[[named]]\n\
         job = \"nightly.yml#sinks\"\n\
         exception = \"saas-sinks\"\n\
         endpoints = [\"db.example.com:5432\", \"[2001:db8::1]:443\"]\n",
    );

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", "nightly.yml#sinks"]);

    assert_eq!(
        outcome.stdout,
        "2001:db8::1 443\n\
         db.example.com 5432\n\
         github.com 443\n\
         release-assets.githubusercontent.com 443\n\
         storage.googleapis.com 443\n"
    );
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::outside_the_deny("pull-request.yml#all-green")]
#[case::not_declared("pull-request.yml#lint")]
fn job_without_a_deny_has_no_allowed_list(#[case] job: &str) {
    let workspace = Workspace::from_fixture("egress");

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", job]);

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.code, 2);
}

#[rstest]
#[case::download_that_is_not_pinned(
    "[[mirror]]\nkind = \"analytics\"\nurls = [\"https://stats.example.com/\"]\n"
)]
#[case::download_over_plain_http(
    "[[mirror]]\nkind = \"browser\"\nurls = [\"http://example.com/chrome/\"]\n"
)]
#[case::endpoints_outside_adr_0043s_jobs(
    "[[named]]\njob = \"nightly.yml#extra\"\nexception = \"telemetry\"\nendpoints = [\"stats.example.com:443\"]\n"
)]
fn endpoints_file_outside_adr_0043_is_refused(#[case] entry: &str) {
    let workspace = Workspace::from_fixture("egress");
    workspace.append("ci/egress/endpoints.toml", entry);

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", "pull-request.yml#quick"]);

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.code, 2);
}

#[test]
fn rules_accept_the_resolved_mirror_and_refuse_and_count_the_rest() {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file("egress-cases/resolved.txt", "resolved.txt");

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "pull-request.yml#quick",
        "--resolved",
        "resolved.txt",
        "--uid",
        "501",
    ]);

    assert_eq!(
        outcome.stdout,
        r#"# ADR 0043's outbound-traffic deny for pull-request.yml#quick (xtask egress rules).
table inet goodfetch_egress
delete table inet goodfetch_egress
table inet goodfetch_egress {
	counter canary {
	}
	counter denied {
	}
	counter background {
	}
	set allowed_ipv4 {
		type ipv4_addr . inet_service
		elements = { 140.82.121.4 . 443, 142.250.180.155 . 443, 185.199.108.133 . 443, 185.199.109.133 . 443 }
	}
	set allowed_ipv6 {
		type ipv6_addr . inet_service
		elements = { 2606:50c0:8000::154 . 443 }
	}
	chain output {
		type filter hook output priority filter; policy accept;
		oifname "lo" accept
		ct state established,related accept
		udp sport 68 udp dport 67 accept
		icmpv6 type { nd-router-solicit, nd-neighbor-solicit, nd-neighbor-advert } accept
		ip daddr . tcp dport @allowed_ipv4 accept
		ip6 daddr . tcp dport @allowed_ipv6 accept
		meta skuid != 501 counter name "background" reject
		ip daddr 1.1.1.1 tcp dport 443 counter name "canary"
		counter name "denied" log prefix "goodfetch-egress-denied: "
		meta l4proto tcp reject with tcp reset
		reject with icmpx admin-prohibited
	}
}
"#
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn rules_with_nothing_resolved_accept_no_address() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("resolved.txt", b"");

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "pull-request.yml#quick",
        "--resolved",
        "resolved.txt",
        "--uid",
        "501",
    ]);

    insta::assert_snapshot!(outcome.stdout);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::host_the_job_may_not_reach("stats.example.com 443 203.0.113.9\n")]
#[case::mirror_host_on_another_port("github.com 22 140.82.121.4\n")]
#[case::mirror_host_resolved_to_the_canary("github.com 443 1.1.1.1\n")]
#[case::address_that_is_not_one("github.com 443 github.com\n")]
fn resolved_address_outside_the_allowed_list_is_refused(#[case] line: &str) {
    let workspace = Workspace::from_fixture("egress");
    workspace.write("resolved.txt", line.as_bytes());

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "pull-request.yml#quick",
        "--resolved",
        "resolved.txt",
        "--uid",
        "501",
    ]);

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.code, 2);
}

#[test]
fn canary_on_the_local_network_is_refused() {
    let workspace = Workspace::from_fixture("egress");
    workspace.write(
        "ci/egress/endpoints.toml",
        b"canary = \"192.168.1.1:443\"\n\n[[deny]]\njob = \"pull-request.yml#quick\"\n",
    );

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", "pull-request.yml#quick"]);

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.code, 2);
}
