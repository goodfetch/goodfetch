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
};

use crate::call_error::CallError;

/// The `WinError.h` codes with which Windows' code integrity refuses to
/// start a file: `ERROR_SYSTEM_INTEGRITY_POLICY_VIOLATION` (4551) and the
/// reputation refusals Smart App Control gives (4556 to 4559, 4580 to 4582).
/// The codes that report a broken or unsigned policy (4550, 4552 to 4555)
/// refuse no file. No other OS's `errno` reaches these values, so the
/// reading needs no `cfg`, and its tests run on every OS.
const CODE_INTEGRITY_REFUSALS: [i32; 8] = [4551, 4556, 4557, 4558, 4559, 4580, 4581, 4582];

/// The failure a start of `file` answers with when `error`, from that start,
/// is Windows refusing it under Smart App Control or another App Control
/// policy; `None` for any other failure.
pub fn blocked_by_smart_app_control(file: &Path, error: &io::Error) -> Option<CallError> {
    let refused = error
        .raw_os_error()
        .is_some_and(|code| CODE_INTEGRITY_REFUSALS.contains(&code));
    refused.then(|| {
        let file = file.to_string_lossy().into_owned();
        CallError::new(
            &EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL,
            format!("Windows refused to start {file}: Smart App Control or an App Control policy blocks it"),
            &ExecutableBlockedBySmartAppControlDetails { file },
        )
    })
}

#[cfg(test)]
mod tests;
