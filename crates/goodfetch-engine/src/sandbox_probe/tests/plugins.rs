//! The startup probe's answer for a process Plugin (ADR 0049): the reading
//! that explains the browser's sandbox refusal refuses a Plugin's start with
//! `confinement_unavailable`, naming the same environment and a fix.

use rstest::rstest;
use serde_json::json;

use super::{EPERM, facts, host, seccomp_refusing, wsl1};
use crate::sandbox_probe::{Facts, HostFacts, NoSandboxPlatform, UserNamespace, judge};

/// One named case per refusal the probe knows, and one it does not: each
/// is `confinement_unavailable` naming the environment that refuses the
/// browser's sandbox too, and a fix that names the remedy.
/// Covers: story:7-27, error:confinement_unavailable
#[rstest]
#[case::wsl1(facts(wsl1()), "wsl1", "wsl --set-version <distribution> 2")]
#[case::dockers_default_seccomp_before_6_12(
    facts(seccomp_refusing()),
    "docker_default_seccomp",
    "docker run --security-opt seccomp=<profile.json>"
)]
#[case::kubernetes_runtime_default_before_6_12(
    Facts { kubernetes: true, ..facts(seccomp_refusing()) },
    "kubernetes_runtime_default",
    "localhostProfile"
)]
#[case::apparmors_restriction_before_6_12(
    facts(HostFacts { apparmor_restricts: true, ..host(UserNamespace::Unmapped(EPERM)) }),
    "apparmor_userns",
    "grants it `userns`"
)]
#[case::dockers_default_seccomp_on_lambda(
    Facts { platform: Some(NoSandboxPlatform::AwsLambda), ..facts(seccomp_refusing()) },
    "docker_default_seccomp",
    "AWS Lambda gives no process the user namespace"
)]
#[case::an_unknown_refusal(
    facts(host(UserNamespace::Refused(EPERM))),
    "unknown",
    "unshare --user --map-root-user true"
)]
fn a_host_that_cannot_confine_refuses_a_plugin_naming_its_environment_and_fix(
    #[case] facts: Facts,
    #[case] environment: &str,
    #[case] remedy: &str,
) {
    let refusal = facts.plugin_refusal().unwrap();

    let data = refusal.details().unwrap();
    let fix = data["fix"].as_str().unwrap();
    assert_eq!(
        (
            refusal.code(),
            refusal.retryable(),
            data["environment"].clone(),
            fix.contains(remedy),
        ),
        ("confinement_unavailable", false, json!(environment), true),
        "{fix}"
    );
}

/// The browser's refusal and the Plugin's name one environment, from one
/// reading of the host.
/// Covers: story:7-27
#[rstest]
#[case::wsl1(facts(wsl1()))]
#[case::dockers_default_seccomp(facts(seccomp_refusing()))]
#[case::kubernetes_runtime_default(Facts { kubernetes: true, ..facts(seccomp_refusing()) })]
#[case::an_unknown_refusal(facts(host(UserNamespace::Refused(EPERM))))]
fn the_browsers_refusal_and_the_plugins_name_the_same_environment(#[case] facts: Facts) {
    let browser = judge(&facts).refusal().unwrap();
    let plugin = facts.plugin_refusal().unwrap();

    assert_eq!(
        browser.details().unwrap()["environment"],
        plugin.details().unwrap()["environment"]
    );
}

/// Landlock ABI 6 (Linux 6.12) confines a Plugin without a user namespace,
/// even under Docker's default seccomp profile, and a namespace made
/// confines one on an older kernel: neither refuses a Plugin.
/// Covers: story:7-27
#[rstest]
#[case::landlock_6_under_dockers_default_seccomp(facts(HostFacts {
    landlock_abi: 6,
    ..seccomp_refusing()
}))]
#[case::a_namespace_made(facts(host(UserNamespace::Created)))]
#[case::apparmors_restriction_with_landlock_6(facts(HostFacts {
    apparmor_restricts: true,
    landlock_abi: 6,
    ..host(UserNamespace::Unmapped(EPERM))
}))]
fn a_host_that_confines_refuses_no_plugin(#[case] facts: Facts) {
    let refusal = facts.plugin_refusal();

    assert!(refusal.is_none(), "{refusal:?}");
}

