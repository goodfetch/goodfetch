//! Start at login (ADR 0141), on macOS: a launchd user agent in the
//! person's `~/Library/LaunchAgents` that runs `/usr/bin/open` on the
//! installed `.app`, so the Kennel starts through `LaunchServices`, which
//! desktop notices need (#1915). The setting `start_at_login` writes it,
//! and turning it off removes it.
//!
//! The agent is written and never loaded: launchd loads it at the next
//! login, and loading it now would open a second Kennel. Once removed, it
//! is unloaded too, for the login it started the Kennel at. No shell runs:
//! the file is written by rename, and `launchctl` runs by its absolute
//! path, behind [`Launchctl`], so tests replace it.
//!
//! Every call here blocks, so the Kennel makes them on the tracker's
//! blocking threads.
#![expect(
    clippy::disallowed_methods,
    reason = "the agent's file and `launchctl` are reached on the tracker's blocking threads, through `Tasks::blocking` (STYLE.md)"
)]

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::home::{read_if_present, replace};

/// The agent's launchd label, after the `.app`'s bundle identifier.
pub const LABEL: &str = "dev.goodfetch.kennel.login";

/// What starts the Kennel through `LaunchServices`.
const OPEN: &str = "/usr/bin/open";

/// `launchctl`, by its absolute path.
const LAUNCHCTL: &str = "/bin/launchctl";

/// The person's agents' folder, under their home folder.
const AGENTS_FOLDER: &str = "Library/LaunchAgents";

/// What an [`LoginAgent::apply`] changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The agent was written, or rewritten for another place of the app.
    Installed,
    /// The agent was removed.
    Removed,
    /// The agent was already as the setting asks.
    Unchanged,
}

/// Unloads a launchd job of the person's session.
pub trait Launchctl {
    /// Unloads the job `label`. Blocks.
    ///
    /// # Errors
    /// When `launchctl` cannot run or refuses, as when the job is not
    /// loaded.
    fn unload(&self, label: &str) -> io::Result<()>;
}

/// The system's `launchctl`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLaunchctl;

impl Launchctl for SystemLaunchctl {
    fn unload(&self, label: &str) -> io::Result<()> {
        // `remove` acts in the caller's own session, so no user ID is
        // needed, unlike `bootout gui/<uid>/<label>`.
        let status = Command::new(LAUNCHCTL)
            .args(["remove", label])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "launchctl remove ended with {status}"
            )))
        }
    }
}

/// The login agent's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginAgent {
    file: PathBuf,
}

impl LoginAgent {
    /// The agent in the agents' folder `folder`.
    pub fn in_folder(folder: &Path) -> Self {
        Self {
            file: folder.join(format!("{LABEL}.plist")),
        }
    }

    /// The agent in the person's own agents' folder; none when the OS
    /// names no home folder.
    pub fn of_the_person() -> Option<Self> {
        dirs::home_dir().map(|home| Self::in_folder(&home.join(AGENTS_FOLDER)))
    }

    /// Makes the agent as the setting asks: with `bundle`, an agent that
    /// opens that `.app`; with none, no agent. Blocks.
    ///
    /// # Errors
    /// When the file cannot be read, written or removed, or `bundle`'s
    /// path is not UTF-8. A refusal of `launchctl` to unload a job it does
    /// not hold is no error.
    pub fn apply(&self, bundle: Option<&Path>, launchctl: &impl Launchctl) -> io::Result<Change> {
        let present = read_if_present(&self.file)?;
        if let Some(bundle) = bundle {
            let wanted = agent(bundle)?;
            if present.as_deref() == Some(wanted.as_bytes()) {
                return Ok(Change::Unchanged);
            }
            replace(&self.file, wanted.as_bytes())?;
            return Ok(Change::Installed);
        }
        if present.is_none() {
            return Ok(Change::Unchanged);
        }
        fs::remove_file(&self.file)?;
        if let Err(error) = launchctl.unload(LABEL) {
            // Not loaded: it was written at this login.
            tracing::debug!(%error, "the login agent was not unloaded");
        }
        Ok(Change::Removed)
    }
}

/// The `.app` around `executable`, `<name>.app/Contents/MacOS/<file>`: the
/// installed app the Kennel runs from, or none when it runs bare.
pub fn bundle_of(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_owned())
}

/// The agent's property list, opening `bundle`.
fn agent(bundle: &Path) -> io::Result<String> {
    let bundle = bundle.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the app's path is not UTF-8, which a property list needs",
        )
    })?;
    let mut text = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n",
        "<dict>\n",
    ));
    let _ = write!(
        text,
        "\t<key>Label</key>\n\t<string>{}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t\t<string>{}</string>\n\t</array>\n",
        escaped(LABEL),
        escaped(OPEN),
        escaped(bundle),
    );
    text.push_str("\t<key>RunAtLoad</key>\n\t<true/>\n</dict>\n</plist>\n");
    Ok(text)
}

/// `text` as XML character data.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
