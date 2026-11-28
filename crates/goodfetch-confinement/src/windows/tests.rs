use std::path::PathBuf;

use goodfetch_protocol::plugin::{NetworkLevel, PluginResourceLimit};
use rstest::rstest;

use super::{
    ConfineError, FilterBrokerGrant, JOB_MSG_ACTIVE_PROCESS_LIMIT, JOB_MSG_ACTIVE_PROCESS_ZERO,
    JOB_MSG_END_OF_JOB_TIME, JOB_MSG_END_OF_PROCESS_TIME, JOB_MSG_JOB_MEMORY_LIMIT,
    JOB_MSG_PROCESS_MEMORY_LIMIT, WindowsProbeTable, check_network_level, check_probes,
    classify_job_notification, committed_probes, cpu_time_to_100ns, rules,
};
use crate::deny_list::{self, Host};
use crate::probes::{Errno, Mismatch, Outcome};

/// Every Windows confinement rule, signal/process-isolation rule, Windows
/// deny-list entry and policy file rule at `none` has a probe in
/// `probes/windows/none.toml`, and no probe lacks a rule (ADR 0067, issue 873).
/// Covers: story:7-105
#[test]
fn every_windows_rule_and_deny_entry_at_none_has_its_probe_and_no_probe_lacks_a_rule() {
    let table = WindowsProbeTable::read(committed_probes(NetworkLevel::None).unwrap()).unwrap();
    let denied: Vec<_> = deny_list::on(Host::Windows).collect();
    let level_rules = rules(NetworkLevel::None).unwrap();
    let mismatches = check_probes(&table, &level_rules, &denied);

    assert_eq!(
        mismatches
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        Vec::<String>::new(),
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

/// `proxy_tcp` and `direct` on Windows wait for the filter broker (#879) and
/// are refused with `ConfineError::Unavailable` without a broker grant, while
/// `none` requires no broker (ADR 0033, issue 873).
/// Covers: story:7-105
#[test]
fn windows_proxy_tcp_and_direct_wait_for_filter_broker() {
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
        };
        assert!(check_network_level(level, Some(&grant)).is_ok());
    }
    assert!(rules(NetworkLevel::ProxyTcp).is_none());
    assert!(rules(NetworkLevel::Direct).is_none());
    assert!(committed_probes(NetworkLevel::ProxyTcp).is_none());
    assert!(committed_probes(NetworkLevel::Direct).is_none());
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
