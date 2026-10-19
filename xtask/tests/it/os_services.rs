//! The real OS services the tests meet (ADR 0059; TESTING.md, Storage,
//! network and OS services): each extra loopback address binds on the host,
//! the Secret Service stores and reads an item, a host without one fails the
//! check, and a local `just test` lists it and skips the tests that need it.
//! The tests needing a service sit in a module named after it, `keychain` or
//! `extra_loopback`, which is what a local run skips.

use std::ffi::OsStr;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn test_jobs_that_set_up_the_services_before_the_deny_pass() {
    let workspace = Workspace::from_fixture("os-services");

    let outcome = workspace.xtask(&["os-services", "workflows"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::without_the_setup("without-setup.yml", "missing-os-services-setup")]
#[case::setup_after_a_test_step("setup-after-test.yml", "os-services-setup-after-test")]
#[case::setup_after_the_deny("setup-after-deny.yml", "os-services-setup-after-deny")]
fn test_job_without_the_services_before_its_tests_fails(
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
