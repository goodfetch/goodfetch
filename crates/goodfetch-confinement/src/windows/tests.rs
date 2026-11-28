use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use goodfetch_protocol::plugin::{NetworkLevel, PluginResourceLimit};
use rstest::rstest;

use super::broker::{
    AllowedDestination, BrokerProtocolError, FilterAction, compile_rules, format_filter_tag,
    is_link_local_or_metadata, needs_loopback_exemption, parse_filter_tag, pipe_sddl_for_user_sid,
};
use super::{
    ConfineError, FilterBrokerGrant, JOB_MSG_ACTIVE_PROCESS_LIMIT, JOB_MSG_ACTIVE_PROCESS_ZERO,
    JOB_MSG_END_OF_JOB_TIME, JOB_MSG_END_OF_PROCESS_TIME, JOB_MSG_JOB_MEMORY_LIMIT,
    JOB_MSG_PROCESS_MEMORY_LIMIT, WindowsProbeTable, check_network_level, check_probes,
    classify_job_notification, committed_probes, cpu_time_to_100ns, rules,
};
use crate::deny_list::{self, Host};
use crate::probes::{Errno, Mismatch, Outcome};

/// Every Windows confinement rule, signal/process-isolation rule, Windows
/// deny-list entry and policy file rule at `none`, `proxy_tcp` and `direct`
/// has a probe in `probes/windows/<level>.toml`, and no probe lacks a rule
/// (ADR 0067, issues 873 and 879).
/// Covers: story:7-105, story:7-24
#[rstest]
#[case::none(NetworkLevel::None)]
#[case::proxy_tcp(NetworkLevel::ProxyTcp)]
#[case::direct(NetworkLevel::Direct)]
fn every_windows_rule_and_deny_entry_has_its_probe_and_no_probe_lacks_a_rule(
    #[case] level: NetworkLevel,
) {
    let table = WindowsProbeTable::read(committed_probes(level).unwrap()).unwrap();
    let denied: Vec<_> = deny_list::on(Host::Windows).collect();
    let level_rules = rules(level).unwrap();
    let mismatches = check_probes(&table, &level_rules, &denied);

    assert_eq!(
        mismatches
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        Vec::<String>::new(),
        "mismatches for {level:?}"
    );
}

/// Covers: story:7-105
#[test]
fn windows_probe_check_catches_missing_rule_missing_deny_entry_and_missing_positive_control() {
    let denied: Vec<_> = deny_list::on(Host::Windows).collect();
    let level_rules = rules(NetworkLevel::None).unwrap();

    let mut missing_op =
        WindowsProbeTable::read(committed_probes(NetworkLevel::None).unwrap()).unwrap();
    missing_op.operation.retain(|c| c.rule != "desktop.default");
    assert_eq!(
        check_probes(&missing_op, &level_rules, &denied),
        vec![Mismatch::RuleWithoutProbe("desktop.default".to_owned())]
    );

    let mut missing_deny =
        WindowsProbeTable::read(committed_probes(NetworkLevel::None).unwrap()).unwrap();
    missing_deny
        .path
        .retain(|c| c.rule != "deny.credentials_roaming");
    assert_eq!(
        check_probes(&missing_deny, &level_rules, &denied),
        vec![Mismatch::RuleWithoutProbe(
            "deny.credentials_roaming".to_owned()
        )]
    );

    let mut bad_control =
        WindowsProbeTable::read(committed_probes(NetworkLevel::None).unwrap()).unwrap();
    bad_control
        .operation
        .iter_mut()
        .find(|c| c.rule == "network.outbound_ip")
        .unwrap()
        .unconfined = Outcome::Errno(Errno::Eacces);
    assert_eq!(
        check_probes(&bad_control, &level_rules, &denied),
        vec![Mismatch::NoPositiveControl {
            rule: "network.outbound_ip".to_owned()
        }]
    );
}

/// `proxy_tcp` and `direct` on Windows require a [`FilterBrokerGrant`] and are
/// refused with `ConfineError::Unavailable` without one, while `none` requires
/// no broker; both levels expose declarative rules and committed probe tables
/// (ADR 0033, issues 873 and 879).
/// Covers: story:7-105, story:7-24
#[test]
fn windows_proxy_tcp_and_direct_require_filter_broker_grant() {
    assert!(check_network_level(NetworkLevel::None, None).is_ok());
    for level in [NetworkLevel::ProxyTcp, NetworkLevel::Direct] {
        let err = check_network_level(level, None).unwrap_err();
        assert!(
            matches!(err, ConfineError::Unavailable(ref msg) if msg.contains("filter broker")),
            "{level:?}: {err}"
        );
        let grant = FilterBrokerGrant {
            app_container_name: "goodfetch.plugin.broker.1".to_owned(),
            proxy_port: (level == NetworkLevel::ProxyTcp).then_some(49152),
            allowed_destinations: Vec::new(),
            pipe_name: None,
            attestation: None,
        };
        assert!(check_network_level(level, Some(&grant)).is_ok());
        assert!(rules(level).is_some());
        assert!(committed_probes(level).is_some());
    }
}

