#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io;
use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;

use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::json;

use super::{Failure, Observed, display_of, refused};

/// What a start came to, without the text a failure keeps.
#[derive(Debug, PartialEq, Eq)]
enum Became {
    Display(u32),
    NotInstalled,
    FailedToStart,
    ExitedEarly,
    Unrecognized,
}

fn became(result: &Result<u32, Failure>) -> Became {
    match result {
        Ok(number) => Became::Display(*number),
        Err(Failure::NotInstalled) => Became::NotInstalled,
        Err(Failure::FailedToStart(_)) => Became::FailedToStart,
        Err(Failure::ExitedEarly(_)) => Became::ExitedEarly,
        Err(Failure::Unrecognized(_)) => Became::Unrecognized,
    }
}

/// The failure `observed` becomes.
fn failure(observed: Observed) -> Failure {
    display_of(observed).expect_err("the start failed")
}

/// Xvfb's translation point (ADR 0127): the display number it writes once
/// it listens, one case per failure it is known for, the OS's refusals to
/// start it included, and an unknown answer, a silence and an unreadable
/// output as `Unrecognized`.
#[rstest]
#[case::a_display(display_of(Observed::Wrote("99\n".to_owned())), Became::Display(99))]
#[case::not_installed(Err(refused(io::ErrorKind::NotFound.into())), Became::NotInstalled)]
#[case::failed_to_start(
    Err(refused(io::ErrorKind::PermissionDenied.into())),
    Became::FailedToStart
)]
#[case::exited_early(
    display_of(Observed::Exited(Ok(ExitStatus::from_raw(1 << 8)))),
    Became::ExitedEarly
)]
#[case::an_unknown_answer(
    display_of(Observed::Wrote("Fatal server error: no screens found\n".to_owned())),
    Became::Unrecognized
)]
#[case::an_unknown_end(
    display_of(Observed::Exited(Err(io::ErrorKind::Other.into()))),
    Became::Unrecognized
)]
#[case::an_unreadable_answer(
    display_of(Observed::Unreadable(io::ErrorKind::InvalidData.into())),
    Became::Unrecognized
)]
#[case::silence(display_of(Observed::Silent), Became::Unrecognized)]
fn a_start_of_xvfb_becomes_its_display_or_a_typed_failure(
    #[case] start: Result<u32, Failure>,
    #[case] expected: Became,
) {
    assert_eq!(became(&start), expected);
}

/// Every failure to start a virtual display, an unknown one too, fails
/// the Session's opening with `capability_unavailable` naming
/// `browser.headless`, set in the layer that asked for a window, whose
/// fix is a headless launch.
///
/// Covers: story:8-41, error:capability_unavailable
#[rstest]
#[case::not_installed(Failure::NotInstalled)]
#[case::failed_to_start(refused(io::ErrorKind::PermissionDenied.into()))]
#[case::exited_early(Failure::ExitedEarly(ExitStatus::from_raw(1 << 8)))]
#[case::unrecognized(failure(Observed::Silent))]
fn a_failed_virtual_display_names_browser_headless(#[case] failure: Failure) {
    let error = failure.call_error(SettingsLayer::Default);

    assert_eq!(
        (error.code(), error.details().cloned()),
        (
            "capability_unavailable",
            json!({
                "key": "browser.headless",
                "layer": "default",
                "kind": "virtual_display",
                "fix": "`browser.headless: true`",
            })
            .as_object()
            .cloned()
        )
    );
}
