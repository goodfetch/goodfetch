//! What a job under ADR 0043's deny may reach, and the rules its VM loads:
//! the fixtures, local to the job, and a named-endpoint job's own list,
//! nothing else. The pinned artifact mirror (the test V8 build and
//! Goodfetch's own releases) is fetched before the deny and is never allowed
//! under it, by the owner's decision of 2026-10-19.
//! Covers: story:1-68

use rstest::rstest;

use crate::workspace::Workspace;

/// A job ADR 0043 names, reaching its own endpoints under the deny.
const NAMED_SINKS: &str = "[[named]]\n\
     job = \"nightly.yml#sinks\"\n\
     exception = \"saas-sinks\"\n\
     endpoints = [\"db.example.com:5432\", \"objects.example.com:443\"]\n";

#[test]
fn job_under_the_deny_reaches_no_endpoint() {
    let workspace = Workspace::from_fixture("egress");

    let outcome = workspace.xtask(&["egress", "endpoints", "--job", "pull-request.yml#quick"]);

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.code, 0);
}

#[test]
fn named_endpoint_job_reaches_its_own_list_only() {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file("egress-cases/releases.toml", "compat/releases.toml");
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
         db.example.com 5432\n"
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
#[case::pinned_mirror_in_the_allowed_set(
    "[[mirror]]\nkind = \"test-v8\"\nurls = [\"https://github.com/goodfetch/goodfetch/releases/download/\"]\n"
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
fn rules_accept_the_resolved_named_endpoints_and_refuse_and_count_the_rest() {
    let workspace = Workspace::from_fixture("egress");
    workspace.append("ci/egress/endpoints.toml", NAMED_SINKS);
    workspace.add_fixture_file("egress-cases/resolved.txt", "resolved.txt");

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "nightly.yml#sinks",
        "--resolved",
        "resolved.txt",
        "--uid",
        "501",
    ]);

    assert_eq!(
        outcome.stdout,
        r#"# ADR 0043's outbound-traffic deny for nightly.yml#sinks (xtask egress rules).
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
		elements = { 203.0.113.10 . 5432, 203.0.113.20 . 443, 203.0.113.21 . 443 }
	}
	set allowed_ipv6 {
		type ipv6_addr . inet_service
		elements = { 2001:db8::20 . 443 }
	}
	chain output {
		type filter hook output priority filter; policy accept;
		oifname "lo" accept
		ct state established,related accept
		udp sport 68 udp dport 67 accept
		icmpv6 type { nd-router-solicit, nd-neighbor-solicit, nd-neighbor-advert, mld-listener-report, mld-listener-done, mld2-listener-report } accept
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

/// A release file in the tree opens none of the release hosts: the rules
/// once allowed what `compat/releases.toml` named.
#[rstest]
#[case::test_v8_build_on_github("pull-request.yml#quick", "github.com 443 140.82.121.4\n")]
#[case::release_asset_redirect(
    "pull-request.yml#quick",
    "release-assets.githubusercontent.com 443 185.199.108.133\n"
)]
#[case::released_image("pull-request.yml#quick", "ghcr.io 443 140.82.114.33\n")]
#[case::mirror_host_for_a_named_job("nightly.yml#sinks", "github.com 443 140.82.121.4\n")]
fn mirror_host_is_refused_under_the_deny(#[case] job: &str, #[case] line: &str) {
    let workspace = Workspace::from_fixture("egress");
    workspace.add_fixture_file("egress-cases/releases.toml", "compat/releases.toml");
    workspace.append("ci/egress/endpoints.toml", NAMED_SINKS);
    workspace.write("resolved.txt", line.as_bytes());

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        job,
        "--resolved",
        "resolved.txt",
        "--uid",
        "501",
    ]);

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.code, 2);
}

#[rstest]
#[case::host_the_job_may_not_reach("stats.example.com 443 203.0.113.9\n")]
#[case::named_host_on_another_port("db.example.com 22 203.0.113.10\n")]
#[case::named_host_resolved_to_the_canary("objects.example.com 443 1.1.1.1\n")]
#[case::address_that_is_not_one("db.example.com 5432 db.example.com\n")]
fn resolved_address_outside_the_allowed_list_is_refused(#[case] line: &str) {
    let workspace = Workspace::from_fixture("egress");
    workspace.append("ci/egress/endpoints.toml", NAMED_SINKS);
    workspace.write("resolved.txt", line.as_bytes());

    let outcome = workspace.xtask(&[
        "egress",
        "rules",
        "--job",
        "nightly.yml#sinks",
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
