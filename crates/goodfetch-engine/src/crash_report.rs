//! Crash reports (ADR 0043): a panic in any thread of the process writes a
//! report into a folder on the machine, its message, where it happened, the
//! thread and a backtrace, and logs where the report is. Nothing reads the
//! folder but a person: no report is ever sent.

use std::borrow::Cow;
use std::cell::Cell;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::clock::Clock;
use crate::mask::{Masks, Scope};

/// Writes a crash report into `folder` for each panic of this process,
/// headed by `executable`, its name and version, and timed by `clock`, in
/// place of the panic's line on stderr; every value a live Run resolved is
/// masked in it by `masks`, the engine's, as what the engine keeps.
pub fn record_panics(folder: PathBuf, executable: String, clock: Clock, masks: Masks) {
    std::panic::set_hook(Box::new(move |panic| {
        let report = report(&executable, clock, panic, &masks);
        match write(&folder, &report) {
            Ok(path) => {
                LAST_WRITTEN.set(Some(report.name));
                tracing::error!(
                    report = %path.display(),
                    "the process panicked; its crash report stays on this machine"
                );
            }
            Err(error) => {
                LAST_WRITTEN.set(None);
                tracing::error!(
                    %error,
                    folder = %folder.display(),
                    "the process panicked, and its crash report cannot be written"
                );
            }
        }
    }));
}

thread_local! {
    /// The file name of the crash report of this thread's latest panic,
    /// until whoever caught that panic takes it; `None` when its report
    /// could not be written.
    static LAST_WRITTEN: Cell<Option<String>> = const { Cell::new(None) };
}

/// The file name of the crash report written for the panic the caller just
/// caught, on the thread that panicked: `None` when the process records
/// no panics ([`record_panics`]), or when the report could not be written.
/// The hook runs on the panicking thread before it unwinds, so a caller
/// that takes it where it caught the unwind reads its own panic's.
pub(crate) fn take_written() -> Option<String> {
    LAST_WRITTEN.take()
}

/// The report of `panic`.
fn report(
    executable: &str,
    clock: Clock,
    panic: &std::panic::PanicHookInfo<'_>,
    masks: &Masks,
) -> Report {
    let at = clock.timestamp();
    let message = panic
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned());
    let mut text = String::new();
    let _ = writeln!(text, "Crash report of {executable}");
    let _ = writeln!(text, "time: {at}");
    let _ = writeln!(
        text,
        "thread: {}",
        std::thread::current().name().unwrap_or("unnamed")
    );
    if let Some(location) = panic.location() {
        let _ = writeln!(text, "location: {location}");
    }
    let _ = writeln!(text, "message: {message}");
    let _ = writeln!(
        text,
        "backtrace:\n{}",
        std::backtrace::Backtrace::force_capture()
    );
    Report {
        name: format!(
            "crash-{}-{}.txt",
            at.strftime("%Y%m%dT%H%M%S%.fZ"),
            std::process::id()
        ),
        // A panic may break off a change of the masks, whose lock it then
        // holds: the report never waits for it, and withholds its text.
        text: masks.try_apply(&text, Scope::Kept).map_or_else(
            || format!("Crash report of {executable}\n(withheld: the panic broke off a change of the engine's masks)\n"),
            Cow::into_owned,
        ),
    }
}

/// A crash report: its file's name and its text.
struct Report {
    name: String,
    text: String,
}

#[expect(
    clippy::disallowed_methods,
    reason = "a panic hook writes its report where it runs, on the panicking thread, outside any await"
)]
fn write(folder: &Path, report: &Report) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let path = folder.join(&report.name);
    std::fs::write(&path, &report.text)?;
    Ok(path)
}

#[cfg(test)]
mod tests;
