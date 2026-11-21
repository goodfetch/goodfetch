use rstest::rstest;
use serde_json::json;

use super::refusal;

/// The full build headless, from a process with no graphic access, is
/// refused with the setting that launches the headless shell as its `fix`.
///
/// Covers: error:browser_needs_gui_session
#[test]
fn the_full_build_headless_with_no_graphic_access_needs_a_gui_session() {
    let refused = refusal(false, true, Some(false)).expect("the launch is refused");

    assert_eq!(
        (refused.code(), refused.details().cloned()),
        (
            "browser_needs_gui_session",
            json!({ "fix": "set `browser.headless_shell: true`" })
                .as_object()
                .cloned()
        )
    );
}

/// The shell, a launch with a window, and a process whose access is known
/// or unknown to have it are never refused.
#[rstest]
#[case::the_headless_shell(true, true, Some(false))]
#[case::a_window(false, false, Some(false))]
#[case::graphic_access(false, true, Some(true))]
#[case::unknown_access(false, true, None)]
fn any_other_launch_goes_ahead(
    #[case] shell: bool,
    #[case] headless: bool,
    #[case] graphic_access: Option<bool>,
) {
    assert!(refusal(shell, headless, graphic_access).is_none());
}
