#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::ExitStatus;

use goodfetch_confinement::launch::LaunchError;
use goodfetch_protocol::plugin::PluginResourceLimit;
use rstest::rstest;

use super::Ended;

/// The wait status of a process that exited with `code`.
fn exited(code: i32) -> ExitStatus {
    ExitStatus::from_raw(code << 8)
}

/// Covers: story:7-105, error:plugin_limit_exceeded, error:executable_blocked_by_smart_app_control
#[rstest]
#[case::the_host_cannot_confine_it(
    exited(i32::from(LaunchError::Unavailable(String::new()).status())),
    Ended::ConfinementUnavailable
)]
#[case::the_os_has_no_confinement(
    exited(i32::from(LaunchError::Unsupported.status())),
    Ended::ConfinementUnavailable
)]
#[case::smart_app_control_blocked_it(
    exited(i32::from(LaunchError::BlockedBySmartAppControl(PathBuf::from("plugin.exe")).status())),
    Ended::BlockedBySmartAppControl
)]
#[case::job_memory_limit_exceeded(
    exited(i32::from(LaunchError::LimitExceeded(PluginResourceLimit::Memory).status())),
    Ended::LimitExceeded(PluginResourceLimit::Memory)
)]
#[case::job_cpu_limit_exceeded(
    exited(i32::from(LaunchError::LimitExceeded(PluginResourceLimit::CpuTime).status())),
    Ended::LimitExceeded(PluginResourceLimit::CpuTime)
)]
#[case::job_processes_limit_exceeded(
    exited(i32::from(LaunchError::LimitExceeded(PluginResourceLimit::Processes).status())),
    Ended::LimitExceeded(PluginResourceLimit::Processes)
)]
#[case::job_open_files_limit_exceeded(
    exited(i32::from(LaunchError::LimitExceeded(PluginResourceLimit::OpenFiles).status())),
    Ended::LimitExceeded(PluginResourceLimit::OpenFiles)
)]
#[case::its_launcher_failed_otherwise(
    exited(i32::from(LaunchError::Request.status())),
    Ended::Exited(Some(246))
)]
#[case::it_exited(exited(0), Ended::Exited(Some(0)))]
#[case::a_signal_killed_it(ExitStatus::from_raw(9), Ended::Exited(None))]
fn an_instance_that_ended_says_why(#[case] status: ExitStatus, #[case] expected: Ended) {
    assert_eq!(Ended::of(status), expected);
}
