use std::collections::BTreeMap;
use std::path::PathBuf;

use goodfetch_protocol::plugin::{Limits, NetworkLevel};
use rstest::rstest;

use super::{
    ConfineError, DNS_MACH_SERVICES, MacOsProbeTable, check_probes, committed_probes, profile,
    rules,
};
use crate::deny_list::{self, Host};
use crate::policy::Policy;
use crate::probes::{Errno, Mismatch, Outcome};

fn sample_policy(network_level: NetworkLevel) -> Policy {
    Policy {
        network_level,
        reads: vec![
            PathBuf::from("/Users/alice/.goodfetch/plugins/demo/1.0.0"),
            PathBuf::from("/Users/alice/.goodfetch/runtimes/node-22"),
        ],
        writes: vec![PathBuf::from("/var/folders/tmp/instance-1")],
        executables: vec![PathBuf::from(
            "/Users/alice/.goodfetch/plugins/demo/1.0.0/index.js",
        )],
        env: BTreeMap::new(),
        limits: Limits::default(),
    }
}

#[test]
fn none_profile_denies_all_network_and_dns_mach_services() {
    let sbpl = profile(&sample_policy(NetworkLevel::None), None).unwrap();

    assert!(sbpl.starts_with("(version 1)\n(deny default)\n"));
    assert!(sbpl.contains("(deny network*)\n"));
    assert!(!sbpl.contains("(allow network-outbound"));
    assert!(sbpl.contains("(deny mach-lookup"));
    for service in DNS_MACH_SERVICES {
        assert!(sbpl.contains(service), "missing DNS service {service}");
    }
    assert!(!sbpl.contains("(allow process-fork)"));
    assert!(sbpl.contains("(allow signal (target self))"));
    assert!(sbpl.contains(
        "(allow file-read* file-write* (subpath \"/private/var/folders/tmp/instance-1\"))"
    ));
}

#[test]
fn proxy_tcp_profile_requires_proxy_port_and_allows_only_that_loopback_port() {
    let policy = sample_policy(NetworkLevel::ProxyTcp);
    let err = profile(&policy, None).unwrap_err();
    assert!(matches!(err, ConfineError::MissingProxyPort), "{err}");

    let sbpl = profile(&policy, Some(49152)).unwrap();
    assert!(sbpl.contains("(deny network*)\n"));
    assert!(sbpl.contains("(allow network-outbound (remote tcp \"localhost:49152\"))"));
    assert!(!sbpl.contains("(allow network-outbound (remote ip))"));
    assert!(sbpl.contains("(deny mach-lookup"));
}

#[test]
fn direct_profile_allows_outbound_ip_and_dns_mach_services_but_denies_inbound() {
    let sbpl = profile(&sample_policy(NetworkLevel::Direct), None).unwrap();

    assert!(sbpl.contains("(deny network*)\n"));
    assert!(sbpl.contains("(allow network-outbound (remote ip))"));
    assert!(sbpl.contains("(allow mach-lookup"));
    for service in DNS_MACH_SERVICES {
        assert!(sbpl.contains(service), "missing DNS service {service}");
    }
    assert!(!sbpl.contains("(allow network-inbound"));
    assert!(!sbpl.contains("(allow network-bind"));
}

#[test]
fn profile_escapes_quotes_and_backslashes_and_rejects_nul_bytes() {
    let mut policy = sample_policy(NetworkLevel::None);
    policy.reads.push(PathBuf::from("/tmp/dir\"with\\chars"));
    let sbpl = profile(&policy, None).unwrap();
    assert!(sbpl.contains(r#"(subpath "/tmp/dir\"with\\chars")"#));

    policy.reads.push(PathBuf::from("/tmp/bad\0path"));
    let err = profile(&policy, None).unwrap_err();
    assert!(matches!(err, ConfineError::InvalidPath(_)), "{err}");
}

/// Every Seatbelt rule, signal rule, macOS deny-list entry and policy file
/// rule at each network level has a probe in `probes/macos/*.toml`, and no
/// probe lacks a rule (ADR 0067, issue 867).
#[rstest]
#[case::none(NetworkLevel::None)]
#[case::proxy_tcp(NetworkLevel::ProxyTcp)]
#[case::direct(NetworkLevel::Direct)]
fn every_macos_rule_and_deny_entry_has_its_probe_and_no_probe_lacks_a_rule(
    #[case] level: NetworkLevel,
) {
    let table = MacOsProbeTable::read(committed_probes(level)).unwrap();
    let denied: Vec<_> = deny_list::on(Host::MacOs).collect();
    let mismatches = check_probes(&table, &rules(level), &denied);

    assert_eq!(
        mismatches
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        Vec::<String>::new(),
    );
}

#[test]
fn macos_probe_check_catches_missing_rule_missing_deny_entry_and_missing_positive_control() {
    let denied: Vec<_> = deny_list::on(Host::MacOs).collect();
    let level_rules = rules(NetworkLevel::None);

    let mut missing_op = MacOsProbeTable::read(committed_probes(NetworkLevel::None)).unwrap();
    missing_op.operation.retain(|c| c.rule != "mach.dns");
    assert_eq!(
        check_probes(&missing_op, &level_rules, &denied),
        vec![Mismatch::RuleWithoutProbe("mach.dns".to_owned())]
    );

    let mut missing_deny = MacOsProbeTable::read(committed_probes(NetworkLevel::None)).unwrap();
    missing_deny.path.retain(|c| c.rule != "deny.keychains");
    assert_eq!(
        check_probes(&missing_deny, &level_rules, &denied),
        vec![Mismatch::RuleWithoutProbe("deny.keychains".to_owned())]
    );

    let mut bad_control = MacOsProbeTable::read(committed_probes(NetworkLevel::None)).unwrap();
    bad_control
        .operation
        .iter_mut()
        .find(|c| c.rule == "mach.dns")
        .unwrap()
        .unconfined = Outcome::Errno(Errno::Eperm);
    assert_eq!(
        check_probes(&bad_control, &level_rules, &denied),
        vec![Mismatch::NoPositiveControl {
            rule: "mach.dns".to_owned()
        }]
    );
}

/// Only `macos/seatbelt.rs` opts into `unsafe_code` under `src/macos/`
/// (ADR 0033, STYLE.md, issue 867).
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "unit test inspects its own crate source tree synchronously"
)]
fn seatbelt_is_the_only_macos_module_that_opts_into_unsafe() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut unsafe_files = Vec::new();
    let mut check_file = |path: PathBuf| {
        let text = std::fs::read_to_string(&path).unwrap();
        if text.contains("unsafe_code") {
            unsafe_files.push(
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    };
    check_file(root.join("macos.rs"));
    for entry in std::fs::read_dir(root.join("macos")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs")
            && path.file_name().and_then(|n| n.to_str()) != Some("tests.rs")
        {
            check_file(path);
        }
    }
    assert_eq!(unsafe_files, vec!["macos/seatbelt.rs".to_owned()]);
}
