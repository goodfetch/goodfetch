//! The details window's process (ADR 0141): a second process of the Kennel's
//! own executable, run with [`ARGUMENT`], which reads the Daemon's runtime
//! file itself and connects as its own `kennel` Connection. The tray passes
//! it nothing but that argument and why it opens it ([`Opening`]), so the
//! Daemon's token never leaves the process that read it. The process exits when its window closes, so a
//! closed window costs nothing.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Stdio;

use crate::text;

/// The argument that makes the Kennel's executable the details window.
pub const ARGUMENT: &str = "window";

/// The argument after [`ARGUMENT`] that opens the window for a hidden icon.
const HIDDEN_ICON_ARGUMENT: &str = "hidden-icon";

/// The most characters of a note.
const NOTE_MOST: usize = 300;

/// Why the details window opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    /// The person chose "Open details".
    Asked,
    /// The hidden-icon check found the icon hidden, so the window opens
    /// with a note asking the person to free space in the menu bar.
    HiddenIcon,
}

impl Opening {
    /// Why the window opens, from the executable's `arguments` after its
    /// own name; none when they do not make it the details window, which
    /// makes it the tray.
    pub fn from_arguments(arguments: impl IntoIterator<Item = OsString>) -> Option<Self> {
        let mut arguments = arguments.into_iter();
        if arguments.next()? != ARGUMENT {
            return None;
        }
        Some(match arguments.next() {
            Some(argument) if argument == HIDDEN_ICON_ARGUMENT => Self::HiddenIcon,
            _ => Self::Asked,
        })
    }

    /// The note the window shows above its views when it opens this way.
    pub fn note(self) -> Option<String> {
        match self {
            Self::Asked => None,
            Self::HiddenIcon => Some(text::shown(
                "The Kennel's icon is hidden, behind the notch or in the menu bar's overflow. Free space in the menu bar, by quitting or removing other menu bar items, and the icon shows again.",
                NOTE_MOST,
            )),
        }
    }

    /// The arguments that open the window this way.
    fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::Asked => &[ARGUMENT],
            Self::HiddenIcon => &[ARGUMENT, HIDDEN_ICON_ARGUMENT],
        }
    }
}

/// Runs the details window's process from `executable`, opened as
/// `opening` says, and waits for it to end, after giving `opened` its
/// process ID. Its standard error is the
/// tray's, where its logs go; it reads and writes nothing else.
///
/// # Cancel safety
/// Dropping the future stops the wait, never the window.
pub async fn run(executable: &Path, opening: Opening, opened: impl FnOnce(u32)) -> io::Result<()> {
    let mut command = tokio::process::Command::new(executable);
    command
        .args(opening.arguments())
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
