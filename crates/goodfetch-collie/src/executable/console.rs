//! `console` (ADRs 0024, 0031 and 0098): starts the major's Daemon unless
//! one runs, then opens the person's browser on its Console, signed in
//! through the sign-in link, whose URL fragment carries the Daemon's token
//! for the page to swap once for its cookie. The browser opens on a page
//! of the user's own (mode `0600`, as the runtime file that holds the same
//! token) that goes on to the link, so the token is never in a command line
//! another user's process list shows. `--no-browser` opens nothing and
//! prints the link, which tests always pass; `--json` prints it as
//! `ConsoleOpened` (ADR 0069). The token is never logged.
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI writes the opening page and starts the OS's opener outside any async worker (ADR 0060)"
)]

use std::io;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use goodfetch_cli::output::{self, Human};
use goodfetch_protocol::console::ConsoleOpened;

use super::daemon::Machine;

/// `console`.
pub(super) fn open(machine: &Machine<'_>, no_browser: bool, json: bool, human: &Human) -> ExitCode {
    let link = match machine.console_link() {
        Ok(link) => link,
        Err(failure) => return failure.print(json, human),
    };
    let console = link.split_once('#').map_or(link.as_str(), |(url, _)| url);
    let opened = if no_browser {
        false
    } else {
        match machine
            .files()
            .write_console_opener(&link)
            .and_then(|page| open_in_browser(&page))
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "the browser did not open on the Console at {console}");
                false
            }
        }
    };
    if json {
        output::json(&ConsoleOpened { url: link });
    } else if opened {
        output::line(&format!(
            "The Console of major {} opens in your browser, signed in: {console}",
            machine.major()
        ));
    } else {
        output::line(&format!(
            "Open the Console of major {} in your browser, signed in: {link}",
            machine.major()
        ));
    }
    ExitCode::SUCCESS
}

/// Opens `page` in the person's browser through the OS's opener.
fn open_in_browser(page: &Path) -> io::Result<()> {
    let mut command = opener();
    let status = command
        .arg(page)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "the OS's opener ended with {status}"
        )))
    }
}

/// The OS's opener of a file in its default application.
#[cfg(target_os = "macos")]
fn opener() -> Command {
    Command::new("open")
}

/// The OS's opener of a file in its default application.
#[cfg(windows)]
fn opener() -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "start", ""]);
    command
}

/// The OS's opener of a file in its default application.
#[cfg(not(any(target_os = "macos", windows)))]
fn opener() -> Command {
    Command::new("xdg-open")
}
