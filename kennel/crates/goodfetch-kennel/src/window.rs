//! The details window's process (ADR 0141): a second process of the Kennel's
//! own executable, run with [`ARGUMENT`], which reads the Daemon's runtime
//! file itself and connects as its own `kennel` Connection. The tray passes
//! it nothing but that argument, so the Daemon's token never leaves the
//! process that read it. The process exits when its window closes, so a
//! closed window costs nothing.

use std::io;
use std::path::Path;
use std::process::Stdio;

/// The argument that makes the Kennel's executable the details window.
pub const ARGUMENT: &str = "window";

/// Runs the details window's process from `executable` and waits for it
/// to end, after giving `opened` its process ID. Its standard error is the
/// tray's, where its logs go; it reads and writes nothing else.
///
/// # Cancel safety
/// Dropping the future stops the wait, never the window.
pub async fn run(executable: &Path, opened: impl FnOnce(u32)) -> io::Result<()> {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    #[expect(
        clippy::disallowed_methods,
        reason = "the tray starts the details window as a second process of its own executable (ADR 0141)"
    )]
    let mut child = command.spawn()?;
    if let Some(pid) = child.id() {
        opened(pid);
    }
    let status = child.wait().await?;
    if !status.success() {
        tracing::warn!(%status, "the details window's process ended with a failure");
    }
    Ok(())
}
