//! The reading of a failed start through its interface: the OS error a
//! start returned, as `std::process::Command` gives it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io;
use std::path::Path;

use rstest::rstest;

use super::blocked_by_smart_app_control;

/// Windows' code-integrity refusals, by their `WinError.h` names.
///
/// Covers: error:executable_blocked_by_smart_app_control
#[rstest]
#[case::policy_violation(4551)]
#[case::reputation_malicious(4556)]
#[case::reputation_potentially_unwanted(4557)]
#[case::reputation_dangerous_extension(4558)]
#[case::reputation_offline(4559)]
#[case::reputation_unfriendly_file(4580)]
#[case::reputation_unattainable(4581)]
#[case::reputation_explicit_deny(4582)]
fn a_start_windows_refuses_fails_with_smart_app_controls_code(#[case] os_error: i32) {
    let file = Path::new(r"C:\Users\ana\AppData\Local\goodfetch\browsers\chrome.exe");

    let refusal =
        blocked_by_smart_app_control(file, &io::Error::from_raw_os_error(os_error)).unwrap();

    assert_eq!(refusal.code(), "executable_blocked_by_smart_app_control");
}

/// The file it names, and that a retry would meet the same refusal.
#[test]
fn smart_app_controls_refusal_names_the_file_and_is_not_retryable() {
    let file = Path::new(r"C:\Program Files\Plugins\ocr.exe");

    let refusal = blocked_by_smart_app_control(file, &io::Error::from_raw_os_error(4551)).unwrap();

    insta::assert_debug_snapshot!(refusal);
}

/// A refusal of another kind keeps its own failure.
#[rstest]
#[case::access_denied(io::Error::from_raw_os_error(5))]
#[case::file_not_found(io::Error::from(io::ErrorKind::NotFound))]
#[case::permission_denied(io::Error::from(io::ErrorKind::PermissionDenied))]
#[case::code_integrity_policy_not_signed(io::Error::from_raw_os_error(4553))]
fn another_start_failure_is_no_smart_app_control_refusal(#[case] error: io::Error) {
    let file = Path::new("/usr/bin/chromium");

    let refusal = blocked_by_smart_app_control(file, &error);

    assert!(refusal.is_none(), "{refusal:?}");
}
