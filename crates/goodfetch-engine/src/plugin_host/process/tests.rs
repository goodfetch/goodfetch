use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;

use goodfetch_confinement::launch::LaunchError;
use rstest::rstest;

use super::Ended;

/// The wait status of a process that exited with `code`.
fn exited(code: i32) -> ExitStatus {
    ExitStatus::from_raw(code << 8)
}

#[rstest]
#[case::the_host_cannot_confine_it(
    exited(i32::from(LaunchError::Unavailable(String::new()).status())),
    Ended::ConfinementUnavailable
)]
#[case::the_os_has_no_confinement(
    exited(i32::from(LaunchError::Unsupported.status())),
    Ended::ConfinementUnavailable
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
