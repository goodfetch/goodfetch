//! The real OS services the tests meet (ADR 0059; TESTING.md, Storage,
//! network and OS services): each extra loopback address binds on the host,
//! the Secret Service stores and reads an item, a host without one fails the
//! check, and a local `just test` lists it and skips the tests that need it.
//! The tests needing a service sit in a module named after it, `keychain`,
//! `extra_loopback` or `tc_netem`, which is what a local run skips.

use std::ffi::OsStr;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn jobs_that_set_up_the_services_before_the_deny_pass() {
    let workspace = Workspace::from_fixture("os-services");

    let outcome = workspace.xtask(&["os-services", "workflows"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::without_the_setup("without-setup.yml", "missing-os-services-setup")]
#[case::setup_after_a_test_step("setup-after-test.yml", "os-services-setup-after-test")]
#[case::setup_after_the_deny("setup-after-deny.yml", "os-services-setup-after-deny")]
fn a_test_job_without_the_services_before_its_tests_fails(
    #[case] workflow: &str,
    #[case] kind: &str,
) {
    let workspace = Workspace::from_fixture("os-services");
    workspace.add_fixture_file(
        &format!("os-services-cases/{workflow}"),
        ".github/workflows/pull-request.yml",
    );

    let outcome = workspace.xtask(&["os-services", "workflows"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, "pull-request.yml#quick")]
    );
    assert_eq!(outcome.code, 1);
}

mod extra_loopback {
    use crate::workspace::Workspace;

    #[test]
    fn the_first_extra_loopback_address_binds() {
        let workspace = Workspace::from_fixture("empty");

        let outcome = workspace.xtask(&["os-services", "check", "--loopback", "127.0.0.2"]);

        assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    }
}

/// Runs on Linux, where the Secret Service is the keychain:
/// `ci/os-services.sh setup` starts it and points these variables at it.
#[cfg(target_os = "linux")]
mod keychain {
    use std::ffi::OsString;

    use crate::workspace::Workspace;

    #[test]
    fn the_secret_service_stores_and_reads_an_item() {
        let workspace = Workspace::from_fixture("empty");
        let session: Vec<(&str, OsString)> = ["DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (name, value)))
            .collect();
        let variables: Vec<_> = session
            .iter()
            .map(|(name, value)| (*name, value.as_os_str()))
            .collect();

        let outcome =
            workspace.xtask_with_env(&["os-services", "check", "--secret-service"], &variables);

        assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    }
}

#[test]
fn an_unreachable_secret_service_fails_the_check() {
    let workspace = Workspace::from_fixture("empty");
    let bus = format!("unix:path={}", workspace.path().join("no-bus").display());

    let outcome = workspace.xtask_with_env(
        &["os-services", "check", "--secret-service"],
        &[("DBUS_SESSION_BUS_ADDRESS", OsStr::new(&bus))],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding("secret-service-missing", "Secret Service")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_local_run_lists_each_missing_service_and_the_tests_it_skips() {
    let workspace = Workspace::from_fixture("empty");
    let bus = format!("unix:path={}", workspace.path().join("no-bus").display());

    let outcome = workspace.xtask_with_env(
        &[
            "os-services",
            "check",
            "--loopback",
            "192.0.2.1",
            "--secret-service",
            "--skipped",
            "skipped.txt",
        ],
        &[("DBUS_SESSION_BUS_ADDRESS", OsStr::new(&bus))],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "extra_loopback::\nkeychain::\n");
    assert_eq!(workspace.read_text("skipped.txt").lines().count(), 2);
}

#[test]
fn a_host_without_tc_lacks_netem_and_a_local_run_skips_its_tests() {
    let workspace = Workspace::from_fixture("empty");
    let nothing = workspace.path().join("no-tools");
    std::fs::create_dir(&nothing).unwrap();

    let outcome = workspace.xtask_with_env(
        &[
            "os-services",
            "check",
            "--netem",
            "--skipped",
            "skipped.txt",
        ],
        &[("PATH", nothing.as_os_str())],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "tc_netem::\n");
    assert_eq!(workspace.read_text("skipped.txt").lines().count(), 1);
}

#[test]
fn a_local_run_on_a_host_with_every_service_skips_nothing() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = workspace.xtask(&[
        "os-services",
        "check",
        "--loopback",
        "127.0.0.1",
        "--skipped",
        "skipped.txt",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "");
    assert_eq!(workspace.read("skipped.txt"), None);
}

/// Outside Linux, `daemon install`'s tests install for real in the Release
/// suite only, so a local run of another suite skips them.
#[cfg(not(target_os = "linux"))]
#[test]
fn outside_the_release_suite_a_local_run_skips_the_service_manager_tests() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = workspace.xtask(&[
        "os-services",
        "check",
        "--service-manager",
        "--suite",
        "pre-merge",
        "--skipped",
        "skipped.txt",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "service_manager::\n");
    assert_eq!(workspace.read_text("skipped.txt").lines().count(), 1);
}

/// On Linux, a host whose `PATH` has no `systemctl` has no user manager.
#[cfg(target_os = "linux")]
#[test]
fn a_host_without_systemctl_lacks_the_service_manager() {
    let workspace = Workspace::from_fixture("empty");
    let nothing = workspace.path().join("no-tools");
    std::fs::create_dir(&nothing).unwrap();

    let outcome = workspace.xtask_with_env(
        &["os-services", "check", "--service-manager"],
        &[("PATH", nothing.as_os_str())],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "service-manager-missing",
            "the user's service manager"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_address_not_on_the_host_fails_the_check() {
    let workspace = Workspace::from_fixture("empty");

    // 192.0.2.1 is TEST-NET-1 (RFC 5737), assigned to no host.
    let outcome = workspace.xtask(&["os-services", "check", "--loopback", "192.0.2.1"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("loopback-address-missing", "192.0.2.1")]
    );
    assert_eq!(outcome.code, 1);
}

/// `ci/os-services.sh skipped`, which `just test` runs through
/// `ci/nextest-skips.sh`, on a host missing the Secret Service: a stand-in
/// `cargo` answers for `xtask os-services check` as it would there. In CI,
/// and where the caller requires every service (`GOODFETCH_OS_SERVICES=required`,
/// the local gate on Linux), the missing service fails; elsewhere its tests
/// are skipped.
#[cfg(unix)]
#[rstest]
#[case::local(&[], Some(0), "keychain::\n")]
#[case::in_ci(&[("CI", "true")], Some(1), "")]
#[case::required(&[("GOODFETCH_OS_SERVICES", "required")], Some(1), "")]
fn a_missing_service_fails_only_where_every_service_is_required(
    #[case] variables: &[(&str, &str)],
    #[case] code: Option<i32>,
    #[case] stdout: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = workspace.path().join("stand-in");
    std::fs::create_dir_all(&stand_in).unwrap();
    let cargo = stand_in.join("cargo");
    std::fs::write(
        &cargo,
        r#"#!/bin/sh
case " $* " in
  *" --skipped "*) echo "keychain::" ;;
  *) echo "secret-service-missing: Secret Service" >&2; exit 1 ;;
esac
"#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/os-services.sh");

    let output = workspace
        .command(script)
        .args(["skipped", "skipped.txt"])
        .env("PATH", format!("{}:/usr/bin:/bin", stand_in.display()))
        .envs(variables.iter().copied())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        code,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), stdout);
}
