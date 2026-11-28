use goodfetch_protocol::plugin::NetworkLevel;
use rstest::rstest;

use super::{HostFacts, UserNamespace, is_wsl1};

const EPERM: i32 = 1;
const EINVAL: i32 = 22;

fn host(user_namespace: UserNamespace, landlock_abi: u8) -> HostFacts {
    HostFacts {
        user_namespace,
        landlock_abi,
        ..HostFacts::NO_NAMESPACE_NEEDED
    }
}

/// Recorded hosts: a Plugin is confined by Landlock from ABI 6, or by a
/// user namespace of its own below it, and refused on WSL1 and wherever
/// neither is given, as a container under Docker's default seccomp profile
/// on a kernel before 6.12 is.
#[rstest]
#[case::landlock_6_with_namespaces(host(UserNamespace::Created, 6), true)]
#[case::landlock_6_under_dockers_default_seccomp(
    HostFacts { seccomp_filtered: true, ..host(UserNamespace::Refused(EPERM), 6) },
    true
)]
#[case::landlock_4_with_namespaces(host(UserNamespace::Created, 4), true)]
#[case::no_landlock_with_namespaces(host(UserNamespace::Created, 0), true)]
#[case::dockers_default_seccomp_before_6_12(
    HostFacts { seccomp_filtered: true, ..host(UserNamespace::Refused(EPERM), 4) },
    false
)]
#[case::apparmors_restriction_before_6_12(
    HostFacts { apparmor_restricts: true, ..host(UserNamespace::Unmapped(EPERM), 4) },
    false
)]
#[case::wsl1(
    HostFacts { wsl1: true, ..host(UserNamespace::Refused(EINVAL), 0) },
    false
)]
#[case::wsl1_even_with_a_namespace(HostFacts { wsl1: true, ..host(UserNamespace::Created, 0) }, false)]
fn a_plugin_is_confined_by_landlock_6_or_a_user_namespace(
    #[case] facts: HostFacts,
    #[case] confines: bool,
) {
    assert_eq!(facts.confines(), confines);
}

#[rstest]
#[case::wsl1("4.4.0-19041-Microsoft", true)]
#[case::wsl1_with_a_newline("4.4.0-22621-Microsoft\n", true)]
#[case::wsl2("5.15.167.4-microsoft-standard-WSL2", false)]
#[case::ubuntu("6.8.0-49-generic", false)]
fn wsl1_is_told_by_its_kernel_release(#[case] release: &str, #[case] wsl1: bool) {
    assert_eq!(is_wsl1(release), wsl1);
}

#[rstest]
#[case::landlock_6_no_userns_none(
    HostFacts { apparmor_restricts: true, ..host(UserNamespace::Unmapped(EPERM), 6) },
    NetworkLevel::None,
    &[]
)]
#[case::landlock_6_no_userns_direct(
    HostFacts { apparmor_restricts: true, ..host(UserNamespace::Unmapped(EPERM), 6) },
    NetworkLevel::Direct,
    &[]
)]
#[case::landlock_6_no_userns_proxy_tcp(
    HostFacts { apparmor_restricts: true, ..host(UserNamespace::Unmapped(EPERM), 6) },
    NetworkLevel::ProxyTcp,
    &["network_namespace", "user_namespace"]
)]
#[case::docker_before_6_12_with_landlock_none(
    HostFacts { seccomp_filtered: true, ..host(UserNamespace::Refused(EPERM), 4) },
    NetworkLevel::None,
    &["signal_scope", "user_namespace"]
)]
#[case::docker_before_6_12_no_landlock_none(
    HostFacts { seccomp_filtered: true, ..host(UserNamespace::Refused(EPERM), 0) },
    NetworkLevel::None,
    &["filesystem", "signal_scope", "user_namespace"]
)]
#[case::wsl1_none(
    HostFacts { wsl1: true, ..host(UserNamespace::Refused(EINVAL), 0) },
    NetworkLevel::None,
    &["seccomp", "filesystem", "signal_scope", "user_namespace"]
)]
#[case::wsl1_proxy_tcp(
    HostFacts { wsl1: true, ..host(UserNamespace::Refused(EINVAL), 0) },
    NetworkLevel::ProxyTcp,
    &["seccomp", "filesystem", "signal_scope", "user_namespace", "network_namespace"]
)]
fn missing_confinement_names_unprovided_parts_per_network_level(
    #[case] facts: HostFacts,
    #[case] level: NetworkLevel,
    #[case] expected: &[&str],
) {
    assert_eq!(facts.missing_confinement(level), expected);
}
