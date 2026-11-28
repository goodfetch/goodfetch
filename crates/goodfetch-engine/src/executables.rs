//! How the engine reads a failed start of an executable Goodfetch did not
//! build, such as a browser, a third-party process Plugin or an interpreter
//! from the runtime cache: Windows' refusal under Smart App Control, or
//! another App Control policy, becomes `executable_blocked_by_smart_app_control`
//! naming the file (ADR 0039). The code that starts the executable keeps
//! every other failure as its own.

use std::io;
use std::path::Path;

use goodfetch_protocol::executables::{
    EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL, ExecutableBlockedBySmartAppControlDetails,
    is_smart_app_control_os_error,
};

use crate::call_error::CallError;

/// The `executable_blocked_by_smart_app_control` refusal naming `file`, when
/// Windows' Smart App Control or another App Control policy refused to start
/// it (ADR 0039).
#[must_use]
pub fn smart_app_control_refusal(file: &Path) -> CallError {
    let file = file.to_string_lossy().into_owned();
    CallError::new(
        &EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL,
        format!(
            "Windows refused to start {file}: Smart App Control or an App Control policy blocks it"
        ),
        &ExecutableBlockedBySmartAppControlDetails { file },
    )
}

/// The failure a start of `file` answers with when `error`, from that start,
/// is Windows refusing it under Smart App Control or another App Control
/// policy; `None` for any other failure.
pub fn blocked_by_smart_app_control(file: &Path, error: &io::Error) -> Option<CallError> {
    error
        .raw_os_error()
        .filter(|&code| is_smart_app_control_os_error(code))
        .map(|_| smart_app_control_refusal(file))
}

#[cfg(test)]
mod tests;
