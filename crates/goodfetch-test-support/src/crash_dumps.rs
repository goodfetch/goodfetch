//! Crash dumps (ADR 0074): the folder in a test's Goodfetch home where a
//! browser it launches writes its crash dumps, and the files a crash left
//! there, which fail the test at teardown.
#![expect(
    clippy::disallowed_methods,
    reason = "test support lists a folder at teardown, outside any async worker (ADR 0062)"
)]

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The variable of the environment map naming the crash-dump folder:
/// Chromium's own, which its crash reporter reads on every desktop OS in
/// place of its default folder.
pub const CRASH_DUMP_VARIABLE: &str = "BREAKPAD_DUMP_LOCATION";

/// The crash-dump folder's name inside the Goodfetch home.
pub(crate) const FOLDER: &str = "crash-dumps";

/// The panic payload of a test whose crash-dump folder held files at
/// teardown: a launched browser crashed.
#[derive(Debug, PartialEq, Eq)]
pub struct CrashDumps {
    /// Every file in the folder and its subfolders, sorted.
    pub dumps: Vec<PathBuf>,
}

impl fmt::Display for CrashDumps {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "crash dumps in the test's crash-dump folder, so a browser it launched crashed (ADR 0074):"
        )?;
        for dump in &self.dumps {
            write!(formatter, " {}", dump.display())?;
        }
        Ok(())
    }
}

/// The file Chromium's crash reporter, Crashpad, keeps at the root of its
/// database whenever it runs, crash or no crash: its settings, no dump.
const CRASHPAD_SETTINGS: &str = "settings.dat";

/// Every file in `folder` and its subfolders, sorted, but Crashpad's own
/// settings; none when the folder is gone.
pub(crate) fn find(folder: &Path) -> io::Result<Vec<PathBuf>> {
    let settings = folder.join(CRASHPAD_SETTINGS);
    let mut dumps = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                folders.push(entry.path());
            } else if entry.path() != settings {
                dumps.push(entry.path());
            }
        }
    }
    dumps.sort();
    Ok(dumps)
}