/// Compiled WFP filter rules always place refusing (`Block`) filters strictly
/// before permitting (`Permit`) filters, refuse link-local/cloud-metadata
/// destinations even when configured, and produce a protected SDDL granting
/// only SYSTEM, Administrators and the installing user (ADRs 0033 and 0067,
/// issue 879).
/// Covers: story:7-24, story:7-112
#[test]
fn wfp_rule_compiler_orders_blocks_first_and_forbids_metadata() {
    let proxy_rules = compile_rules(
        "goodfetch.plugin.1",
        "inst-1",
        NetworkLevel::ProxyTcp,
        Some(43123),
        &[],
    )
    .unwrap();
    let first_permit = proxy_rules
        .iter()
        .position(|rule| rule.action == FilterAction::Permit)
        .unwrap();
    assert!(
        proxy_rules[..first_permit]
            .iter()
            .all(|rule| rule.action == FilterAction::Block),
        "all refusing filters must precede permitting filters"
    );
    assert!(
        proxy_rules[first_permit..]
            .iter()
            .all(|rule| rule.action == FilterAction::Permit)
    );
    assert!(needs_loopback_exemption(NetworkLevel::ProxyTcp, &[]));

    let configured = vec![AllowedDestination {
        ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: Some(5432),
    }];
    let direct_rules = compile_rules(
        "goodfetch.plugin.2",
        "inst-2",
        NetworkLevel::Direct,
        None,
        &configured,
    )
    .unwrap();
    let first_direct_permit = direct_rules
        .iter()
        .position(|rule| rule.action == FilterAction::Permit)
        .unwrap();
    assert!(
        direct_rules[..first_direct_permit]
            .iter()
            .all(|rule| rule.action == FilterAction::Block)
    );
    assert!(needs_loopback_exemption(NetworkLevel::Direct, &configured));
    assert!(!needs_loopback_exemption(NetworkLevel::Direct, &[]));

    // Link-local and cloud-metadata addresses are always refused.
    for forbidden in [
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
        IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)),
        IpAddr::V6(Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254)),
    ] {
        assert!(is_link_local_or_metadata(forbidden));
        let err = compile_rules(
            "goodfetch.plugin.3",
            "inst-3",
            NetworkLevel::Direct,
            None,
            &[AllowedDestination {
                ip: forbidden,
                port: Some(80),
            }],
        )
        .unwrap_err();
        assert_eq!(
            err,
            BrokerProtocolError::LinkLocalOrMetadataForbidden(forbidden)
        );
    }

    // Filter metadata tag round-trips cleanly.
    let tag = format_filter_tag("goodfetch.plugin.1", "inst-1", "block.outbound_v4");
    assert_eq!(
        parse_filter_tag(&tag),
        Some(("goodfetch.plugin.1", "inst-1", "block.outbound_v4"))
    );
    assert_eq!(parse_filter_tag("other-provider:foo"), None);

    // Named-pipe SDDL restricts access to SYSTEM, Administrators and installing user SID.
    let sddl = pipe_sddl_for_user_sid("S-1-5-21-100-200-300-1001").unwrap();
    assert_eq!(
        sddl,
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;S-1-5-21-100-200-300-1001)"
    );
    assert!(pipe_sddl_for_user_sid("not-a-sid;WD").is_err());
}

/// Every Windows Job Object limit notification maps to its
/// `PluginResourceLimit`, and non-limit lifecycle messages map to `None`
/// (ADR 0033, issue 873).
/// Covers: story:7-105
#[rstest]
#[case::end_of_job_time(JOB_MSG_END_OF_JOB_TIME, Some(PluginResourceLimit::CpuTime))]
#[case::end_of_process_time(JOB_MSG_END_OF_PROCESS_TIME, Some(PluginResourceLimit::CpuTime))]
#[case::active_process_limit(JOB_MSG_ACTIVE_PROCESS_LIMIT, Some(PluginResourceLimit::Processes))]
#[case::process_memory_limit(JOB_MSG_PROCESS_MEMORY_LIMIT, Some(PluginResourceLimit::Memory))]
#[case::job_memory_limit(JOB_MSG_JOB_MEMORY_LIMIT, Some(PluginResourceLimit::Memory))]
#[case::active_process_zero(JOB_MSG_ACTIVE_PROCESS_ZERO, None)]
fn job_object_notifications_classify_to_plugin_resource_limits(
    #[case] msg_id: u32,
    #[case] expected: Option<PluginResourceLimit>,
) {
    assert_eq!(classify_job_notification(msg_id), expected);
    assert_eq!(cpu_time_to_100ns(60), 600_000_000);
    assert_eq!(cpu_time_to_100ns(u64::MAX), i64::MAX);
}

/// Only `windows/win32.rs` opts into `unsafe_code` under `src/windows/`
/// (ADR 0033, STYLE.md, issue 873).
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "unit test inspects its own crate source tree synchronously"
)]
fn win32_is_the_only_windows_module_that_opts_into_unsafe() {
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
    check_file(root.join("windows.rs"));
    for entry in std::fs::read_dir(root.join("windows")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs")
            && path.file_name().and_then(|n| n.to_str()) != Some("tests.rs")
        {
            check_file(path);
        }
    }
    assert_eq!(unsafe_files, vec!["windows/win32.rs".to_owned()]);
}
