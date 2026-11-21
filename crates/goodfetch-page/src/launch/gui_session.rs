//! Whether the full Chrome for Testing build can start headless where the
//! engine runs (ADR 0115's amendment of 2026-11-21; #3224): on macOS, from
//! a process whose security session has no graphic access, such as a
//! LaunchDaemon, an SSH login or a CI runner in the Background session, the
//! full build's headless mode crashes at start-up, and the launch fails at
//! once with `browser_needs_gui_session` instead, nothing launched. The
//! session's attributes come from Security's `SessionGetInfo`, one call and
//! no subprocess; where it cannot answer, the launch goes ahead, since an
//! unknown never refuses. The engine never switches to the headless shell
//! by itself: the `fix` names the setting that does.

#[cfg(test)]
mod tests;

use goodfetch_engine::CallError;
use goodfetch_protocol::page::{
    BROWSER_HEADLESS_SHELL, BROWSER_NEEDS_GUI_SESSION, BrowserNeedsGuiSessionDetails,
};

/// `browser_needs_gui_session` for a launch of the full build (`shell`
/// false) headless (`headless` true) from a process with no graphic access;
/// `None` for any other launch, and where the access is unknown.
pub(super) fn refusal(
    shell: bool,
    headless: bool,
    graphic_access: Option<bool>,
) -> Option<CallError> {
    if shell || !headless || graphic_access != Some(false) {
        return None;
    }
    let fix = format!("set `{}: true`", BROWSER_HEADLESS_SHELL.name);
    Some(CallError::new(
        &BROWSER_NEEDS_GUI_SESSION,
        format!(
            "This process has no GUI session, where the full Chrome for Testing build cannot start headless: {fix} to launch the headless shell, or run the engine in a login session."
        ),
        &BrowserNeedsGuiSessionDetails { fix },
    ))
}

/// Whether this process's security session has graphic access: `None` off
/// macOS, where nothing is asked, and where Security gives no answer.
pub(super) fn graphic_access() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        macos_graphic_access()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "a process's security session is read only through Security's C API, `SessionGetInfo`; the macOS tests cover it, which Miri cannot run (STYLE.md, `unsafe`)"
)]
fn macos_graphic_access() -> Option<bool> {
    /// `callerSecuritySession`: the session of the calling process.
    const CALLER_SECURITY_SESSION: u32 = 0xFFFF_FFFF;
    /// `sessionHasGraphicAccess`: the session can reach the window server.
    const SESSION_HAS_GRAPHIC_ACCESS: u32 = 0x0010;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        /// `<Security/AuthSession.h>`: the ID and attributes of `session`,
        /// an `OSStatus` of 0 when it answers.
        fn SessionGetInfo(session: u32, session_id: *mut u32, attributes: *mut u32) -> i32;
    }

    let mut session_id: u32 = 0;
    let mut attributes: u32 = 0;
    // SAFETY: `session_id` and `attributes` are valid places for the call's
    // two answers, which it only writes, and outlive it.
    let status = unsafe {
        SessionGetInfo(
            CALLER_SECURITY_SESSION,
            &raw mut session_id,
            &raw mut attributes,
        )
    };
    (status == 0).then_some(attributes & SESSION_HAS_GRAPHIC_ACCESS != 0)
}