/// On Ubuntu 23.10+ with AppArmor restricting unprivileged user namespaces,
/// `proxy_tcp` is refused with `confinement_unavailable` whose `fix` names
/// `plugin setup` until `goodfetch-sandbox` is installed (`sandbox_helper =
/// true`), after which `proxy_tcp` is accepted and routed through the helper
/// (ADRs 0033 and 0039, issue 884).
/// Covers: story:7-23, error:confinement_unavailable
#[test]
fn apparmor_restriction_refuses_proxy_tcp_naming_plugin_setup_until_helper_is_installed() {
    let without_helper = facts(HostFacts {
        apparmor_restricts: true,
        landlock_abi: 6,
        ..host(UserNamespace::Unmapped(EPERM))
    });
    let (missing, refusal) = without_helper
        .plugin_confinement(goodfetch_protocol::plugin::NetworkLevel::ProxyTcp)
        .expect_err("without plugin setup, proxy_tcp must be refused under AppArmor restriction");
    let data = refusal.details().unwrap();
    let fix = data["fix"].as_str().unwrap();
    assert_eq!(refusal.code(), "confinement_unavailable");
    assert_eq!(data["environment"], json!("apparmor_userns"));
    assert!(
        fix.contains("plugin setup"),
        "expected fix to name `plugin setup`, got: {fix}"
    );
    assert!(
        missing.contains(&"network_namespace".to_owned()),
        "{missing:?}"
    );

    let with_helper = facts(HostFacts {
        apparmor_restricts: true,
        landlock_abi: 6,
        sandbox_helper: true,
        ..host(UserNamespace::Unmapped(EPERM))
    });
    assert!(
        with_helper
            .plugin_confinement(goodfetch_protocol::plugin::NetworkLevel::ProxyTcp)
            .is_ok(),
        "with goodfetch-sandbox installed, proxy_tcp must be accepted"
    );
    assert!(
        with_helper
            .host
            .needs_sandbox_helper(goodfetch_protocol::plugin::NetworkLevel::ProxyTcp)
    );
}

/// A macOS host where Seatbelt (`sandbox_init`) is refused rejects Plugins
/// with non-retryable `confinement_unavailable` at every network level and
/// never runs them weakened (ADR 0033, issue 867).
/// Covers: story:7-22, error:confinement_unavailable
#[rstest]
#[case::none(goodfetch_protocol::plugin::NetworkLevel::None)]
#[case::proxy_tcp(goodfetch_protocol::plugin::NetworkLevel::ProxyTcp)]
#[case::direct(goodfetch_protocol::plugin::NetworkLevel::Direct)]
fn macos_host_that_refuses_seatbelt_refuses_plugins_with_confinement_unavailable(
    #[case] level: goodfetch_protocol::plugin::NetworkLevel,
) {
    let host = facts(HostFacts {
        seatbelt_refused: true,
        ..HostFacts::NO_NAMESPACE_NEEDED
    });
    let (missing, refusal) = host
        .plugin_confinement(level)
        .expect_err("a macOS host refusing Seatbelt must refuse Plugins");

    assert_eq!(missing, vec!["seatbelt".to_owned()]);
    assert_eq!(refusal.code(), "confinement_unavailable");
    assert!(!refusal.retryable());
    let data = refusal.details().unwrap();
    assert!(
        data["fix"].as_str().unwrap().contains("Seatbelt"),
        "expected fix to mention Seatbelt: {data:?}"
    );
}

/// On Windows, `none` is confined by AppContainer without the filter broker,
/// while `proxy_tcp` and `direct` wait for the filter broker (`plugin setup`)
/// and refuse with `confinement_unavailable` until it is installed; if
/// AppContainer itself is unavailable, every level is refused (ADR 0033,
/// issue 873).
/// Covers: story:7-105, error:confinement_unavailable
#[test]
fn windows_none_confines_without_broker_and_proxy_tcp_direct_wait_for_filter_broker() {
    let windows_without_broker = facts(HostFacts {
        windows_filter_broker: Some(false),
        ..HostFacts::NO_NAMESPACE_NEEDED
    });
    assert!(
        windows_without_broker
            .plugin_confinement(goodfetch_protocol::plugin::NetworkLevel::None)
            .is_ok(),
        "Windows `none` must confine with AppContainer without the filter broker"
    );
    for level in [
        goodfetch_protocol::plugin::NetworkLevel::ProxyTcp,
        goodfetch_protocol::plugin::NetworkLevel::Direct,
    ] {
        let (missing, refusal) = windows_without_broker
            .plugin_confinement(level)
            .expect_err("Windows proxy_tcp/direct must wait for the filter broker");
        assert_eq!(missing, vec!["filter_broker".to_owned()]);
        assert_eq!(refusal.code(), "confinement_unavailable");
        assert!(!refusal.retryable());
        let fix = refusal.details().unwrap()["fix"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            fix.contains("plugins setup"),
            "expected fix to name `plugins setup`, got: {fix}"
        );
    }

    let windows_with_broker = facts(HostFacts {
        windows_filter_broker: Some(true),
        ..HostFacts::NO_NAMESPACE_NEEDED
    });
    for level in [
        goodfetch_protocol::plugin::NetworkLevel::None,
        goodfetch_protocol::plugin::NetworkLevel::ProxyTcp,
        goodfetch_protocol::plugin::NetworkLevel::Direct,
    ] {
        assert!(windows_with_broker.plugin_confinement(level).is_ok());
    }

    let windows_no_appcontainer = facts(HostFacts {
        windows_appcontainer_refused: true,
        windows_filter_broker: Some(false),
        ..HostFacts::NO_NAMESPACE_NEEDED
    });
    let (missing, refusal) = windows_no_appcontainer
        .plugin_confinement(goodfetch_protocol::plugin::NetworkLevel::None)
        .expect_err("Windows host refusing AppContainer must refuse `none`");
    assert_eq!(missing, vec!["appcontainer".to_owned()]);
    assert_eq!(refusal.code(), "confinement_unavailable");
    assert!(!refusal.retryable());
}
